//! Wrap MCP tools as `AgentTool`s and register them on an `AgentLoop`.

use crate::agent::tool::{AgentTool, ToolExecuteResult};
use crate::agent::AgentLoop;
use crate::config::McpServerConfig;
use crate::mcp::client::{McpClient, McpServerSpec, McpToolInfo};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

struct McpAgentTool {
    /// Exposed name: `mcp__{server}__{tool}`
    exposed_name: String,
    remote_name: String,
    description: String,
    input_schema: Value,
    read_only: bool,
    client: Arc<Mutex<McpClient>>,
}

#[async_trait]
impl AgentTool for McpAgentTool {
    fn name(&self) -> &str {
        &self.exposed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        if self.input_schema.is_null() {
            serde_json::json!({"type": "object", "properties": {}})
        } else {
            self.input_schema.clone()
        }
    }

    fn requires_permission(&self) -> bool {
        !self.read_only
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let client = self.client.lock().await;
        match client.call_tool(&self.remote_name, args).await {
            Ok((content, is_error)) => {
                if is_error {
                    ToolExecuteResult::error(content)
                } else {
                    ToolExecuteResult::ok(content)
                }
            }
            Err(e) => ToolExecuteResult::error(e),
        }
    }
}

fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Connect each enabled server, list tools, register as `mcp__server__tool`.
pub async fn register_mcp_servers(
    agent: &mut AgentLoop,
    servers: &[McpServerConfig],
) -> Vec<String> {
    let mut status = Vec::new();
    for cfg in servers {
        if cfg.enabled == Some(false) {
            status.push(format!("MCP `{}`: disabled", cfg.name));
            continue;
        }
        if cfg.command.trim().is_empty() {
            status.push(format!("MCP `{}`: missing command", cfg.name));
            continue;
        }
        let spec = McpServerSpec {
            name: cfg.name.clone(),
            command: cfg.command.clone(),
            args: cfg.args.clone(),
            env: cfg.env.clone(),
        };
        match McpClient::start(&spec).await {
            Ok(client) => {
                let tools = match client.list_tools().await {
                    Ok(t) => t,
                    Err(e) => {
                        status.push(format!("MCP `{}`: tools/list failed: {e}", cfg.name));
                        continue;
                    }
                };
                let client = Arc::new(Mutex::new(client));
                let server_slug = sanitize_name(&cfg.name);
                let mut count = 0usize;
                for info in tools {
                    register_one(agent, &server_slug, &info, client.clone());
                    count += 1;
                }
                status.push(format!("MCP `{}`: connected, {count} tool(s)", cfg.name));
            }
            Err(e) => status.push(format!("MCP `{}`: {e}", cfg.name)),
        }
    }
    status
}

fn register_one(
    agent: &mut AgentLoop,
    server_slug: &str,
    info: &McpToolInfo,
    client: Arc<Mutex<McpClient>>,
) {
    let remote = info.name.clone();
    let exposed = format!("mcp__{}__{}", server_slug, sanitize_name(&remote));
    let read_only = info
        .annotations
        .as_ref()
        .and_then(|a| a.read_only_hint)
        .unwrap_or(false);
    let desc = if info.description.is_empty() {
        format!("MCP tool `{remote}` from server `{server_slug}`")
    } else {
        format!("[MCP:{server_slug}] {}", info.description)
    };
    let tool = McpAgentTool {
        exposed_name: exposed,
        remote_name: remote,
        description: desc,
        input_schema: info.input_schema.clone(),
        read_only,
        client,
    };
    agent.register_tool(Arc::new(tool));
}

/// Convenience for empty env maps in tests/helpers.
#[allow(dead_code)]
pub fn empty_env() -> HashMap<String, String> {
    HashMap::new()
}
