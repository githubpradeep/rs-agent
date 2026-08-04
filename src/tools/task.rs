//! Top-level `task` tool — spawn a nested sub-agent (agent_query without REPL).

use crate::agent::control::AbortFlag;
use crate::agent::tool::*;
use crate::agent::AgentLoop;
use crate::ai::provider::Provider;
use crate::rlm::host::RlmHost;
use crate::rlm::tree::CallTree;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize)]
struct TaskArgs {
    /// Task / prompt for the sub-agent.
    #[serde(alias = "prompt", alias = "description", alias = "query")]
    task: String,
    /// Optional tool allow-list (e.g. ["read","grep","ls"]). Omit = all default tools.
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// Optional role profile: plan | implement | verify.
    #[serde(default)]
    profile: Option<String>,
}

fn profile_defaults(profile: &str) -> (String, Option<Vec<String>>) {
    match profile.trim().to_lowercase().as_str() {
        "plan" => (
            "PROFILE: plan. Read-only research and design. Do not edit files or run mutating shell. \
             Return a clear plan and success criteria."
                .into(),
            Some(vec![
                "read".into(),
                "grep".into(),
                "ls".into(),
                "find".into(),
                "webfetch".into(),
                "websearch".into(),
                "bead".into(),
            ]),
        ),
        "verify" => (
            "PROFILE: verify. Prove the task criteria with tools. Final line must start with \
             VERIFIED or NOT_VERIFIED."
                .into(),
            Some(vec![
                "bash".into(),
                "read".into(),
                "grep".into(),
                "ls".into(),
                "find".into(),
                "bead".into(),
            ]),
        ),
        "implement" => (
            "PROFILE: implement. Execute the assigned work. Prefer small diffs and run checks."
                .into(),
            None,
        ),
        _ => (String::new(), None),
    }
}

/// Spawns a nested AgentLoop (same path as REPL `agent_query`).
pub struct TaskTool {
    provider: Arc<dyn Provider>,
    model: String,
    provider_name: String,
    system_prompt: String,
    abort: AbortFlag,
    tree: CallTree,
    depth: u32,
    max_depth: u32,
    max_iterations: usize,
    parent_node_id: String,
}

impl TaskTool {
    pub fn new(
        provider: Arc<dyn Provider>,
        model: String,
        provider_name: String,
        system_prompt: String,
        abort: AbortFlag,
        tree: CallTree,
        depth: u32,
        max_depth: u32,
        max_iterations: usize,
        parent_node_id: String,
    ) -> Self {
        Self {
            provider,
            model,
            provider_name,
            system_prompt,
            abort,
            tree,
            depth,
            max_depth,
            max_iterations,
            parent_node_id,
        }
    }
}

#[async_trait]
impl AgentTool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Spawn a nested sub-agent for a focused subtask. The child has the same tools \
         (read/edit/bash/…) and returns a summary to you — use for parallelizable research, \
         isolated fixes, or long explorations that would clutter this transcript. \
         Optional tools=[...] restricts the child. Optional profile=plan|implement|verify \
         sets role defaults. Prefer this over stuffing huge context here; \
         for corpora already in the REPL, use repl + agent_query instead."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Clear task for the sub-agent (what to do + success criteria)"
                },
                "tools": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional allow-list of tool names for the child"
                },
                "profile": {
                    "type": "string",
                    "description": "Optional: plan | implement | verify"
                }
            },
            "required": ["task"]
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: TaskArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid task args: {e}. Expected {{task: \"...\", tools?: [...]}}."
                ))
            }
        };
        let task = parsed.task.trim().to_string();
        if task.is_empty() {
            return ToolExecuteResult::error("task must not be empty");
        }
        if self.abort.is_aborted() {
            return ToolExecuteResult::error("aborted");
        }

        let (profile_note, profile_tools) = parsed
            .profile
            .as_deref()
            .map(profile_defaults)
            .unwrap_or_else(|| (String::new(), None));
        let tools = parsed.tools.or(profile_tools);
        let mut system = self.system_prompt.clone();
        if !profile_note.is_empty() {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(&profile_note);
        }

        let host = RlmHost {
            provider: self.provider.clone(),
            model: self.model.clone(),
            provider_name: self.provider_name.clone(),
            system_prompt: system,
            abort: self.abort.clone(),
            tree: self.tree.clone(),
            parent_node_id: self.parent_node_id.clone(),
            depth: self.depth,
            max_depth: self.max_depth,
            max_iterations: self.max_iterations,
            tool_factory: Arc::new(|| crate::tools::default_tools_list()),
        };

        match host.agent_query(&task, tools).await {
            Ok(text) => ToolExecuteResult::ok(format!("Sub-agent result:\n{text}")),
            Err(e) => ToolExecuteResult::error(format!("Sub-agent failed: {e}")),
        }
    }
}

/// Attach top-level `task` tool (nested agent_query) to an agent loop.
pub fn attach_task_tool(agent: &mut AgentLoop, max_rlm_depth: u32) {
    if agent.tools().get("task").is_some() {
        return;
    }
    let root_id = agent.call_tree().ensure_root("session");
    let tool = TaskTool::new(
        agent.provider(),
        agent.state().model.clone(),
        agent.state().provider.clone(),
        agent.state().system_prompt.clone(),
        agent.abort_flag(),
        agent.call_tree().clone(),
        agent.rlm_depth(),
        max_rlm_depth,
        40,
        root_id,
    );
    agent.register_tool(Arc::new(tool) as SharedTool);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_task_aliases() {
        let v = json!({"prompt": "do thing", "tools": ["read"]});
        let a: TaskArgs = serde_json::from_value(v).unwrap();
        assert_eq!(a.task, "do thing");
        assert_eq!(a.tools.as_ref().unwrap()[0], "read");
    }
}
