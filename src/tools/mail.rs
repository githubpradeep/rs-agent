//! `mail` tool — send/read/ack city inbox messages.

use crate::agent::tool::*;
use crate::mail;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct MailArgs {
    /// send | read | inbox | ack
    action: String,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    beads: Option<Vec<String>>,
}

pub struct MailTool;

#[async_trait]
impl AgentTool for MailTool {
    fn name(&self) -> &str {
        "mail"
    }

    fn description(&self) -> &str {
        "City mail (.rs-agent/mail). Actions: send (to, body, beads?), read/inbox, ack (id). \
         Escalate human needs to Seneschal via send."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "description": "send | read | inbox | ack"},
                "to": {"type": "string"},
                "body": {"type": "string"},
                "id": {"type": "string"},
                "from": {"type": "string"},
                "beads": {"type": "array", "items": {"type": "string"}}
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
        let parsed: MailArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid mail args: {e}")),
        };
        let action = parsed.action.trim().to_lowercase();
        let from = parsed
            .from
            .or_else(crate::tools::handoff::active_seat)
            .unwrap_or_else(|| "agent".into());
        match action.as_str() {
            "send" => {
                let to = parsed.to.unwrap_or_else(|| "Seneschal".into());
                let body = parsed.body.unwrap_or_default();
                match mail::send(&from, &to, &body, parsed.beads.unwrap_or_default()) {
                    Ok(m) => ToolExecuteResult::ok(format!(
                        "Sent {} → {} ({})",
                        m.id, m.to, m.created_at
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "read" | "inbox" | "list" => {
                ToolExecuteResult::ok(mail::format_inbox(Some(&from)))
            }
            "ack" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("ack requires id");
                }
                match mail::ack(&id) {
                    Ok(m) => ToolExecuteResult::ok(format!("Acked {}", m.id)),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            _ => ToolExecuteResult::error("Unknown mail action. Use send|read|ack."),
        }
    }
}
