use crate::agent::registry::ToolRegistry;
use crate::agent::state::AgentState;
use crate::agent::tool::{ToolExecutionMode, ToolExecuteResult};
use crate::ai::provider::Provider;
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
}

pub struct AgentLoop {
    provider: Arc<dyn Provider>,
    tools: ToolRegistry,
    state: AgentState,
    max_iterations: usize,
    permission_tx: Option<channel::Sender<PendingPermission>>,
}

impl AgentLoop {
    pub fn new(provider: Arc<dyn Provider>, state: AgentState) -> Self {
        Self {
            provider,
            tools: ToolRegistry::new(),
            state,
            max_iterations: 25,
            permission_tx: None,
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

    async fn run_loop(
        &mut self,
        callback: &mut dyn FnMut(AgentEvent),
    ) -> Result<(), String> {
        for _ in 0..self.max_iterations {
            let assistant_msg = self.stream_assistant(callback).await?;

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


