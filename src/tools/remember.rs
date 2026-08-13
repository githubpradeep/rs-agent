//! `remember` / brain tool — write and falsify operational facts.

use crate::agent::tool::*;
use crate::brain;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct RememberArgs {
    /// remember | falsify | facts
    #[serde(default = "default_action")]
    action: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    query: Option<String>,
}

fn default_action() -> String {
    "remember".into()
}

pub struct RememberTool;

#[async_trait]
impl AgentTool for RememberTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn description(&self) -> &str {
        "Project brain facts (brain/facts.jsonl). Actions: remember (text), falsify (query), facts."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "description": "remember | falsify | facts"},
                "text": {"type": "string"},
                "query": {"type": "string", "description": "id or substring for falsify"}
            }
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: RememberArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid remember args: {e}")),
        };
        let action = parsed.action.trim().to_lowercase();
        match action.as_str() {
            "remember" | "add" => {
                let text = parsed.text.or(parsed.query).unwrap_or_default();
                match brain::remember(&text) {
                    Ok(f) => ToolExecuteResult::ok(format!(
                        "Remembered {} — {}",
                        f.id.as_deref().unwrap_or("?"),
                        f.text
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "falsify" => {
                let q = parsed.query.or(parsed.text).unwrap_or_default();
                match brain::falsify(&q) {
                    Ok(n) => ToolExecuteResult::ok(format!("Falsified {n} fact(s)")),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "facts" | "list" => {
                let facts = brain::recent_facts(20);
                if facts.is_empty() {
                    ToolExecuteResult::ok("No active facts.")
                } else {
                    let mut out = String::from("Facts:\n");
                    for f in facts {
                        out.push_str(&format!(
                            "  [{}] {} — {}\n",
                            f.id.as_deref().unwrap_or("-"),
                            f.written_at,
                            f.text
                        ));
                    }
                    ToolExecuteResult::ok(out)
                }
            }
            _ => ToolExecuteResult::error("Unknown action. Use remember|falsify|facts."),
        }
    }
}
