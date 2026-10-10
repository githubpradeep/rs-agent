//! Host-side handlers for REPL-mediated llm_query / agent_query.

use crate::agent::control::AbortFlag;
use crate::agent::state::AgentState;
use crate::agent::tool::{AgentTool, SharedTool};
use crate::agent::{AgentEvent, AgentLoop};
use crate::ai::provider::Provider;
use crate::ai::types::*;
use crate::rlm::tree::{CallKind, CallStatus, CallTree};
use crate::rlm::ReplSession;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const SUMMARY_CAP: usize = 50_000;
const DEFAULT_CONCURRENCY: usize = 4;
/// Chars of sub-call prompt+result allowed inside one REPL turn.
const SUBCALL_CHAR_BUDGET: usize = 400_000;
/// Wall clock for one llm_query or agent_query. Long on purpose: a short
/// silence kill teaches the model to stop recursing.
const SUBCALL_TIMEOUT: Duration = Duration::from_secs(600);
const HOST_TEXT_CAP: usize = 120_000;

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
    /// Shared across host clones for one REPL execution.
    pub subcall_chars: Arc<AtomicUsize>,
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
                let mut pairs: Vec<(String, String)> = Vec::new();
                for (i, batch) in prompt_strs.chunks(DEFAULT_CONCURRENCY).enumerate() {
                    let mut futs = Vec::new();
                    for (j, p) in batch.iter().enumerate() {
                        let prompt = p.clone();
                        let this = self.clone();
                        let label = format!("batch{i}-{j}");
                        futs.push(async move {
                            let t = this.llm_query(&prompt).await?;
                            Ok::<_, String>((label, t))
                        });
                    }
                    let results = futures::future::join_all(futs).await;
                    for r in results {
                        let (label, text) = r?;
                        pairs.push((label, text.clone()));
                        texts.push(Value::String(text));
                    }
                }
                // Conductor JOIN-style compact merge — attach as trailing meta string
                // without breaking the array return shape Python callers expect.
                let joined = crate::orchestration::join_summaries(&pairs, 240, 4_000);
                if !joined.is_empty() {
                    texts.push(Value::String(format!("__joined__\n{joined}")));
                }
                Ok(Value::Array(texts))
            }
            "agent_query" => {
                let task = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or("agent_query requires a task string")?;
                let tools_filter = kwargs.get("tools").and_then(|t| t.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                });
                let text = self.agent_query(task, tools_filter).await?;
                Ok(Value::String(text))
            }
            "read_file" => {
                let path = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or("read_file requires a path")?;
                let offset = kwargs.get("offset").and_then(|v| v.as_u64());
                let limit = kwargs.get("limit").and_then(|v| v.as_u64());
                let mut body = serde_json::json!({"file_path": path});
                if let Some(o) = offset {
                    body["offset"] = serde_json::json!(o);
                }
                if let Some(l) = limit {
                    body["limit"] = serde_json::json!(l);
                }
                self.local_tool("read", body).await.map(Value::String)
            }
            "grep" => {
                let pattern = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or("grep requires a pattern")?;
                let path = kwargs.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let mut body = serde_json::json!({"pattern": pattern, "path": path});
                if let Some(include) = kwargs.get("include").and_then(|v| v.as_str()) {
                    body["include"] = serde_json::json!(include);
                }
                self.local_tool("grep", body).await.map(Value::String)
            }
            "bash" => {
                let command = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or("bash requires a command")?;
                self.local_tool(
                    "bash",
                    serde_json::json!({"command": command, "timeout": 180000}),
                )
                .await
                .map(Value::String)
            }
            other => Err(format!("unknown host method: {}", other)),
        }
    }

    async fn llm_query(&self, prompt: &str) -> Result<String, String> {
        self.charge(0)?;
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

        let result = tokio::time::timeout(
            SUBCALL_TIMEOUT,
            self.provider.chat(&api_key, request),
        )
        .await
        .map_err(|_| {
            "llm_query exceeded 10 minutes. Return FINAL from what you have; do not keep waiting."
                .to_string()
        })?
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
        self.charge(prompt.len() + capped.len())?;
        Ok(capped)
    }

    pub async fn agent_query(
        &self,
        task: &str,
        tools_filter: Option<Vec<String>>,
    ) -> Result<String, String> {
        self.charge(0)?;
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
            .with_rlm_depth(self.depth + 1, self.max_depth)
            .with_exit_gates(false);

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

        let brief = crate::agent::ancestor::brief_for(&self.tree, &self.parent_node_id);
        let tasked = if brief.is_empty() {
            task.to_string()
        } else {
            format!(
                "{brief}\n\n<task>\n{task}\n</task>\nWork only the task. The parent transcript is not included."
            )
        };

        let mut final_text = String::new();
        let run = match tokio::time::timeout(SUBCALL_TIMEOUT, async {
            agent
                .run(&tasked, &mut |event| {
                    if let AgentEvent::TextDelta { text } = event {
                        final_text.push_str(&text);
                    }
                })
                .await
        })
        .await
        {
            Ok(inner) => inner,
            Err(_) => {
                let msg =
                    "agent_query exceeded 10 minutes. Return FINAL from what you have.".to_string();
                self.tree
                    .finish(&node_id, CallStatus::Error, Some(msg.clone()));
                return Err(msg);
            }
        };

        match run {
            Ok(()) => {
                // Prefer last assistant text from state if stream was empty.
                if final_text.trim().is_empty() {
                    if let Some(msg) = agent
                        .state()
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == Role::Assistant)
                    {
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
                self.charge(task.len() + capped.len())?;
                Ok(capped)
            }
            Err(e) => {
                self.tree
                    .finish(&node_id, CallStatus::Error, Some(e.clone()));
                Err(e)
            }
        }
    }

    fn charge(&self, add: usize) -> Result<(), String> {
        let prev = self.subcall_chars.fetch_add(add, Ordering::Relaxed);
        if prev >= SUBCALL_CHAR_BUDGET {
            return Err(format!(
                "sub-call budget exhausted ({SUBCALL_CHAR_BUDGET} chars). Return FINAL from what you already have."
            ));
        }
        Ok(())
    }

    async fn local_tool(&self, name: &str, args: Value) -> Result<String, String> {
        if self.abort.is_aborted() {
            return Err("aborted".into());
        }
        let result = match name {
            "read" => crate::tools::ReadTool.execute("repl", args).await,
            "grep" => crate::tools::GrepTool.execute("repl", args).await,
            "bash" => {
                crate::tools::BashTool::new(self.abort.clone())
                    .execute("repl", args)
                    .await
            }
            other => return Err(format!("unknown local tool {other}")),
        };
        let text = cap_host_text(result.content);
        if result.is_error {
            Err(text)
        } else {
            Ok(text)
        }
    }
}

/// Shared REPL behind a mutex for the `repl` tool.
pub type SharedRepl = Arc<Mutex<Option<ReplSession>>>;

fn cap_host_text(s: String) -> String {
    let n = s.chars().count();
    if n <= HOST_TEXT_CAP {
        s
    } else {
        let head: String = s.chars().take(HOST_TEXT_CAP).collect();
        format!("{head}\n…[truncated {n} chars; narrow the slice]")
    }
}

pub async fn ensure_repl(shared: &SharedRepl) -> Result<(), String> {
    let mut g = shared.lock().await;
    if g.is_none() {
        *g = Some(ReplSession::spawn().await?);
    }
    Ok(())
}
