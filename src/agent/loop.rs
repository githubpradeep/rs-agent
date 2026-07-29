use crate::agent::control::{AbortFlag, SteerQueue};
use crate::agent::registry::ToolRegistry;
use crate::agent::state::AgentState;
use crate::agent::compact_pins::{
    append_pins_to_summary, collect_pins_from_messages, CompactPins,
};
use crate::agent::repair::{
    is_weak_model, make_arg_parse_error_value, prepare_tool_args, resolve_tool,
    tool_call_fingerprint, weak_model_system_note,
};
use crate::agent::rlm_escalate;
use crate::agent::tool::{ToolExecutionMode, ToolExecuteResult};
use crate::ai::provider::Provider;
use crate::ai::token_count;
use crate::ai::types::*;
use crate::permission::{PendingPermission, PermissionReply};
use crate::hooks::HookRegistry;
use crate::rlm::tree::CallTree;
use crossbeam_channel as channel;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tracing;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    ToolUseStart { id: String, name: String },
    ToolUseDelta { input: String },
    ToolResult { id: String, name: String, result: ToolExecuteResult },
    /// Streaming REPL stdout/stderr lines while `repl` runs.
    ReplOutput { stream: String, text: String },
    /// Progressive tool output (e.g. bash stdout) while a tool is running.
    ToolOutput {
        name: String,
        stream: String,
        text: String,
    },
    TurnEnd { stop_reason: Option<StopReason> },
    Error { message: String },
    Status { message: String },
    Done,
    Aborted,
    ContextWarning { fraction: f64, used: usize, limit: usize },
    TokenUpdate {
        used: usize,
        limit: usize,
        input_tokens: usize,
        output_tokens: usize,
    },
    Compacting,
    Compacted { summary: String },
    TreeUpdate { tree: CallTree },
    TitleUpdate { title: String },
    /// Session id/title changed (`/new`, `/fork`).
    SessionMeta { id: String, title: Option<String> },
    /// Replace the visible transcript (e.g. after fork-at-N).
    ReloadTranscript { messages: Vec<crate::ai::types::Message> },
    /// API-message timeline for `/timeline` / fork-at-N.
    TimelineSnapshot { entries: Vec<(usize, String)> },
    /// LSP diagnostics summary for the status bar.
    LspUpdate { summary: String },
}

/// Truncates a tool result to `max` chars by keeping the head and tail and
/// dropping the middle, so huge outputs (e.g. giant file dumps) don't blow
/// up the context window while still preserving the most useful parts
/// (the beginning, which usually has context/headers, and the end, which
/// usually has the final result/error).
///
/// Operates on chars (not bytes) so multi-byte UTF-8 sequences are never split.
pub fn truncate_tool_output(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }

    let truncated_count = chars.len() - max;
    let marker = format!("\n\n...[truncated {} chars]...\n\n", truncated_count);

    let half = max / 2;
    let head: String = chars[..half].iter().collect();
    let tail: String = chars[chars.len() - (max - half)..].iter().collect();

    format!("{}{}{}", head, marker, tail)
}

const MAX_TOOL_OUTPUT_CHARS: usize = 100_000;

pub struct AgentLoop {
    provider: Arc<dyn Provider>,
    tools: ToolRegistry,
    state: AgentState,
    max_iterations: usize,
    permission_tx: Option<channel::Sender<PendingPermission>>,
    compacted_up_to: usize,
    overflow_retried: bool,
    blank_retries: u8,
    abort: AbortFlag,
    steer: SteerQueue,
    call_tree: CallTree,
    rlm_depth: u32,
    max_rlm_depth: u32,
    /// Optional sink so tools (e.g. REPL) can emit live events mid-execution.
    event_sink: Option<channel::Sender<AgentEvent>>,
    /// Force sequential tool execution (also auto-on for weak models).
    force_sequential: bool,
    /// Recent tool fingerprints for doom-loop detection.
    recent_tool_fps: std::sync::Mutex<std::collections::VecDeque<String>>,
    /// Inject one-shot RLM escalate system note on the next model turn.
    rlm_escalate_hint_pending: std::sync::atomic::AtomicBool,
    rlm_escalate_hint_sent: std::sync::atomic::AtomicBool,
    /// Optional disk-loaded hooks (before_tool / after_tool / on_message).
    hooks: HookRegistry,
}

