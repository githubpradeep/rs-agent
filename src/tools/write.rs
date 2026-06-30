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

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: WriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        if let Some(parent) = std::path::Path::new(&parsed.file_path).parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = fs::create_dir_all(parent).await {
                    return ToolExecuteResult::error(format!("Failed to create directory {}: {}", parent.display(), e));
                }
            }
        }

        match fs::write(&parsed.file_path, &parsed.content).await {
            Ok(_) => ToolExecuteResult::ok(format!("Successfully wrote {} bytes to {}", parsed.content.len(), parsed.file_path)),
            Err(e) => ToolExecuteResult::error(format!("Failed to write {}: {}", parsed.file_path, e)),
        }
    }
}
