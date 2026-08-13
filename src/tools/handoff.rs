//! `handoff` tool — agent consents to close the day and write continuity notes.

use crate::agent::handoff::{self, HandoffNotes};
use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::cell::RefCell;

// Active seat name for diary append (set when `/seat` binds) — per session runtime thread.
thread_local! {
    static ACTIVE_SEAT: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn set_active_seat(name: Option<String>) {
    ACTIVE_SEAT.with(|s| *s.borrow_mut() = name);
}

pub fn active_seat() -> Option<String> {
    ACTIVE_SEAT.with(|s| s.borrow().clone())
}

#[derive(Deserialize)]
struct HandoffArgs {
    summary: String,
    #[serde(default)]
    open_threads: Option<String>,
    #[serde(default)]
    next_steps: Option<String>,
    #[serde(default)]
    beads_touched: Option<Vec<String>>,
}

pub struct HandoffTool;

#[async_trait]
impl AgentTool for HandoffTool {
    fn name(&self) -> &str {
        "handoff"
    }

    fn description(&self) -> &str {
        "Close out this session cleanly: write handoff notes for the next wake. \
         Call when the user asks you to hand off, or when approaching context limits. \
         Ends the current turn after notes are saved."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "What you accomplished and key context"
                },
                "open_threads": {
                    "type": "string",
                    "description": "Unresolved issues / blockers"
                },
                "next_steps": {
                    "type": "string",
                    "description": "Concrete next actions for the next session"
                },
                "beads_touched": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Bead ids you claimed/closed/touched"
                }
            },
            "required": ["summary"]
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: HandoffArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid handoff args: {e}. Expected {{summary, open_threads?, next_steps?}}."
                ))
            }
        };
        let summary = parsed.summary.trim().to_string();
        if summary.is_empty() {
            return ToolExecuteResult::error("summary must not be empty");
        }

        let notes = HandoffNotes::new(
            summary,
            parsed.open_threads.unwrap_or_default(),
            parsed.next_steps.unwrap_or_default(),
            parsed.beads_touched.unwrap_or_default(),
        );
        handoff::store(notes.clone());

        if let Some(seat_name) = active_seat() {
            if let Ok(mut seat) = crate::agent::seat::load(&seat_name) {
                seat.append_handoff(notes.clone());
                let _ = crate::agent::seat::save(&seat);
            }
        }

        ToolExecuteResult::terminate(format!("Handoff recorded.\n{}", notes.format_block()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn requires_summary() {
        let tool = HandoffTool;
        let r = tool.execute("1", json!({"summary": "  "})).await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn terminates_on_success() {
        handoff::clear();
        let tool = HandoffTool;
        let r = tool
            .execute("1", json!({"summary": "done", "next_steps": "sleep"}))
            .await;
        assert!(!r.is_error);
        assert!(r.terminate);
        assert!(handoff::snapshot().unwrap().summary.contains("done"));
        handoff::clear();
    }
}
