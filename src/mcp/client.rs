//! MCP stdio transport: Content-Length framed JSON-RPC 2.0.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct McpServerSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Option<McpToolAnnotations>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpToolAnnotations {
    #[serde(default, rename = "readOnlyHint")]
    pub read_only_hint: Option<bool>,
}

pub struct McpClient {
    pub server_name: String,
    #[allow(dead_code)]
    child: Child,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    next_id: AtomicU64,
}

impl McpClient {
    pub async fn start(spec: &McpServerSpec) -> Result<Self, String> {
        let mut cmd = Command::new(&spec.command);
        cmd.args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn MCP `{}` ({}): {e}", spec.name, spec.command))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("MCP `{}`: no stdin", spec.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("MCP `{}`: no stdout", spec.name))?;

        let client = Self {
            server_name: spec.name.clone(),
            child,
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
            next_id: AtomicU64::new(1),
        };

        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "rs-agent", "version": "0.1.0"}
                }),
            )
            .await?;
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, String> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for t in tools {
            match serde_json::from_value::<McpToolInfo>(t) {
                Ok(info) => out.push(info),
                Err(e) => tracing::warn!(error = %e, "skip malformed MCP tool"),
            }
        }
        Ok(out)
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<(String, bool), String> {
        let result = self
            .request(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": arguments,
                }),
            )
            .await?;
        let is_error = result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
        Ok((format_tool_result(&result), is_error))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&msg).await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&msg).await?;

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(format!(
                    "MCP `{}` timed out waiting for {method}",
                    self.server_name
                ));
            }
            let resp = self.read_message().await?;
            let resp_id = resp
                .get("id")
                .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)));
            if resp_id == Some(id) {
                if let Some(err) = resp.get("error") {
                    return Err(format!("MCP `{}` {method} error: {err}", self.server_name));
                }
                return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    async fn write_message(&self, msg: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| format!("MCP write header: {e}"))?;
        stdin
            .write_all(&body)
            .await
            .map_err(|e| format!("MCP write body: {e}"))?;
        stdin.flush().await.map_err(|e| format!("MCP flush: {e}"))?;
        Ok(())
    }

    async fn read_message(&self) -> Result<Value, String> {
        let mut stdout = self.stdout.lock().await;
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = stdout
                .read_line(&mut line)
                .await
                .map_err(|e| format!("MCP read header: {e}"))?;
            if n == 0 {
                return Err(format!("MCP `{}` closed stdout", self.server_name));
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                break;
            }
            if let Some(rest) = trimmed
                .strip_prefix("Content-Length:")
                .or_else(|| trimmed.strip_prefix("content-length:"))
            {
                content_length = rest.trim().parse().ok();
            }
        }
        let len = content_length
            .ok_or_else(|| format!("MCP `{}`: missing Content-Length", self.server_name))?;
        let mut buf = vec![0u8; len];
        stdout
            .read_exact(&mut buf)
            .await
            .map_err(|e| format!("MCP read body: {e}"))?;
        serde_json::from_slice(&buf).map_err(|e| format!("MCP JSON: {e}"))
    }
}

fn format_tool_result(result: &Value) -> String {
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        let mut parts = Vec::new();
        for item in content {
            let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("text");
            if ty == "text" {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(t.to_string());
                }
            } else {
                parts.push(item.to_string());
            }
        }
        return parts.join("\n");
    }
    result.to_string()
}
