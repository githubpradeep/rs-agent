use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

#[derive(Deserialize)]
pub struct ReadArgs {
    pub file_path: String,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

pub struct ReadTool;

#[async_trait]
impl AgentTool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Supports text files. Can read specific line ranges with offset and limit. Use this to examine files instead of using cat or sed."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start from (1-indexed)",
                    "default": 1
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read",
                    "default": 2000
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: ReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        let content = match fs::read_to_string(&parsed.file_path).await {
            Ok(c) => c,
            Err(e) => return ToolExecuteResult::error(format!("Failed to read {}: {}", parsed.file_path, e)),
        };

        let lines: Vec<&str> = content.lines().collect();
        let offset = parsed.offset.unwrap_or(1).max(1) - 1;
        let limit = parsed.limit.unwrap_or(2000);

        if offset >= lines.len() {
            return ToolExecuteResult::error(format!(
                "Offset {} is beyond file length {}",
                offset + 1,
                lines.len()
            ));
        }

        let end = (offset + limit).min(lines.len());
        let selected = &lines[offset..end];

        let numbered: Vec<String> = selected
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}: {}", offset + i + 1, line))
            .collect();

        let result = numbered.join("\n");
        if result.len() > 50000 {
            let truncated = result.chars().take(50000).collect::<String>();
            ToolExecuteResult::ok(format!("{}\n... (truncated, {} total chars)", truncated, result.len()))
        } else {
            ToolExecuteResult::ok(result)
        }
    }
}
