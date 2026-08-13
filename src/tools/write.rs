use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

#[derive(Deserialize)]
pub struct WriteArgs {
    pub file_path: String,
    pub content: String,
}

pub struct WriteTool;

#[async_trait]
impl AgentTool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Overwrites the entire file. Creates parent directories automatically. Use this for creating new files or complete rewrites of existing files."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file (absolute or relative to current directory)"
                },
                "content": {
                    "type": "string",
                    "description": "The full content to write"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let args = crate::tools::normalize_file_tool_args(args);
        let parsed: WriteArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid args: {e}. Expected file_path + content (aliases: path, contents). Got keys: {}",
                    args.as_object()
                        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_else(|| "(not an object)".into())
                ))
            }
        };

        let path = parsed.file_path.clone();
        crate::tools::mutation_queue::with_file_lock(&path, || async {
            let _ = crate::tools::turn_snapshot::track(&path);

            if let Some(parent) = std::path::Path::new(&path).parent() {
                if !parent.as_os_str().is_empty() {
                    if let Err(e) = fs::create_dir_all(parent).await {
                        return ToolExecuteResult::error(format!(
                            "Failed to create directory {}: {}",
                            parent.display(),
                            e
                        ));
                    }
                }
            }

            match atomic_write(&path, &parsed.content).await {
                Ok(_) => {
                    let body = format!(
                        "Successfully wrote {} bytes to {}",
                        parsed.content.len(),
                        path
                    );
                    let body = crate::tools::post_mutation::after_mutation(&path, body).await;
                    ToolExecuteResult::ok(body)
                }
                Err(e) => ToolExecuteResult::error(format!("Failed to write {}: {}", path, e)),
            }
        })
        .await
    }
}

/// Write via temp file + rename so mid-crash doesn't leave truncated content.
pub async fn atomic_write(path: &str, content: &str) -> Result<(), String> {
    atomic_write_bytes(path, content.as_bytes()).await
}

/// Binary-safe atomic write (temp + rename).
pub async fn atomic_write_bytes(path: &str, content: &[u8]) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let parent = p.parent().filter(|x| !x.as_os_str().is_empty());
    let tmp = match parent {
        Some(dir) => {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            dir.join(format!(".{name}.rs-agent.tmp"))
        }
        None => std::path::PathBuf::from(format!(".{path}.rs-agent.tmp")),
    };
    fs::write(&tmp, content)
        .await
        .map_err(|e| format!("temp write: {e}"))?;
    fs::rename(&tmp, path).await.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename: {e}")
    })?;
    Ok(())
}