impl AgentLoop {
    pub fn new(provider: Arc<dyn Provider>, state: AgentState) -> Self {
        let force_sequential = is_weak_model(&state.model);
        Self {
            provider,
            tools: ToolRegistry::new(),
            state,
            max_iterations: 100,
            permission_tx: None,
            compacted_up_to: 0,
            overflow_retried: false,
            blank_retries: 0,
            abort: AbortFlag::new(),
            steer: SteerQueue::new(),
            call_tree: CallTree::new(),
            rlm_depth: 0,
            max_rlm_depth: 2,
            event_sink: None,
            force_sequential,
            recent_tool_fps: std::sync::Mutex::new(std::collections::VecDeque::with_capacity(4)),
            rlm_escalate_hint_pending: std::sync::atomic::AtomicBool::new(false),
            rlm_escalate_hint_sent: std::sync::atomic::AtomicBool::new(false),
            hooks: HookRegistry::load(),
        }
    }

    pub fn with_hooks(mut self, hooks: HookRegistry) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn hooks(&self) -> &HookRegistry {
        &self.hooks
    }

    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Force one-at-a-time tools (recommended for free/flash/mini models).
    pub fn with_force_sequential(mut self, force: bool) -> Self {
        self.force_sequential = force;
        self
    }

    fn should_force_sequential(&self) -> bool {
        self.force_sequential || is_weak_model(&self.state.model)
    }

    pub fn with_abort(mut self, abort: AbortFlag) -> Self {
        self.abort = abort;
        self
    }

    pub fn with_steer(mut self, steer: SteerQueue) -> Self {
        self.steer = steer;
        self
    }

    pub fn with_rlm_depth(mut self, depth: u32, max_depth: u32) -> Self {
        self.rlm_depth = depth;
        self.max_rlm_depth = max_depth;
        self
    }

    pub fn with_call_tree(mut self, tree: CallTree) -> Self {
        self.call_tree = tree;
        self
    }

