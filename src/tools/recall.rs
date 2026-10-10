//! `recall` — read one archived trajectory step back into the window.

use crate::agent::tool::*;
use crate::agent::trajectory;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct RecallArgs {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    query: Option<String>,
}

pub struct RecallTool;

#[async_trait]
impl AgentTool for RecallTool {
    fn name(&self) -> &str {
        "recall"
    }

    fn description(&self) -> &str {
        "Read one archived conversation step by id (s1, s2, …) from the trajectory index \
         printed after compaction. Optional query searches step text and returns matching ids."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Step id from the trajectory index, e.g. s12"},
                "query": {"type": "string", "description": "Search archived step text; returns matching ids"}
            }
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: RecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {e}")),
        };
        if let Some(id) = parsed
            .id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return match trajectory::recall(id) {
                Ok(text) => ToolExecuteResult::ok(text),
                Err(e) => ToolExecuteResult::error(e),
            };
        }
        if let Some(q) = parsed
            .query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let hits = trajectory::search(q, 8);
            if hits.is_empty() {
                return ToolExecuteResult::ok(format!("No trajectory steps match `{q}`."));
            }
            let mut out = String::from("Matching steps (recall an id for the full text):\n");
            for (id, preview) in hits {
                out.push_str(&format!("{id} {preview}\n"));
            }
            return ToolExecuteResult::ok(out);
        }
        ToolExecuteResult::error("recall requires id or query")
    }
}
