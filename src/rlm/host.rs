//! Host-side handlers for REPL-mediated llm_query / agent_query.

use crate::agent::control::AbortFlag;
use crate::agent::state::AgentState;
use crate::agent::tool::SharedTool;
use crate::agent::{AgentEvent, AgentLoop};
use crate::ai::provider::Provider;
use crate::ai::types::*;
use crate::rlm::tree::{CallKind, CallStatus, CallTree};
use crate::rlm::ReplSession;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

const SUMMARY_CAP: usize = 50_000;
const DEFAULT_CONCURRENCY: usize = 4;

/// Shared RLM host bridging the Python REPL to Rust provider / nested agents.
#[derive(Clone)]
pub struct RlmHost {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub provider_name: String,
    pub system_prompt: String,
    pub abort: AbortFlag,
    pub tree: CallTree,
    pub parent_node_id: String,
    pub depth: u32,
    pub max_depth: u32,
    pub max_iterations: usize,
    /// Tool factory for nested agent_query (names allowed).
    pub tool_factory: Arc<dyn Fn() -> Vec<SharedTool> + Send + Sync>,
}

impl RlmHost {
    pub async fn handle(
        &self,
        method: &str,
        args: Vec<Value>,
        kwargs: Value,
    ) -> Result<Value, String> {
        if self.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        match method {
            "llm_query" => {
                let prompt = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or("llm_query requires a prompt string")?;
                let text = self.llm_query(prompt).await?;
                Ok(Value::String(text))
            }
            "llm_query_batched" => {
                let prompts = args
                    .first()
                    .and_then(|v| v.as_array())
                    .ok_or("llm_query_batched requires a list")?;
                let prompt_strs: Vec<String> = prompts
                    .iter()
                    .map(|p| p.as_str().unwrap_or("").to_string())
                    .collect();
                let mut texts = Vec::new();
                for batch in prompt_strs.chunks(DEFAULT_CONCURRENCY) {
                    let mut futs = Vec::new();
                    for p in batch {
                        let prompt = p.clone();
                        let this = self.clone();
                        futs.push(async move { this.llm_query(&prompt).await });
                    }
                    let results = futures::future::join_all(futs).await;
                    for r in results {
                        texts.push(Value::String(r?));
                    }
                }
                Ok(Value::Array(texts))
            }
            "agent_query" => {
                let task = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or("agent_query requires a task string")?;
                let tools_filter = kwargs
                    .get("tools")
                    .and_then(|t| t.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    });
                let text = self.agent_query(task, tools_filter).await?;
                Ok(Value::String(text))
            }
            other => Err(format!("unknown host method: {}", other)),
        }
    }

    async fn llm_query(&self, prompt: &str) -> Result<String, String> {
        let node_id = self.tree.spawn(
            Some(&self.parent_node_id),
            CallKind::Llm,
            &prompt.chars().take(120).collect::<String>(),
        );
        let api_key = std::env::var(self.provider.api_key_env_var())
            .map_err(|_| format!("{} not set", self.provider.api_key_env_var()))?;

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: Role::User,
                content: vec![Content {
                    content_type: ContentType::Text,
                    text: Some(prompt.to_string()),
                    ..Default::default()
                }],
            }],
            system: Some(
                "You are a focused sub-LM. Answer the query directly and concisely. No tools."
                    .to_string(),
            ),
            tools: Vec::new(),
            max_tokens: 2048,
            temperature: Some(0.0),
            top_p: None,
            stop_sequences: None,
            stream: false,
            thinking: None,
        };

        let result = self
            .provider
            .chat(&api_key, request)
            .await
            .map_err(|e| format!("llm_query failed: {:?}", e))?;

        let text = result
            .content
            .iter()
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join("");
        let capped = if text.len() > SUMMARY_CAP {
            format!("{}…[truncated]", &text[..SUMMARY_CAP])
        } else {
            text
        };
        self.tree.finish(
            &node_id,
            CallStatus::Done,
            Some(capped.chars().take(200).collect()),
        );
        Ok(capped)
    }

    pub async fn agent_query(
        &self,
        task: &str,
        tools_filter: Option<Vec<String>>,
    ) -> Result<String, String> {
        if self.depth >= self.max_depth {
            return Err(format!(
                "max Deep Context depth {} reached; use a narrower task or llm_query instead",
                self.max_depth
            ));
        }
        let node_id = self.tree.spawn(
            Some(&self.parent_node_id),
            CallKind::Agent,
            &task.chars().take(120).collect::<String>(),
        );

        let state = AgentState::new(self.model.clone(), self.provider_name.clone())
            .with_system_prompt(self.system_prompt.clone());

        let mut agent = AgentLoop::new(self.provider.clone(), state)
            .with_max_iterations(self.max_iterations.min(40))
            .with_abort(self.abort.clone())
            .with_call_tree(self.tree.clone())
            .with_rlm_depth(self.depth + 1, self.max_depth);

        let tools = (self.tool_factory)();
        for tool in tools {
            let name = tool.name().to_string();
            if let Some(ref filter) = tools_filter {
                if !filter.iter().any(|f| f == &name) {
                    continue;
                }
            }
            // Nested agents beyond max_depth-1 cannot spawn further agent_query via REPL depth.
            agent.register_tool(tool);
        }
        // Always allow repl at child unless at max depth-1? Keep repl; agent_query depth-checked in host.
        crate::tools::register_rlm_tools(&mut agent, self.depth + 1, self.max_depth);

        let mut final_text = String::new();
        let run = agent
            .run(task, &mut |event| {
                if let AgentEvent::TextDelta { text } = event {
                    final_text.push_str(&text);
                }
            })
            .await;

        match run {
            Ok(()) => {
                // Prefer last assistant text from state if stream was empty.
                if final_text.trim().is_empty() {
                    if let Some(msg) = agent.state().messages.iter().rev().find(|m| {
                        m.role == Role::Assistant
                    }) {
                        for c in &msg.content {
                            if let Some(t) = &c.text {
                                final_text.push_str(t);
                            }
                        }
                    }
                }
                let capped = if final_text.len() > SUMMARY_CAP {
                    format!("{}…[truncated]", &final_text[..SUMMARY_CAP])
                } else {
                    final_text
                };
                self.tree.finish(
                    &node_id,
                    CallStatus::Done,
                    Some(capped.chars().take(200).collect()),
                );
                Ok(capped)
            }
            Err(e) => {
                self.tree
                    .finish(&node_id, CallStatus::Error, Some(e.clone()));
                Err(e)
            }
        }
    }
}

/// Shared REPL behind a mutex for the `repl` tool.
pub type SharedRepl = Arc<Mutex<Option<ReplSession>>>;

pub async fn ensure_repl(shared: &SharedRepl) -> Result<(), String> {
    let mut g = shared.lock().await;
    if g.is_none() {
        *g = Some(ReplSession::spawn().await?);
    }
    Ok(())
}
