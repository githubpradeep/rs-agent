use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Deserialize)]
pub struct BashArgs {
    pub command: String,
    pub timeout: Option<u64>,
    pub workdir: Option<String>,
}

pub struct BashTool;

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in the current working directory. Returns stdout and stderr. Use this to run commands, install packages, run tests, build projects, etc. Output is truncated to 10000 characters."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in milliseconds",
                    "default": 30000
                },
                "workdir": {
                    "type": "string",
                    "description": "Optional working directory"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: BashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        let timeout = Duration::from_millis(parsed.timeout.unwrap_or(30_000));

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(&parsed.command);

        if let Some(dir) = &parsed.workdir {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolExecuteResult::error(format!("Failed to spawn: {}", e)),
        };

        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut text = String::new();

                if !output.stdout.is_empty() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    text.push_str(&stdout);
                }

                if !output.stderr.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    text.push_str(&stderr);
                }

                let exit_code = output.status.code().unwrap_or(-1);
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&format!("Exit code: {}", exit_code));

                if exit_code != 0 {
                    let result_text = if text.len() > 10000 {
                        let truncated = text.chars().take(10000).collect::<String>();
                        format!("{}\n... (truncated, {} total chars)", truncated, text.len())
                    } else {
                        text
                    };
                    ToolExecuteResult::error(result_text)
                } else {
                    if text.len() > 10000 {
                        let truncated = text.chars().take(10000).collect::<String>();
                        ToolExecuteResult::ok(format!("{}\n... (truncated, {} total chars)", truncated, text.len()))
                    } else {
                        ToolExecuteResult::ok(text)
                    }
                }
            }
            Ok(Err(e)) => ToolExecuteResult::error(format!("Command failed: {}", e)),
            Err(_) => ToolExecuteResult::error(format!(
                "Command timed out after {}ms: {}",
                timeout.as_millis(),
                parsed.command
            )),
        }
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }
}
