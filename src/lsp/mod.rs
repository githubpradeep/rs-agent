//! Minimal LSP client (stdio JSON-RPC) for diagnostics.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub path: String,
    pub line: u32,
    pub character: u32,
    pub severity: u8, // 1=error 2=warn 3=info 4=hint
    pub message: String,
    pub source: String,
}

#[derive(Debug, Default, Clone)]
pub struct DiagnosticSnapshot {
    pub by_file: HashMap<String, Vec<Diagnostic>>,
}

impl DiagnosticSnapshot {
    pub fn error_count(&self) -> usize {
        self.by_file
            .values()
            .flatten()
            .filter(|d| d.severity == 1)
            .count()
    }

    pub fn warn_count(&self) -> usize {
        self.by_file
            .values()
            .flatten()
            .filter(|d| d.severity == 2)
            .count()
    }

    pub fn summary_line(&self) -> String {
        let e = self.error_count();
        let w = self.warn_count();
        if e == 0 && w == 0 {
            if self.by_file.is_empty() {
                String::new()
            } else {
                " LSP✓".into()
            }
        } else {
            format!(" LSP E:{e} W:{w}")
        }
    }

    pub fn format_report(&self, limit: usize) -> String {
        let mut lines = Vec::new();
        let mut n = 0usize;
        for (path, diags) in &self.by_file {
            for d in diags {
                if n >= limit {
                    lines.push(format!("… ({} more)", self.total().saturating_sub(limit)));
                    return lines.join("\n");
                }
                let sev = match d.severity {
                    1 => "error",
                    2 => "warn",
                    3 => "info",
                    _ => "hint",
                };
                lines.push(format!(
                    "{path}:{}:{} [{sev}] {}",
                    d.line + 1,
                    d.character + 1,
                    d.message
                ));
                n += 1;
            }
        }
        if lines.is_empty() {
            "No diagnostics.".into()
        } else {
            lines.join("\n")
        }
    }

    fn total(&self) -> usize {
        self.by_file.values().map(|v| v.len()).sum()
    }
}

/// Shared diagnostics bag updated by the LSP reader task.
pub type SharedDiagnostics = Arc<Mutex<DiagnosticSnapshot>>;

pub struct LspClient {
    #[allow(dead_code)]
    child: Child,
    stdin: AsyncMutex<ChildStdin>,
    next_id: std::sync::atomic::AtomicU64,
    pub root: PathBuf,
    pub command: String,
    pub diagnostics: SharedDiagnostics,
}

impl LspClient {
    /// Start `rust-analyzer` (or `RS_AGENT_LSP` override) for `root`.
    pub async fn start_rust_analyzer(root: PathBuf) -> Result<(Self, tokio::task::JoinHandle<()>), String> {
        let cmd = std::env::var("RS_AGENT_LSP").unwrap_or_else(|_| "rust-analyzer".into());
        Self::start(cmd, root).await
    }

    pub async fn start(
        command: String,
        root: PathBuf,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), String> {
        let mut child = Command::new(&command)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("spawn LSP `{command}`: {e}"))?;
        let stdin = child.stdin.take().ok_or("LSP: no stdin")?;
        let stdout = child.stdout.take().ok_or("LSP: no stdout")?;
        let diagnostics: SharedDiagnostics = Arc::new(Mutex::new(DiagnosticSnapshot::default()));
        let diags_reader = diagnostics.clone();

        let handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_message(&mut reader).await {
                    Ok(msg) => {
                        if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                            if method == "textDocument/publishDiagnostics" {
                                if let Some(params) = msg.get("params") {
                                    apply_publish_diagnostics(diags_reader.clone(), params);
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let client = Self {
            child,
            stdin: AsyncMutex::new(stdin),
            next_id: std::sync::atomic::AtomicU64::new(1),
            root: root.clone(),
            command,
            diagnostics,
        };

        let root_uri = path_to_uri(&root);
        client
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "capabilities": {
                        "textDocument": {
                            "publishDiagnostics": {},
                            "synchronization": { "didSave": true }
                        }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "root" }]
                }),
            )
            .await?;
        client.notify("initialized", json!({})).await?;
        Ok((client, handle))
    }

    pub async fn did_open(&self, path: &Path, text: &str, language_id: &str) -> Result<(), String> {
        let uri = path_to_uri(path);
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text
                }
            }),
        )
        .await
    }

    pub async fn did_save(&self, path: &Path, text: Option<&str>) -> Result<(), String> {
        let uri = path_to_uri(path);
        let mut params = json!({ "textDocument": { "uri": uri } });
        if let Some(t) = text {
            params["text"] = json!(t);
        }
        self.notify("textDocument/didSave", params).await
    }

    pub fn snapshot(&self) -> DiagnosticSnapshot {
        self.diagnostics.lock().map(|g| g.clone()).unwrap_or_default()
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.write_message(&msg).await?;
        // Minimal: don't wait for response body (initialize result unused).
        Ok(Value::Null)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.write_message(&msg).await
    }

    async fn write_message(&self, msg: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdin.write_all(&body).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn path_to_uri(path: &Path) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", abs.display())
}

fn apply_publish_diagnostics(store: SharedDiagnostics, params: &Value) {
    let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
    let path = uri_to_path(uri);
    let empty: Vec<Value> = Vec::new();
    let diags = params
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .unwrap_or(&empty);
    let mut list = Vec::new();
    for d in diags {
        let severity = d.get("severity").and_then(|s| s.as_u64()).unwrap_or(1) as u8;
        let message = d
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let line = d
            .pointer("/range/start/line")
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as u32;
        let character = d
            .pointer("/range/start/character")
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as u32;
        let source = d
            .get("source")
            .and_then(|s| s.as_str())
            .unwrap_or("lsp")
            .to_string();
        list.push(Diagnostic {
            path: path.clone(),
            line,
            character,
            severity,
            message,
            source,
        });
    }
    if let Ok(mut g) = store.lock() {
        if list.is_empty() {
            g.by_file.remove(&path);
        } else {
            g.by_file.insert(path, list);
        }
    }
}

fn uri_to_path(uri: &str) -> String {
    uri.strip_prefix("file://")
        .unwrap_or(uri)
        .to_string()
}

async fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<Value, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("EOF".into());
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = content_length.ok_or("missing Content-Length")?;
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| e.to_string())
}

/// Guess language id from path extension.
pub fn language_id_for(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str())? {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "c" => Some("c"),
        "cpp" | "cc" | "cxx" | "h" | "hpp" => Some("cpp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_empty() {
        let s = DiagnosticSnapshot::default();
        assert_eq!(s.summary_line(), "");
    }

    #[test]
    fn counts_errors() {
        let mut s = DiagnosticSnapshot::default();
        s.by_file.insert(
            "a.rs".into(),
            vec![Diagnostic {
                path: "a.rs".into(),
                line: 0,
                character: 0,
                severity: 1,
                message: "boom".into(),
                source: "ra".into(),
            }],
        );
        assert!(s.summary_line().contains("E:1"));
    }
}
