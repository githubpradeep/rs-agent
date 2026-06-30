pub mod bash;
pub mod read;
pub mod write;
pub mod edit;
pub mod grep;
pub mod ls;

pub use bash::BashTool;
pub use read::ReadTool;
pub use write::WriteTool;
pub use edit::EditTool;
pub use grep::GrepTool;
pub use ls::LsTool;

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
}
