pub mod apply_patch;
pub mod bash;
pub mod bead;
pub mod diffutil;
pub mod edit;
pub mod escalate;
pub mod find;
pub mod grep;
pub mod handoff;
pub mod ls;
pub mod mail;
pub mod moot;
pub mod mutation_queue;
pub mod output_sink;
pub mod post_mutation;
pub mod question;
pub mod read;
pub mod remember;
pub mod repl_tool;
pub mod task;
pub mod todowrite;
pub mod truncate_store;
pub mod turn_snapshot;
pub mod web_fetch;
pub mod web_search;
pub mod write;

pub use apply_patch::ApplyPatchTool;
pub use bash::BashTool;
pub use bead::BeadTool;
pub use edit::EditTool;
pub use escalate::EscalateTool;
pub use find::FindTool;
pub use grep::GrepTool;
pub use handoff::HandoffTool;
pub use ls::LsTool;
pub use mail::MailTool;
pub use moot::MootTool;
pub use question::QuestionTool;
pub use read::ReadTool;
pub use remember::RememberTool;
pub use repl_tool::ReplTool;
pub use task::TaskTool;
pub use todowrite::TodoWriteTool;
pub use web_fetch::WebFetchTool;
pub use web_search::WebSearchTool;
pub use write::WriteTool;

use crate::agent::tool::SharedTool;
use crate::agent::AgentLoop;
use serde_json::{Map, Value};
use std::sync::Arc;

/// Tolerate common LLM arg aliases (`path`/`contents`/…) for file tools.
pub(crate) fn normalize_file_tool_args(args: Value) -> Value {
    let Value::Object(mut map) = args else {
        return args;
    };
    alias_into(&mut map, "file_path", &["path", "file", "filename", "filepath"]);
    alias_into(&mut map, "content", &["contents", "text", "body", "data"]);
    alias_into(&mut map, "old_string", &["old", "old_str", "oldString", "search"]);
    alias_into(&mut map, "new_string", &["new", "new_str", "newString", "replace"]);
    // replace_all aliases
    if !map.contains_key("replace_all") {
        for alias in ["replaceAll", "all", "global"] {
            if let Some(v) = map.remove(alias) {
                map.insert("replace_all".into(), v);
                break;
            }
        }
    }
    Value::Object(map)
}

fn alias_into(map: &mut Map<String, Value>, canonical: &str, aliases: &[&str]) {
    if map.get(canonical).map(|v| !v.is_null()).unwrap_or(false) {
        return;
    }
    for alias in aliases {
        if let Some(v) = map.remove(*alias) {
            if !v.is_null() {
                map.insert(canonical.to_string(), v);
                return;
            }
        }
    }
}

pub fn default_tools_list() -> Vec<SharedTool> {
    vec![
        Arc::new(BashTool) as SharedTool,
        Arc::new(ReadTool) as SharedTool,
        Arc::new(WriteTool) as SharedTool,
        Arc::new(EditTool) as SharedTool,
        Arc::new(ApplyPatchTool) as SharedTool,
        Arc::new(GrepTool) as SharedTool,
        Arc::new(LsTool) as SharedTool,
        Arc::new(FindTool) as SharedTool,
        Arc::new(WebSearchTool) as SharedTool,
        Arc::new(WebFetchTool) as SharedTool,
        Arc::new(TodoWriteTool) as SharedTool,
        Arc::new(QuestionTool) as SharedTool,
        Arc::new(HandoffTool) as SharedTool,
        Arc::new(BeadTool) as SharedTool,
        Arc::new(EscalateTool) as SharedTool,
        Arc::new(MailTool) as SharedTool,
        Arc::new(RememberTool) as SharedTool,
        Arc::new(MootTool) as SharedTool,
    ]
}

pub fn register_default_tools(agent: &mut AgentLoop) {
    for tool in default_tools_list() {
        agent.register_tool(tool);
    }
}

/// Register default tools + RLM repl tool.
pub fn register_default_tools_with_rlm(agent: &mut AgentLoop, max_rlm_depth: u32) {
    register_default_tools(agent);
    register_rlm_tools(agent, agent.rlm_depth(), max_rlm_depth);
}

pub fn register_rlm_tools(agent: &mut AgentLoop, depth: u32, max_depth: u32) {
    let _ = depth;
    if agent.tools().get("repl").is_none() {
        repl_tool::attach_repl_tool(agent, max_depth);
    }
    if agent.tools().get("task").is_none() {
        task::attach_task_tool(agent, max_depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_path_and_contents_aliases() {
        let v = normalize_file_tool_args(json!({
            "path": "snake.py",
            "contents": "print(1)"
        }));
        assert_eq!(v["file_path"], "snake.py");
        assert_eq!(v["content"], "print(1)");
        assert!(v.get("path").is_none());
        assert!(v.get("contents").is_none());
    }

    #[test]
    fn keeps_canonical_fields() {
        let v = normalize_file_tool_args(json!({
            "file_path": "a.py",
            "path": "ignored.py",
            "content": "x"
        }));
        assert_eq!(v["file_path"], "a.py");
        assert_eq!(v["content"], "x");
    }
}