    pub fn with_event_sink(mut self, sink: channel::Sender<AgentEvent>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    pub fn event_sink(&self) -> Option<channel::Sender<AgentEvent>> {
        self.event_sink.clone()
    }

    pub fn abort_flag(&self) -> AbortFlag {
        self.abort.clone()
    }

    pub fn steer_queue(&self) -> SteerQueue {
        self.steer.clone()
    }

    pub fn call_tree(&self) -> &CallTree {
        &self.call_tree
    }

    pub fn call_tree_mut(&mut self) -> &mut CallTree {
        &mut self.call_tree
    }

    pub fn rlm_depth(&self) -> u32 {
        self.rlm_depth
    }

    pub fn max_rlm_depth(&self) -> u32 {
        self.max_rlm_depth
    }

    pub fn provider(&self) -> Arc<dyn Provider> {
        self.provider.clone()
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

    pub fn unregister_tool(&mut self, name: &str) {
        self.tools.unregister(name);
    }

    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    pub fn set_permission_channel(&mut self, tx: channel::Sender<PendingPermission>) {
        self.permission_tx = Some(tx);
    }

    pub fn request_abort(&self) {
        self.abort.abort();
    }

    pub fn clear_abort(&self) {
        self.abort.clear();
    }

    pub fn enqueue_steer(&self, text: String) {
        self.steer.push(text);
    }

    pub fn set_model(&mut self, model: String) {
        self.state.model = model;
    }

    /// Swap the live provider client and model (pi-style mid-session switch).
    /// Re-attaches the RLM `repl` tool so sub-calls use the new client.
    pub fn set_provider_and_model(
        &mut self,
        provider: Arc<dyn Provider>,
        model: String,
    ) {
        let max_depth = self.max_rlm_depth;
        self.state.provider = provider.name().to_string();
        self.state.model = model;
        self.provider = provider;
        // Thinking budget: enable default when new provider supports it and none set.
        if self.state.thinking_budget.is_none() && self.provider.supports_thinking() {
            self.state.thinking_budget = Some(10_000);
        }
        if !self.provider.supports_thinking() {
            // Keep budget stored but loop only sends when supports_thinking —
            // no need to clear.
        }
        self.tools.unregister("repl");
        self.tools.unregister("task");
        crate::tools::register_rlm_tools(self, self.rlm_depth, max_depth);
    }

    pub fn clear_messages(&mut self) {
        self.state.messages.clear();
        self.state.clear_skill_tools();
        self.compacted_up_to = 0;
        self.call_tree = CallTree::new();
        self.rlm_escalate_hint_pending
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.rlm_escalate_hint_sent
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    pub async fn compact_now(
        &mut self,
        callback: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<(), String> {
        self.compact(callback).await
    }

    fn check_aborted(&self) -> Result<(), String> {
        if self.abort.is_aborted() {
            Err("aborted".to_string())
        } else {
            Ok(())
        }
    }

    fn inject_steer_messages(&mut self, callback: &mut (dyn FnMut(AgentEvent) + Send)) {
        for text in self.steer.drain() {
            callback(AgentEvent::Status {
                message: format!("steering: {}", text.chars().take(80).collect::<String>()),
            });
            self.state.add_message(Message {
                role: Role::User,
                content: vec![Content {
                    content_type: ContentType::Text,
                    text: Some(format!("[steer] {}", text)),
                    ..Default::default()
                }],
            });
        }
    }

    fn is_retryable(err: &ProviderError) -> bool {
        match err {
            ProviderError::RateLimited(_) | ProviderError::Timeout => true,
            ProviderError::Http(code, _) if (500..=599).contains(code) => true,
            _ => false,
        }
    }

    fn retry_delay(err: &ProviderError, attempt: u32) -> Duration {
        match err {
            ProviderError::RateLimited(secs) if *secs > 0.0 => {
                Duration::from_secs_f64((*secs).min(60.0))
            }
            _ => Duration::from_millis(500 * 2u64.pow(attempt.min(4))),
        }
    }

    pub async fn run(
        &mut self,
        user_message: &str,
        callback: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<(), String> {
        self.abort.clear();
        self.hooks.on_message(user_message);
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
                is_error: false,
            }],
        };
        self.state.add_message(user_msg);

        match self.run_loop(callback).await {
            Ok(()) => Ok(()),
            Err(e) if e == "aborted" => {
                callback(AgentEvent::Aborted);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn run_with_followups(
        &mut self,
        user_message: &str,
        follow_up_messages: Vec<String>,
        callback: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<(), String> {
        self.run(user_message, callback).await?;
        for msg in follow_up_messages {
            if self.abort.is_aborted() {
                callback(AgentEvent::Aborted);
                return Ok(());
            }
            self.state.add_message(Message {
                role: Role::User,
                content: vec![Content {
                    content_type: ContentType::Text,
                    text: Some(msg),
                    ..Default::default()
                }],
            });
            match self.run_loop(callback).await {
                Ok(()) => {}
                Err(e) if e == "aborted" => {
                    callback(AgentEvent::Aborted);
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
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

    fn is_blank_assistant(msg: &AssistantMessage) -> bool {
        if msg.content.is_empty() {
            return true;
        }
        msg.content.iter().all(|c| match c.content_type {
            ContentType::Text => c.text.as_deref().unwrap_or("").trim().is_empty(),
            ContentType::Thinking => c.thinking.as_deref().unwrap_or("").trim().is_empty(),
            _ => false,
        })
    }

    async fn run_loop(
        &mut self,
        callback: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<(), String> {
        self.overflow_retried = false;
        self.blank_retries = 0;
        let tool_defs_json = serde_json::to_string(&self.tools.tool_defs()).unwrap_or_default();

        for _ in 0..self.max_iterations {
            self.check_aborted()?;
            self.inject_steer_messages(callback);

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
                Err(e) if e == "aborted" => return Err(e),
                Err(e) if !self.overflow_retried && Self::is_context_overflow(&e) => {
                    callback(AgentEvent::Error {
                        message: "Context overflow detected, compacting and retrying...".to_string(),
                    });
                    self.overflow_retried = true;
                    let _ = self.compact(callback).await;
                    continue;
                }
                Err(e) => return Err(e),
            };

            let used = self.state.estimated_context_tokens(&tool_defs_json);
            callback(AgentEvent::TokenUpdate {
                used,
                limit,
                input_tokens: self.state.total_input_tokens,
                output_tokens: self.state.total_output_tokens,
            });

            let tool_calls: Vec<Content> = assistant_msg
                .content
                .iter()
                .filter(|c| c.content_type == ContentType::ToolUse)
                .cloned()
                .collect();

            if tool_calls.is_empty() {
                if Self::is_blank_assistant(&assistant_msg) && self.blank_retries < 2 {
                    self.blank_retries += 1;
                    tracing::warn!(
                        attempt = self.blank_retries,
                        "empty assistant response; retrying"
                    );
                    callback(AgentEvent::Error {
                        message: format!(
                            "Empty model response, retrying ({}/2)...",
                            self.blank_retries
                        ),
                    });
                    continue;
                }
                self.blank_retries = 0;
                self.state.add_assistant(&assistant_msg);
                callback(AgentEvent::Done);
                return Ok(());
            }

            self.blank_retries = 0;
            self.state.add_assistant(&assistant_msg);

            let has_sequential = self.tools.iter().any(|t| {
                t.execution_mode() == ToolExecutionMode::Sequential
            });

            if has_sequential || self.should_force_sequential() {
                self.execute_tools_sequential(&tool_calls, callback).await?;
            } else {
                self.execute_tools_parallel(&tool_calls, callback).await?;
            }

            self.check_aborted()?;
            self.inject_steer_messages(callback);
        }

        callback(AgentEvent::Error {
            message: format!("Reached max iterations ({})", self.max_iterations),
        });
        Err(format!("Reached max iterations ({})", self.max_iterations))
    }

    async fn stream_assistant(
        &self,
        callback: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<AssistantMessage, String> {
        let api_key = std::env::var(self.provider.api_key_env_var())
            .map_err(|_| format!("{} not set", self.provider.api_key_env_var()))?;

        let thinking = self.state.thinking_budget.filter(|b| *b > 0).map(|b| ThinkingConfig {
            r#type: "enabled".to_string(),
            budget_tokens: b,
        });
        // Anthropic extended thinking: max_tokens must exceed budget; temperature must be omitted/1.
        let (max_tokens, temperature) = if let Some(ref t) = thinking {
            (
                self.provider
                    .default_max_tokens()
                    .saturating_add(t.budget_tokens)
                    .max(t.budget_tokens.saturating_add(4096)),
                None,
            )
        } else {
            (self.provider.default_max_tokens(), Some(0.0))
        };

        let mut system = self.state.system_prompt.clone();
        if let Some(note) = self.state.mode.system_note() {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(note);
        }
        if self.should_force_sequential() {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(weak_model_system_note());
        }
        if self
            .rlm_escalate_hint_pending
            .load(std::sync::atomic::Ordering::Relaxed)
            && !self
                .rlm_escalate_hint_sent
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(rlm_escalate::system_note());
            self.rlm_escalate_hint_sent
                .store(true, std::sync::atomic::Ordering::Relaxed);
            self.rlm_escalate_hint_pending
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let tools: Vec<_> = self
            .tools
            .tool_defs()
            .into_iter()
            .filter(|t| self.state.allows_tool(&t.name))
            .collect();

        let request = ChatRequest {
            model: self.state.model.clone(),
            messages: self.state.messages.clone(),
            system: if system.is_empty() { None } else { Some(system) },
            tools,
            max_tokens,
            temperature,
            top_p: None,
            stop_sequences: None,
            stream: true,
            thinking,
        };

        let mut last_err = None;
        for attempt in 0..3u32 {
            self.check_aborted()?;
            match self.provider.chat_stream(&api_key, request.clone()).await {
                Ok(mut stream) => {
                    let mut content_blocks: Vec<Option<Content>> = Vec::new();
                    let mut tool_arg_buf: Vec<String> = Vec::new();
                    let usage: Option<Usage> = None;
                    let mut stop_reason: Option<StopReason> = None;
                    let model = String::new();
                    let msg_id: Option<String> = None;

                    while let Some(result) = stream.next().await {
                        self.check_aborted()?;
                        match result {
                            Ok(delta) => {
                                let idx = delta.content_index as usize;
                                match delta.r#type {
                                    DeltaType::Text { text } => {
                                        if content_blocks.len() <= idx {
                                            content_blocks.resize(idx + 1, None);
                                        }
                                        callback(AgentEvent::TextDelta { text: text.clone() });
                                        if let Some(Some(b)) = content_blocks.get_mut(idx) {
                                            if b.content_type == ContentType::Text {
                                                let existing = b.text.take().unwrap_or_default();
                                                b.text = Some(existing + &text);
                                            }
                                        } else {
                                            content_blocks[idx] = Some(Content {
                                                content_type: ContentType::Text,
                                                text: Some(text),
                                                ..Default::default()
                                            });
                                        }
                                    }
                                    DeltaType::Thinking { thinking } => {
                                        if content_blocks.len() <= idx {
                                            content_blocks.resize(idx + 1, None);
                                        }
                                        callback(AgentEvent::ThinkingDelta {
                                            thinking: thinking.clone(),
                                        });
                                        if let Some(Some(b)) = content_blocks.get_mut(idx) {
                                            if b.content_type == ContentType::Thinking {
                                                let existing = b.thinking.take().unwrap_or_default();
                                                b.thinking = Some(existing + &thinking);
                                            }
                                        } else {
                                            content_blocks[idx] = Some(Content {
                                                content_type: ContentType::Thinking,
                                                thinking: Some(thinking),
                                                ..Default::default()
                                            });
                                        }
                                    }
                                    DeltaType::Signature { signature } => {
                                        if let Some(Some(b)) = content_blocks.get_mut(idx) {
                                            b.signature = Some(signature);
                                        }
                                    }
                                    DeltaType::ToolCallStart { id, name, input } => {
                                        if content_blocks.len() <= idx {
                                            content_blocks.resize(idx + 1, None);
                                        }
                                        if tool_arg_buf.len() <= idx {
                                            tool_arg_buf.resize(idx + 1, String::new());
                                        }
                                        callback(AgentEvent::ToolUseStart {
                                            id: id.clone(),
                                            name: name.clone(),
                                        });
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
                                        callback(AgentEvent::ToolUseDelta {
                                            input: input.clone(),
                                        });
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
                                if Self::is_retryable(&e) && attempt < 2 {
                                    last_err = Some(e);
                                    break;
                                }
                                callback(AgentEvent::Error {
                                    message: format!("stream error: {:?}", e),
                                });
                                return Err(format!("stream error: {:?}", e));
                            }
                        }
                    }

                    if let Some(err) = last_err.take() {
                        let delay = Self::retry_delay(&err, attempt);
                        callback(AgentEvent::Status {
                            message: format!(
                                "retrying after {:?} (attempt {}/3)...",
                                err,
                                attempt + 1
                            ),
                        });
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    for (i, block) in content_blocks.iter_mut().enumerate() {
                        if let Some(b) = block {
                            if b.content_type == ContentType::ToolUse {
                                if let Some(raw) = tool_arg_buf.get(i) {
                                    b.input = Some(match serde_json::from_str(raw) {
                                        Ok(v) => v,
                                        Err(e) => make_arg_parse_error_value(
                                            &e.to_string(),
                                            raw,
                                        ),
                                    });
                                }
                            }
                        }
                    }

                    return Ok(AssistantMessage {
                        content: content_blocks.into_iter().flatten().collect(),
                        stop_reason,
                        usage,
                        model,
                        id: msg_id,
                    });
                }
                Err(e) if Self::is_retryable(&e) && attempt < 2 => {
                    let delay = Self::retry_delay(&e, attempt);
                    callback(AgentEvent::Status {
                        message: format!(
                            "retrying after {:?} (attempt {}/3)...",
                            e,
                            attempt + 1
                        ),
                    });
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(format!("stream error: {:?}", e)),
            }
        }

        Err(format!(
            "stream error after retries: {:?}",
            last_err.unwrap_or(ProviderError::Other("unknown".into()))
        ))
    }


    async fn execute_tools_sequential(
        &mut self,
        tool_calls: &[Content],
        callback: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<(), String> {
        for tc in tool_calls {
            self.check_aborted()?;
            let id = tc.id.as_deref().unwrap_or("");
            let name = tc.name.as_deref().unwrap_or("");
            let input = tc.input.clone().unwrap_or(serde_json::Value::Null);

            let result = self.execute_single_tool(id, name, &input).await;
            callback(AgentEvent::ToolResult {
                id: id.to_string(),
                name: name.to_string(),
                result: result.clone(),
            });
            self.store_tool_result(id, name, &result.content, result.is_error);

            if result.terminate {
                break;
            }
        }
        Ok(())
    }

    async fn execute_tools_parallel(
        &mut self,
        tool_calls: &[Content],
        callback: &mut (dyn FnMut(AgentEvent) + Send),
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

        let futures: Vec<_> = tool_data.iter().map(|job| {
            self.execute_single_tool(&job.id, &job.name, &job.input)
        }).collect();

        let results = futures::future::join_all(futures).await;

        for (job, result) in tool_data.iter().zip(results.iter()) {
            callback(AgentEvent::ToolResult {
                id: job.id.clone(),
                name: job.name.clone(),
                result: result.clone(),
            });
            self.store_tool_result(&job.id, &job.name, &result.content, result.is_error);
        }

        for result in &results {
            if result.terminate {
                break;
            }
        }

        Ok(())
    }

    fn store_tool_result(&mut self, id: &str, name: &str, content: &str, is_error: bool) {
        let original_len = content.chars().count();
        let mut stored = truncate_tool_output(content, MAX_TOOL_OUTPUT_CHARS);
        if stored.chars().count() < original_len {
            stored = rlm_escalate::append_truncate_escalate_hint(name, &stored, original_len);
        }
        if rlm_escalate::content_has_escalate(&stored) {
            self.rlm_escalate_hint_pending
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.state
            .add_tool_result(id.to_string(), name.to_string(), stored, is_error);
    }

    async fn compact(
        &mut self,
        callback: &mut (dyn FnMut(AgentEvent) + Send),
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

        // Pin recent file paths + failed edits so they survive later compaction.
        let mut pins = CompactPins::default();
        if let Some(prev) = previous_summary {
            pins.merge(CompactPins::from_summary_text(prev));
        }
        pins.merge(collect_pins_from_messages(&to_summarize));
        pins.merge(collect_pins_from_messages(&keep_msgs));
        let summary = append_pins_to_summary(&summary, &pins);

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
        let tool = match resolve_tool(&self.tools, name) {
            Ok(t) => t,
            Err(msg) => return ToolExecuteResult::error(msg),
        };
        let resolved_name = tool.name().to_string();

        if !self.state.allows_tool(&resolved_name) {
            if !self.state.mode.allows_tool(&resolved_name) {
                return ToolExecuteResult::error(format!(
                    "Tool '{}' blocked in {} mode. Switch with /mode agent (or plan/ask).",
                    resolved_name,
                    self.state.mode.as_str()
                ));
            }
            return ToolExecuteResult::error(format!(
                "Tool '{}' blocked by active skill tool allow-list [{}]. Load a different skill or clear with /new.",
                resolved_name,
                self.state.skill_tools.join(", ")
            ));
        }

        let args = match prepare_tool_args(tool.as_ref(), input.clone()) {
            Ok(a) => a,
            Err(msg) => return ToolExecuteResult::error(msg),
        };

        if let Err(msg) = self.hooks.before_tool(&resolved_name, &args.to_string()) {
            return ToolExecuteResult::error(msg);
        }

        let fp = tool_call_fingerprint(&resolved_name, &args);
        if let Ok(mut recent) = self.recent_tool_fps.lock() {
            // Block after 3 identical calls in a row (opencode-style last-3 guard).
            let same_streak = recent.iter().rev().take(3).filter(|p| *p == &fp).count();
            if same_streak >= 3 {
                return ToolExecuteResult::error(format!(
                    "Repeated identical `{resolved_name}` call detected (doom loop).\n\
                     Change your approach: different arguments, another tool, or ask the user.\n\
                     Do not retry the exact same call again."
                ));
            }
            recent.push_back(fp);
            while recent.len() > 6 {
                recent.pop_front();
            }
        }

        if let Some(ref tx) = self.permission_tx {
            if tool.requires_permission() {
                let danger_reason = if resolved_name == "bash" {
                    args.get("command")
                        .and_then(|v| v.as_str())
                        .and_then(crate::tools::bash::is_dangerous_command)
                        .map(|s| s.to_string())
                } else {
                    None
                };
                let diff_preview = if resolved_name == "edit" {
                    crate::tools::edit::preview_edit_diff(&args)
                } else if resolved_name == "apply_patch" {
                    crate::tools::apply_patch::preview_apply_patch(&args)
                } else {
                    None
                };
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(PendingPermission {
                    request: crate::permission::PermissionRequest {
                        tool_name: resolved_name.clone(),
                        tool_input: args.to_string(),
                        danger_reason,
                        diff_preview,
                    },
                    reply_tx,
                });
                match reply_rx.await {
                    Ok(PermissionReply::AllowOnce) | Ok(PermissionReply::AllowAlways) => {}
                    Ok(PermissionReply::Deny) => {
                        return ToolExecuteResult::error("Permission denied by user");
                    }
                    Err(_) => {
                        return ToolExecuteResult::error("Permission prompt cancelled");
                    }
                }
            }
        }
        tracing::info!(tool = %resolved_name, requested = %name, "executing tool");
        let result = tool.execute(tool_call_id, args).await;
        self.hooks
            .after_tool(&resolved_name, result.is_error, &result.content);
        result
    }
}

#[cfg(test)]
mod tool_output_truncation_tests {
    use super::truncate_tool_output;

    #[test]
    fn leaves_short_output_untouched() {
        let s = "hello world";
        assert_eq!(truncate_tool_output(s, 100), s);
    }

    #[test]
    fn leaves_output_at_exactly_max_untouched() {
        let s = "a".repeat(50);
        assert_eq!(truncate_tool_output(&s, 50), s);
    }

    #[test]
    fn truncates_long_output_keeping_head_and_tail() {
        let s = "a".repeat(50) + &"b".repeat(50) + &"c".repeat(50);
        let out = truncate_tool_output(&s, 60);
        assert!(out.starts_with(&"a".repeat(30)));
        assert!(out.ends_with(&"c".repeat(30)));
        assert!(out.contains("...[truncated"));
        assert!(out.contains("chars]..."));
    }

    #[test]
    fn truncated_marker_reports_correct_dropped_count() {
        let s = "x".repeat(1000);
        let out = truncate_tool_output(&s, 100);
        // 1000 total chars, 100 kept -> 900 dropped.
        assert!(out.contains("...[truncated 900 chars]..."));
    }

    #[test]
    fn does_not_split_multibyte_utf8_chars() {
        // Each "é" is 2 bytes in UTF-8 but 1 char; ensure char-based slicing
        // never panics or produces invalid UTF-8 sequences.
        let s = "é".repeat(200);
        let out = truncate_tool_output(&s, 50);
        assert!(out.chars().count() < s.chars().count());
        // Would panic on invalid UTF-8 boundaries if this were byte-based.
        let _ = out.len();
    }
}


