//! Minimal MCP stdio client (JSON-RPC + Content-Length framing).

mod client;
mod tool;

pub use client::{McpClient, McpServerSpec};
pub use tool::register_mcp_servers;

use crate::config::McpConfig;
use crate::agent::AgentLoop;

/// Connect configured MCP servers and register their tools on the agent.
/// Returns human-readable status lines (ok / skipped / error).
pub async fn attach_mcp_from_config(agent: &mut AgentLoop, cfg: &McpConfig) -> Vec<String> {
    register_mcp_servers(agent, &cfg.servers).await
}
