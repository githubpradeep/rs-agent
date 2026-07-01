use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

#[derive(Deserialize)]
pub struct EditArgs {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
}

pub struct EditTool;

#[async_trait]
impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a file using exact text replacement. Finds old_string and replaces it with new_string. old_string must match a unique, non-overlapping region of the file. Use this for surgical edits instead of full rewrites."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: EditArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        let content = match fs::read_to_string(&parsed.file_path).await {
            Ok(c) => c,
            Err(e) => return ToolExecuteResult::error(format!("Failed to read {}: {}", parsed.file_path, e)),
        };

        if !content.contains(&parsed.old_string) {
            return ToolExecuteResult::error(format!(
                "Could not find old_string in {}\n\nfile content:\n{}",
                parsed.file_path,
                content.chars().take(2000).collect::<String>()
            ));
        }

        let count = content.matches(&parsed.old_string).count();
        if count > 1 {
            return ToolExecuteResult::error(format!(
                "Found {} occurrences of old_string in {}. Please provide more context to uniquely identify the match.",
                count, parsed.file_path
            ));
        }

        let new_content = content.replace(&parsed.old_string, &parsed.new_string);

        match fs::write(&parsed.file_path, &new_content).await {
            Ok(_) => ToolExecuteResult::ok(format!(
                "Successfully edited {} (replaced {} chars with {} chars)",
                parsed.file_path,
                parsed.old_string.len(),
                parsed.new_string.len()
            )),
            Err(e) => ToolExecuteResult::error(format!("Failed to write {}: {}", parsed.file_path, e)),
        }
    }
}
