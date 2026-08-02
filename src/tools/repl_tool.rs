use crate::agent::control::AbortFlag;
use crate::agent::tool::*;
use crate::agent::{AgentEvent, AgentLoop};
use crate::ai::provider::Provider;
use crate::rlm::host::{ensure_repl, RlmHost, SharedRepl};
use crate::rlm::tree::CallTree;
use async_trait::async_trait;
use crossbeam_channel as channel;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ReplArgs {
    pub code: String,
    pub context: Option<String>,
}

/// CodeAct-style REPL tool for RLM-style recursive decomposition.
pub struct ReplTool {
    repl: SharedRepl,
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
    event_sink: Option<channel::Sender<AgentEvent>>,
}

impl ReplTool {
    pub fn new(
        repl: SharedRepl,
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
        event_sink: Option<channel::Sender<AgentEvent>>,
    ) -> Self {
        Self {
            repl,
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
            event_sink,
        }
    }
}

#[async_trait]
impl AgentTool for ReplTool {
    fn name(&self) -> &str {
        "repl"
    }

    fn description(&self) -> &str {
        "Execute Python in a persistent Deep Context REPL. Large context lives in the `context` variable \
         (set via optional context arg or load_file/load_dir). Use llm_query(prompt) for leaf LM \
         calls and agent_query(task) for recursive coding sub-agents. Call FINAL(value) when done. \
         Stdout is truncated; prefer variables and sub-calls over dumping huge strings."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Python code to execute in the persistent REPL namespace"
                },
                "context": {
                    "type": "string",
                    "description": "Optional: set/replace the `context` variable before exec"
                }
            },
            "required": ["code"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: ReplArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        if self.abort.is_aborted() {
            return ToolExecuteResult::error("aborted");
        }

        if let Err(e) = ensure_repl(&self.repl).await {
            return ToolExecuteResult::error(e);
        }

        let host = RlmHost {
            provider: self.provider.clone(),
            model: self.model.clone(),
            provider_name: self.provider_name.clone(),
            system_prompt: self.system_prompt.clone(),
            abort: self.abort.clone(),
            tree: self.tree.clone(),
            parent_node_id: self.parent_node_id.clone(),
            depth: self.depth,
            max_depth: self.max_depth,
            max_iterations: self.max_iterations,
            tool_factory: Arc::new(|| crate::tools::default_tools_list()),
        };

        let mut guard = self.repl.lock().await;
        let session = match guard.as_mut() {
            Some(s) => s,
            None => return ToolExecuteResult::error("REPL not available"),
        };

        if let Some(ctx) = &parsed.context {
            if let Err(e) = session.set_context(ctx).await {
                return ToolExecuteResult::error(e);
            }
        }

        let node_id = self.tree.spawn(
            Some(&self.parent_node_id),
            crate::rlm::CallKind::Repl,
            &repl_task_label(&parsed.code),
        );

        let result = session
            .exec_with_host_and_output(
                &parsed.code,
                {
                    let host = host.clone();
                    move |method, args, kwargs| {
                        let host = host.clone();
                        async move { host.handle(&method, args, kwargs).await }
                    }
                },
                {
                    let sink = self.event_sink.clone();
                    move |stream, text| {
                        if let Some(tx) = &sink {
                            let _ = tx.send(AgentEvent::ReplOutput {
                                stream: stream.to_string(),
                                text: text.to_string(),
                            });
                        }
                    }
                },
            )
            .await;

        match result {
            Ok(r) => {
                self.tree.finish(
                    &node_id,
                    if r.ok {
                        crate::rlm::CallStatus::Done
                    } else {
                        crate::rlm::CallStatus::Error
                    },
                    r.final_value.as_ref().map(|v| v.to_string()),
                );
                let mut out = String::new();
                if !r.stdout.is_empty() {
                    out.push_str(&r.stdout);
                }
                if !r.stderr.is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("STDERR:\n");
                    out.push_str(&r.stderr);
                }
                if let Some(final_v) = r.final_value {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&format!("FINAL: {}", final_v));
                }
                if out.is_empty() {
                    out = if r.ok {
                        "(ok, no output)".to_string()
                    } else {
                        "(failed)".to_string()
                    };
                }
                if r.ok {
                    ToolExecuteResult::ok(out)
                } else {
                    ToolExecuteResult::error(out)
                }
            }
            Err(e) => {
                self.tree
                    .finish(&node_id, crate::rlm::CallStatus::Error, Some(e.clone()));
                ToolExecuteResult::error(e)
            }
        }
    }
}

/// Short one-line label for a REPL node (avoid dumping code into Call Tree).
fn repl_task_label(code: &str) -> String {
    let first = code
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("repl");
    // Prefer a hint about what the code does.
    let hint = if code.contains("llm_query") {
        "llm_query"
    } else if code.contains("agent_query") {
        "agent_query"
    } else if code.contains("FINAL") {
        "FINAL"
    } else if code.contains("load_file") || code.contains("load_dir") {
        "load"
    } else {
        first
    };
    hint.chars().take(40).collect()
}

/// Attach RLM `repl` tool to an agent loop (and ensure root tree node).
pub fn attach_repl_tool(agent: &mut AgentLoop, max_rlm_depth: u32) {
    let root_task = "session";
    let root_id = agent.call_tree().ensure_root(root_task);
    let repl: SharedRepl = Arc::new(tokio::sync::Mutex::new(None));
    let tool = ReplTool::new(
        repl,
        agent.provider(),
        agent.state().model.clone(),
        agent.state().provider.clone(),
        agent.state().system_prompt.clone(),
        agent.abort_flag(),
        agent.call_tree().clone(),
        agent.rlm_depth(),
        max_rlm_depth,
        100,
        root_id,
        agent.event_sink(),
    );
    agent.register_tool(Arc::new(tool) as SharedTool);
}
