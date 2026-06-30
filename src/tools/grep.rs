use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    pub path: Option<String>,
    pub include: Option<String>,
}

pub struct GrepTool;

#[async_trait]
impl AgentTool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for a pattern in files using ripgrep (rg). Supports regex patterns. Returns matching file paths with line numbers. Use this to find relevant code, function definitions, usages, etc."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to current dir)"
                },
                "include": {
                    "type": "string",
                    "description": "File pattern to include (e.g. *.rs, *.{ts,js})"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: GrepArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        let search_path = parsed.path.unwrap_or_else(|| ".".to_string());

        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("-n");
        cmd.arg("--color");
        cmd.arg("never");
        cmd.arg(&parsed.pattern);
        cmd.arg(&search_path);

        if let Some(include) = &parsed.include {
            cmd.arg("-g");
            cmd.arg(include);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let output = match cmd.output().await {
            Ok(o) => o,
            Err(e) => {
                return ToolExecuteResult::error(format!("Failed to run rg: {}. Is ripgrep installed?", e));
            }
        };

        if output.status.code() == Some(2) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return ToolExecuteResult::error(stderr.to_string());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.is_empty() {
            return ToolExecuteResult::ok("No matches found.");
        }

        if stdout.len() > 10000 {
            let truncated = stdout.chars().take(10000).collect::<String>();
            ToolExecuteResult::ok(format!("{}\n... (truncated, {} total chars)", truncated, stdout.len()))
        } else {
            ToolExecuteResult::ok(stdout.to_string())
        }
    }
}
