use crate::agent::registry::ToolRegistry;
use crate::agent::state::AgentState;
use crate::agent::tool::{ToolExecutionMode, ToolExecuteResult};
use crate::ai::provider::Provider;
use crate::ai::token_count;
use crate::ai::types::*;
use crate::permission::{PendingPermission, PermissionReply};
use crossbeam_channel as channel;
use futures::StreamExt;
use std::sync::Arc;
use tracing;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    ToolUseStart { id: String, name: String },
    ToolUseDelta { input: String },
    ToolResult { id: String, name: String, result: ToolExecuteResult },
    TurnEnd { stop_reason: Option<StopReason> },
    Error { message: String },
    Done,
    ContextWarning { fraction: f64, used: usize, limit: usize },
    TokenUpdate { used: usize, limit: usize },
    Compacting,
    Compacted { summary: String },
}

pub struct AgentLoop {
    provider: Arc<dyn Provider>,
    tools: ToolRegistry,
    state: AgentState,
    max_iterations: usize,
    permission_tx: Option<channel::Sender<PendingPermission>>,
    compacted_up_to: usize,
    overflow_retried: bool,
}

impl AgentLoop {
    pub fn new(provider: Arc<dyn Provider>, state: AgentState) -> Self {
        Self {
            provider,
            tools: ToolRegistry::new(),
            state,
            max_iterations: 25,
            permission_tx: None,
            compacted_up_to: 0,
            overflow_retried: false,
        }
    }

    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    pub fn state(&self) -> &AgentState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut AgentState {
        &mut self.state
    }

    pub fn register_tool(&mut self, tool: crate::agent::tool::SharedTool) {
        self.tools.register(tool);
    }

    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    pub fn set_permission_channel(&mut self, tx: channel::Sender<PendingPermission>) {
        self.permission_tx = Some(tx);
    }

