pub mod bash;
pub mod edit;
pub mod find;
pub mod grep;
pub mod ls;
pub mod read;
pub mod repl_tool;
pub mod web_fetch;
pub mod web_search;
pub mod write;

pub use bash::BashTool;
pub use edit::EditTool;
pub use find::FindTool;
pub use grep::GrepTool;
pub use ls::LsTool;
pub use read::ReadTool;
pub use repl_tool::ReplTool;
pub use web_fetch::WebFetchTool;
pub use web_search::WebSearchTool;
pub use write::WriteTool;

use crate::agent::tool::SharedTool;
use crate::agent::AgentLoop;
use std::sync::Arc;

pub fn default_tools_list() -> Vec<SharedTool> {
    vec![
        Arc::new(BashTool) as SharedTool,
        Arc::new(ReadTool) as SharedTool,
        Arc::new(WriteTool) as SharedTool,
        Arc::new(EditTool) as SharedTool,
        Arc::new(GrepTool) as SharedTool,
        Arc::new(LsTool) as SharedTool,
        Arc::new(FindTool) as SharedTool,
        Arc::new(WebSearchTool) as SharedTool,
        Arc::new(WebFetchTool) as SharedTool,
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
    // Avoid duplicate repl registration
    if agent.tools().get("repl").is_some() {
        return;
    }
    let _ = depth;
    repl_tool::attach_repl_tool(agent, max_depth);
}
