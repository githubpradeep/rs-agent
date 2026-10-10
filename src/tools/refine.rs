//! `refine` — propose a brain fact or skill note. Does not apply it.

use crate::agent::refine::{self, RefineKind};
use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct RefineArgs {
    /// propose | list
    #[serde(default = "default_action")]
    action: String,
    /// fact | skill
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

fn default_action() -> String {
    "propose".into()
}

pub struct RefineTool;

#[async_trait]
impl AgentTool for RefineTool {
    fn name(&self) -> &str {
        "refine"
    }

    fn description(&self) -> &str {
        "Propose a small harness update from evidence in this session. \
         action=propose with kind=fact|skill, text, and evidence (a short quote of what happened). \
         action=list shows pending proposals. This does not apply the change and never edits the system prompt. \
         A person runs /refine apply <id>."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "description": "propose | list"},
                "kind": {"type": "string", "description": "fact | skill"},
                "text": {"type": "string", "description": "The fact or skill note to store if a person applies it"},
                "evidence": {"type": "string", "description": "What in this session justifies the note"}
            },
            "required": ["action"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: RefineArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {e}")),
        };
        match parsed.action.trim().to_lowercase().as_str() {
            "list" | "ls" => ToolExecuteResult::ok(refine::format_list(&refine::list_all())),
            "propose" | "" => {
                let kind = match parsed.kind.as_deref().unwrap_or("fact").trim() {
                    "skill" | "skill_note" | "note" => RefineKind::SkillNote,
                    "fact" | "" => RefineKind::Fact,
                    other => {
                        return ToolExecuteResult::error(format!(
                            "kind must be fact or skill, got {other}"
                        ))
                    }
                };
                let text = parsed.text.unwrap_or_default();
                let evidence = parsed.evidence.unwrap_or_default();
                match refine::propose(kind, &text, &evidence) {
                    Ok(p) => ToolExecuteResult::ok(format!(
                        "Proposed {} (pending). A person applies it with /refine apply {}.",
                        p.id, p.id
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            other => ToolExecuteResult::error(format!(
                "Unknown refine action `{other}`. Use propose or list. Applying is /refine apply."
            )),
        }
    }
}
