pub mod bash;
pub mod edit;
pub mod find;
pub mod grep;
pub mod ls;
pub mod read;
pub mod web_fetch;
pub mod web_search;
pub mod write;

pub use bash::BashTool;
pub use edit::EditTool;
pub use find::FindTool;
pub use grep::GrepTool;
pub use ls::LsTool;
pub use read::ReadTool;
pub use web_fetch::WebFetchTool;
pub use web_search::WebSearchTool;
pub use write::WriteTool;

use crate::agent::tool::SharedTool;
use crate::agent::AgentLoop;
use std::sync::Arc;

pub fn register_default_tools(agent: &mut AgentLoop) {
    agent.register_tool(Arc::new(BashTool) as SharedTool);
    agent.register_tool(Arc::new(ReadTool) as SharedTool);
    agent.register_tool(Arc::new(WriteTool) as SharedTool);
    agent.register_tool(Arc::new(EditTool) as SharedTool);
    agent.register_tool(Arc::new(GrepTool) as SharedTool);
    agent.register_tool(Arc::new(LsTool) as SharedTool);
    agent.register_tool(Arc::new(FindTool) as SharedTool);
    agent.register_tool(Arc::new(WebSearchTool) as SharedTool);
    agent.register_tool(Arc::new(WebFetchTool) as SharedTool);
}
