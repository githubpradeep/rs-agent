use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

#[derive(Deserialize)]
pub struct LsArgs {
    pub path: Option<String>,
}

pub struct LsTool;

#[async_trait]
impl AgentTool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List files and directories in a given path. Shows file sizes and modification times. Use this to explore project structure instead of bash ls."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to list"
                }
            }
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: LsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        let dir_path = parsed.path.unwrap_or_else(|| ".".to_string());
        let mut entries = match fs::read_dir(&dir_path).await {
            Ok(e) => e,
            Err(e) => return ToolExecuteResult::error(format!("Failed to read dir {}: {}", dir_path, e)),
        };

        let mut result = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
            let metadata = entry.metadata().await;
            let file_name = entry.file_name().to_string_lossy().to_string();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

            let size_str = if is_dir {
                String::new()
            } else if size < 1024 {
                format!("{}B", size)
            } else if size < 1024 * 1024 {
                format!("{}KB", size / 1024)
            } else {
                format!("{}MB", size / (1024 * 1024))
            };

            let modified = metadata
                .map(|m| m.modified().ok())
                .unwrap_or(None)
                .map(|t| {
                    let duration = t
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    let secs = duration.as_secs();
                    let dt = chrono::DateTime::from_timestamp(secs as i64, 0)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_default();
                    dt
                })
                .unwrap_or_default();

            let entry_str = if is_dir {
                format!("{} {}/", modified, file_name)
            } else {
                format!("{} {:>8} {}", modified, size_str, file_name)
            };
            result.push(entry_str);
        }

        result.sort();
        ToolExecuteResult::ok(result.join("\n"))
    }
}
