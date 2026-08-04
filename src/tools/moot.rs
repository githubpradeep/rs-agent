//! `moot` tool — open/append/close agent meeting threads.

use crate::agent::tool::*;
use crate::moot;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct MootArgs {
    action: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

pub struct MootTool;

#[async_trait]
impl AgentTool for MootTool {
    fn name(&self) -> &str {
        "moot"
    }

    fn description(&self) -> &str {
        "Agent meeting threads (.rs-agent/moot). Actions: open (topic), append (id, text), \
         close (id, summary?), show (id), list."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string"},
                "id": {"type": "string"},
                "topic": {"type": "string"},
                "text": {"type": "string"},
                "from": {"type": "string"},
                "summary": {"type": "string"}
            },
            "required": ["action"]
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: MootArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid moot args: {e}")),
        };
        let action = parsed.action.trim().to_lowercase();
        let from = parsed
            .from
            .or_else(crate::tools::handoff::active_seat)
            .unwrap_or_else(|| "agent".into());
        match action.as_str() {
            "open" => {
                let topic = parsed.topic.or(parsed.text).unwrap_or_default();
                match moot::open(&topic) {
                    Ok(m) => ToolExecuteResult::ok(format!("Opened {} — {}", m.id, m.topic)),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "append" | "say" => {
                let id = parsed.id.unwrap_or_default();
                let text = parsed.text.unwrap_or_default();
                match moot::append(&id, &from, &text) {
                    Ok(m) => ToolExecuteResult::ok(format!(
                        "Appended to {} ({} entries)",
                        m.id,
                        m.entries.len()
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "close" => {
                let id = parsed.id.unwrap_or_default();
                match moot::close(&id, parsed.summary.as_deref().or(parsed.text.as_deref())) {
                    Ok(m) => ToolExecuteResult::ok(format!("Closed {}", m.id)),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "show" => {
                let id = parsed.id.unwrap_or_default();
                match moot::show(&id) {
                    Ok(s) => ToolExecuteResult::ok(s),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "list" | "ls" => ToolExecuteResult::ok(moot::list()),
            _ => ToolExecuteResult::error("Unknown moot action. Use open|append|close|show|list."),
        }
    }
}