    pub async fn run(
        &mut self,
        user_message: &str,
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        let user_msg = Message {
            role: Role::User,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some(user_message.to_string()),
                id: None,
                name: None,
                input: None,
                tool_use_id: None,
                content: None,
                signature: None,
                thinking: None,
            }],
        };
        self.state.add_message(user_msg);

        self.run_loop(callback).await
    }

    pub async fn run_with_followups(
        &mut self,
        user_message: &str,
        follow_up_messages: Vec<String>,
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        let user_msg = Message {
            role: Role::User,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some(user_message.to_string()),
                id: None,
                name: None,
                input: None,
                tool_use_id: None,
                content: None,
                signature: None,
                thinking: None,
            }],
        };
        self.state.add_message(user_msg);

        self.run_loop(callback).await?;

        for msg in follow_up_messages {
            let follow_up = Message {
                role: Role::User,
                content: vec![Content {
                    content_type: ContentType::Text,
                    text: Some(msg),
                    id: None,
                    name: None,
                    input: None,
                    tool_use_id: None,
                    content: None,
                    signature: None,
                    thinking: None,
                }],
            };
            self.state.add_message(follow_up);
            self.run_loop(callback).await?;
        }

        Ok(())
    }

    fn is_context_overflow(err: &str) -> bool {
        let lower = err.to_lowercase();
        lower.contains("context_length_exceeded")
            || (lower.contains("context length") && (lower.contains("exceed") || lower.contains("too long")))
            || lower.contains("maximum context")
            || lower.contains("prompt is too long")
            || lower.contains("too many tokens")
            || lower.contains("request too large")
    }

    async fn run_loop(
        &mut self,
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        self.overflow_retried = false;
        let tool_defs_json = serde_json::to_string(&self.tools.tool_defs()).unwrap_or_default();

        for _ in 0..self.max_iterations {
            let used = self.state.estimated_context_tokens(&tool_defs_json);
            let limit = self.state.context_limit();
            let fraction = used as f64 / limit as f64;

            if fraction >= 0.95 {
                callback(AgentEvent::Error {
                    message: format!(
                        "Context limit approaching ({}/{} tokens, {:.0}%). Please use a new session.",
                        used, limit, fraction * 100.0
                    ),
                });
                return Err("Context limit exceeded".to_string());
            }

            if fraction >= 0.65 {
                callback(AgentEvent::ContextWarning { fraction, used, limit });
                let _ = self.compact(callback).await;
            }

            let assistant_result = self.stream_assistant(callback).await;
            let assistant_msg = match assistant_result {
                Ok(msg) => msg,
                Err(e) if !self.overflow_retried && Self::is_context_overflow(&e) => {
                    callback(AgentEvent::Error {
                        message: format!("Context overflow detected, compacting and retrying..."),
                    });
                    self.overflow_retried = true;
                    let _ = self.compact(callback).await;
                    continue;
                }
                Err(e) => return Err(e),
            };

            let used = self.state.estimated_context_tokens(&tool_defs_json);
            callback(AgentEvent::TokenUpdate { used, limit });

            let tool_calls: Vec<Content> = assistant_msg
                .content
                .iter()
                .filter(|c| c.content_type == ContentType::ToolUse)
                .cloned()
                .collect();

            if tool_calls.is_empty() {
                self.state.add_assistant(&assistant_msg);
                callback(AgentEvent::Done);
                return Ok(());
            }

            self.state.add_assistant(&assistant_msg);

            let has_sequential = self.tools.iter().any(|t| {
                t.execution_mode() == ToolExecutionMode::Sequential
            });

            if has_sequential {
                self.execute_tools_sequential(&tool_calls, callback).await?;
            } else {
                self.execute_tools_parallel(&tool_calls, callback).await?;
            }
        }

        callback(AgentEvent::Error { message: format!("Reached max iterations ({})", self.max_iterations) });
        Err(format!("Reached max iterations ({})", self.max_iterations))
    }

    async fn stream_assistant(
        &self,
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<AssistantMessage, String> {
        let api_key = std::env::var(self.provider.api_key_env_var())
            .map_err(|_| format!("{} not set", self.provider.api_key_env_var()))?;

        let request = ChatRequest {
            model: self.state.model.clone(),
            messages: self.state.messages.clone(),
            system: if self.state.system_prompt.is_empty() {
                None
            } else {
                Some(self.state.system_prompt.clone())
            },
            tools: self.tools.tool_defs(),
            max_tokens: self.provider.default_max_tokens(),
            temperature: Some(0.0),
            top_p: None,
            stop_sequences: None,
            stream: true,
            thinking: self.state.thinking_budget.map(|b| ThinkingConfig {
                r#type: "enabled".to_string(),
                budget_tokens: b,
            }),
        };

        let mut stream = self
            .provider
            .chat_stream(&api_key, request)
            .await
            .map_err(|e| format!("stream error: {:?}", e))?;

        let mut content_blocks: Vec<Option<Content>> = Vec::new();
        let mut tool_arg_buf: Vec<String> = Vec::new();
        let usage: Option<Usage> = None;
        let mut stop_reason: Option<StopReason> = None;
        let model = String::new();
        let msg_id: Option<String> = None;
        while let Some(result) = stream.next().await {
            match result {
                Ok(delta) => {
                    let idx = delta.content_index as usize;
                    match delta.r#type {
                        DeltaType::Text { text } => {
                            if content_blocks.len() <= idx { content_blocks.resize(idx + 1, None); }
                            callback(AgentEvent::TextDelta { text: text.clone() });
                            if let Some(Some(b)) = content_blocks.get_mut(idx) {
                                if b.content_type == ContentType::Text {
                                    let existing = b.text.take().unwrap_or_default();
                                    b.text = Some(existing + &text);
                                }
                            } else {
                                content_blocks[idx] = Some(Content {
                                    content_type: ContentType::Text,
                                    text: Some(text), ..Default::default()
                                });
                            }
                        }
                        DeltaType::Thinking { thinking } => {
                            if content_blocks.len() <= idx { content_blocks.resize(idx + 1, None); }
                            callback(AgentEvent::ThinkingDelta { thinking: thinking.clone() });
                            if let Some(Some(b)) = content_blocks.get_mut(idx) {
                                if b.content_type == ContentType::Thinking {
                                    let existing = b.thinking.take().unwrap_or_default();
                                    b.thinking = Some(existing + &thinking);
                                }
                            } else {
                                content_blocks[idx] = Some(Content {
                                    content_type: ContentType::Thinking,
                                    thinking: Some(thinking), ..Default::default()
                                });
                            }
                        }
                        DeltaType::Signature { signature } => {
                            if let Some(Some(b)) = content_blocks.get_mut(idx) {
                                b.signature = Some(signature);
                            }
                        }
                        DeltaType::ToolCallStart { id, name, input } => {
                            if content_blocks.len() <= idx { content_blocks.resize(idx + 1, None); }
                            if tool_arg_buf.len() <= idx { tool_arg_buf.resize(idx + 1, String::new()); }
                            callback(AgentEvent::ToolUseStart { id: id.clone(), name: name.clone() });
                            content_blocks[idx] = Some(Content {
                                content_type: ContentType::ToolUse,
                                id: Some(id),
                                name: Some(name),
                                input: None,
                                ..Default::default()
                            });
                            tool_arg_buf[idx] = input;
                        }
                        DeltaType::ToolCallDelta { input } => {
                            callback(AgentEvent::ToolUseDelta { input: input.clone() });
                            if tool_arg_buf.len() <= idx {
                                tool_arg_buf.resize(idx + 1, String::new());
                                tool_arg_buf[idx] = input;
                            } else {
                                tool_arg_buf[idx].push_str(&input);
                            }
                        }
                        DeltaType::Stop { stop_reason: reason } => {
                            stop_reason = reason;
                        }
                    }
                }
                Err(e) => {
                    callback(AgentEvent::Error { message: format!("stream error: {:?}", e) });
                    return Err(format!("stream error: {:?}", e));
                }
            }
        }

        for (i, block) in content_blocks.iter_mut().enumerate() {
            if let Some(b) = block {
                if b.content_type == ContentType::ToolUse {
                    if let Some(raw) = tool_arg_buf.get(i) {
                        b.input = Some(serde_json::from_str(raw).unwrap_or(serde_json::Value::Object(serde_json::Map::new())));
                    }
                }
            }
        }

        let mut expanded: Vec<Content> = Vec::new();
        for block in content_blocks.into_iter().flatten() {
            if block.content_type == ContentType::Text {
                if let Some(ref text) = block.text {
                    if let Some(start) = text.find("{\"name\"") {
                        let prefix = &text[..start];
                        if !prefix.is_empty() {
                            expanded.push(Content {
                                content_type: ContentType::Text,
                                text: Some(prefix.to_string()),
                                ..Default::default()
                            });
                        }
                        let after = &text[start..];
                        let mut depth = 0i32;
                        let mut end = after.len();
                        for (j, ch) in after.char_indices() {
                            if ch == '{' { depth += 1; }
                            else if ch == '}' {
                                depth -= 1;
                                if depth == 0 {
                                    end = j + 1;
                                    break;
                                }
                            }
                        }
                        if depth == 0 {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&after[..end]) {
                                if let Some(name) = value["name"].as_str() {
                                    if value["arguments"].is_object() {
                                        callback(AgentEvent::ToolUseStart { id: format!("call_{}", expanded.len()), name: name.to_string() });
                                        expanded.push(Content {
                                            content_type: ContentType::ToolUse,
                                            id: Some(format!("call_{}", expanded.len())),
                                            name: Some(name.to_string()),
                                            input: Some(value["arguments"].clone()),
                                            ..Default::default()
                                        });
                                        let suffix = &after[end..];
                                        if !suffix.is_empty() {
                                            expanded.push(Content {
                                                content_type: ContentType::Text,
                                                text: Some(suffix.to_string()),
                                                ..Default::default()
                                            });
                                        }
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            expanded.push(block);
        }

        Ok(AssistantMessage {
            content: expanded,
            stop_reason,
            usage,
            model,
            id: msg_id,
        })
    }

    async fn execute_tools_sequential(
        &mut self,
        tool_calls: &[Content],
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        for tc in tool_calls {
            let id = tc.id.as_deref().unwrap_or("");
            let name = tc.name.as_deref().unwrap_or("");
            let input = tc.input.clone().unwrap_or(serde_json::Value::Null);

            let result = self.execute_single_tool(id, name, &input).await;
            callback(AgentEvent::ToolResult {
                id: id.to_string(),
                name: name.to_string(),
                result: result.clone(),
            });
            self.state.add_tool_result(id.to_string(), name.to_string(), result.content.clone(), result.is_error);

            if result.terminate {
                break;
            }
        }
        Ok(())
    }

    async fn execute_tools_parallel(
        &mut self,
        tool_calls: &[Content],
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        struct ToolJob {
            id: String,
            name: String,
            input: serde_json::Value,
        }
        let tool_data: Vec<ToolJob> = tool_calls.iter().map(|tc| ToolJob {
            id: tc.id.as_deref().unwrap_or("").to_string(),
            name: tc.name.as_deref().unwrap_or("").to_string(),
            input: tc.input.clone().unwrap_or(serde_json::Value::Null),
        }).collect();

        let mut futures = Vec::new();
        for job in tool_data {
            let result = self.execute_single_tool(&job.id, &job.name, &job.input).await;
            futures.push((job, result));
        }

        for (job, result) in &futures {
            callback(AgentEvent::ToolResult {
                id: job.id.clone(),
                name: job.name.clone(),
                result: result.clone(),
            });
            self.state.add_tool_result(job.id.clone(), job.name.clone(), result.content.clone(), result.is_error);
        }

        for (_, result) in futures {
            if result.terminate {
                break;
            }
        }

        Ok(())
    }

    async fn compact(
        &mut self,
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        const KEEP_BUDGET: usize = 20_000;
        const TRUNCATE_LEN: usize = 2000;

        let total = self.state.messages.len();
        if total <= self.compacted_up_to + 2 {
            return Ok(());
        }

        // Walk backwards from newest, find split point by token budget
        let mut accumulated = 0usize;
        let mut split = total;
        for i in (0..total).rev() {
            let t = token_count::estimate_message(&self.state.messages[i]);
            if accumulated + t > KEEP_BUDGET && accumulated > 0 {
                break;
            }
            accumulated += t;
            split = i;
        }

        // Adjust split to nearest user message boundary (turn boundary)
        for i in split..total {
            if self.state.messages[i].role == Role::User {
                split = i;
                break;
            }
        }

        if split <= self.compacted_up_to {
            return Ok(());
        }

        let to_summarize: Vec<Message> = self.state.messages[..split].to_vec();
        let keep_msgs: Vec<Message> = self.state.messages[split..].to_vec();

        // Extract previous compaction summary for incremental update
        let previous_summary = to_summarize.iter().find_map(|m| {
            if m.role == Role::System {
                m.content.iter().find_map(|c| {
                    c.text.as_deref().and_then(|t| {
                        t.strip_prefix("[Compacted summary of earlier conversation]\n")
                    })
                })
            } else {
                None
            }
        });

        // Serialize conversation for summarization with truncation
        let mut conv_text = String::new();
        for msg in &to_summarize {
            if msg.role == Role::System && msg.content.iter().any(|c| {
                c.text.as_deref().map_or(false, |t| {
                    t.starts_with("[Compacted summary of earlier conversation]")
                })
            }) {
                continue;
            }
            let role = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool",
                Role::System => "System",
            };
            for c in &msg.content {
                let text = c.text.as_deref().unwrap_or("");
                let truncated = if text.len() > TRUNCATE_LEN {
                    format!("{}... [truncated {} chars]", &text[..TRUNCATE_LEN], text.len())
                } else {
                    text.to_string()
                };
                if !truncated.is_empty() {
                    conv_text.push_str(&format!("[{}] {}\n\n", role, truncated));
                }
            }
        }

        if conv_text.trim().is_empty() {
            return Ok(());
        }

        callback(AgentEvent::Compacting);

        let api_key = std::env::var(self.provider.api_key_env_var())
            .map_err(|_| format!("{} not set", self.provider.api_key_env_var()))?;

        let user_msg = if let Some(prev) = previous_summary {
            format!(
                "Update the anchored summary below with the new conversation. \
                 Preserve still-true details, remove stale details, and merge in new facts.\n\n\
                 <previous-summary>\n{prev}\n</previous-summary>\n\n\
                 <new-conversation>\n{conv_text}\n</new-conversation>"
            )
        } else {
            format!(
                "Summarize the following conversation. Use this exact structure:\n\
                 ## Goal\n...\n\
                 ## Constraints & Preferences\n...\n\
                 ## Progress\n\
                 ### Done\n...\n\
                 ### In Progress\n...\n\
                 ### Blocked\n...\n\
                 ## Key Decisions\n...\n\
                 ## Next Steps\n...\n\
                 ## Critical Context\n...\n\
                 ## Relevant Files\n...\n\n\
                 <conversation>\n{conv_text}\n</conversation>"
            )
        };

        let system = "You are a conversation summarizer. \
                      Do NOT continue the conversation or respond to questions. \
                      ONLY output the structured summary with the requested sections. \
                      Be concise and factual. Use third person past tense.";

        let request = ChatRequest {
            model: self.state.model.clone(),
            messages: vec![Message {
                role: Role::User,
                content: vec![Content {
                    content_type: ContentType::Text,
                    text: Some(user_msg),
                    ..Default::default()
                }],
            }],
            system: Some(system.to_string()),
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
            .map_err(|e| format!("Compaction failed: {:?}", e))?;

        let summary = result
            .content
            .first()
            .and_then(|c| c.text.as_deref())
            .unwrap_or("")
            .to_string();

        let summary_msg = Message {
            role: Role::System,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some(format!(
                    "[Compacted summary of earlier conversation]\n{}",
                    summary
                )),
                ..Default::default()
            }],
        };

        self.state.messages.clear();
        self.state.messages.push(summary_msg);
        self.state.messages.extend(keep_msgs);
        self.compacted_up_to = 1;

        callback(AgentEvent::Compacted { summary });
        Ok(())
    }

    async fn execute_single_tool(
        &self,
        tool_call_id: &str,
        name: &str,
        input: &serde_json::Value,
    ) -> ToolExecuteResult {
        match self.tools.get(name) {
            Some(tool) => {
                if let Some(ref tx) = self.permission_tx {
                    if tool.requires_permission() {
                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                        let _ = tx.send(PendingPermission {
                            request: crate::permission::PermissionRequest {
                                tool_name: name.to_string(),
                                tool_input: input.to_string(),
                            },
                            reply_tx,
                        });
                        match reply_rx.await {
                            Ok(PermissionReply::Allow) => {}
                            Ok(PermissionReply::Deny) => {
                                return ToolExecuteResult::error("Permission denied by user");
                            }
                            Err(_) => {
                                return ToolExecuteResult::error("Permission prompt cancelled");
                            }
                        }
                    }
                }
                tracing::info!(tool = name, "executing tool");
                tool.execute(tool_call_id, input.clone()).await
            }
            None => ToolExecuteResult::error(format!("Unknown tool: {}", name)),
        }
    }
}


