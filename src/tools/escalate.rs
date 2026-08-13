//! `escalate` tool — right to refuse / ask for a human.

use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Default)]
pub struct EscalateState {
    pub pending: bool,
    pub reason: String,
    pub needs: String,
}

static ESCALATE: OnceLock<Mutex<EscalateState>> = OnceLock::new();

fn slot() -> &'static Mutex<EscalateState> {
    ESCALATE.get_or_init(|| Mutex::new(EscalateState::default()))
}

pub fn take_pending() -> Option<EscalateState> {
    let mut g = slot().lock().ok()?;
    if !g.pending {
        return None;
    }
    let out = g.clone();
    *g = EscalateState::default();
    Some(out)
}

pub fn peek_pending() -> Option<EscalateState> {
    let g = slot().lock().ok()?;
    if g.pending {
        Some(g.clone())
    } else {
        None
    }
}

pub fn clear() {
    if let Ok(mut g) = slot().lock() {
        *g = EscalateState::default();
    }
}

#[derive(Deserialize)]
struct EscalateArgs {
    reason: String,
    #[serde(default)]
    needs: Option<String>,
}

pub struct EscalateTool;

#[async_trait]
impl AgentTool for EscalateTool {
    fn name(&self) -> &str {
        "escalate"
    }

    fn description(&self) -> &str {
        "Pause autonomous work and escalate to a human. Use when the task needs \
         judgment, credentials, irreversible decisions, or is outside standing orders. \
         Ends the turn and pauses any active /goal."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Why this needs a human"
                },
                "needs": {
                    "type": "string",
                    "description": "human | review | credentials | decision (default: human)"
                }
            },
            "required": ["reason"]
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: EscalateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid escalate args: {e}. Expected {{reason, needs?}}."
                ))
            }
        };
        let reason = parsed.reason.trim().to_string();
        if reason.is_empty() {
            return ToolExecuteResult::error("reason must not be empty");
        }
        let needs = parsed
            .needs
            .unwrap_or_else(|| "human".into())
            .trim()
            .to_lowercase();
        if let Ok(mut g) = slot().lock() {
            g.pending = true;
            g.reason = reason.clone();
            g.needs = needs.clone();
        }
        // Phase E: also drop mail to Seneschal so remote triage works.
        let from = crate::tools::handoff::active_seat().unwrap_or_else(|| "agent".into());
        let _ = crate::mail::send(
            &from,
            "Seneschal",
            &format!("ESCALATE (needs: {needs}): {reason}"),
            vec![],
        );
        ToolExecuteResult::terminate(format!(
            "Escalated (needs: {needs}): {reason}\n\
             Autonomous work paused — waiting on human. Mail sent to Seneschal."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn escalate_sets_pending() {
        clear();
        let tool = EscalateTool;
        let r = tool
            .execute(
                "1",
                json!({"reason": "needs API key", "needs": "credentials"}),
            )
            .await;
        assert!(r.terminate);
        let p = take_pending().unwrap();
        assert!(p.reason.contains("API key"));
        assert_eq!(p.needs, "credentials");
        clear();
    }
}
