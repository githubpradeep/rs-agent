use crate::agent::r#loop::AgentEvent;
use crate::agent::state::AgentState;
use crate::agent::{AgentLoop, AgentMode};
use crate::ai::provider::Provider;
use crate::ai::types::Message;
use crate::context::{
    build_commands_section, build_context_section, discover_agent_commands,
    discover_context_files,
};
use crate::permission::{PendingPermission, PermissionReply, TrustStore};
use crate::session::{self, SessionData, SessionStore};
use crate::skills::{
    discover_skills, discover_templates, find_skill, find_template, format_skill_injection,
    list_skills_summary, render_template,
};
use crossbeam_channel as channel;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use super::keys::{merge_keybindings, KeyMap};
use super::renderer::render_markdown;
use super::theme::{Palette, ThemeName};
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use walkdir::WalkDir;

#[derive(Clone)]
struct ChatMessage {
    role: String,
    text: String,
    thinking: Option<String>,
    show_thinking: bool,
    tool_blocks: Vec<ToolBlock>,
}

/// A collapsible record of a single tool invocation's result, rendered
/// beneath its owning chat message.
#[derive(Clone)]
struct ToolBlock {
    name: String,
    preview: String,
    full: String,
    expanded: bool,
    is_error: bool,
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    Insert,
    Waiting,
    /// Capturing an API key for `pending_api_key_provider`.
    ApiKey,
}

#[derive(PartialEq, Clone, Copy)]
enum PickerMode {
    File,
    Dir,
    Skill,
    Prompt,
    Model,
    Provider,
}

#[allow(dead_code)]
enum AppCommand {
    Submit { text: String },
    Steer { text: String },
    Abort,
    Compact,
    NewSession,
    SetModel { model: String },
    /// Mid-session provider+model swap (pi parity).
    SetProvider {
        provider: Arc<dyn Provider>,
        model: String,
    },
    SetMode { mode: AgentMode },
    SetTitle { title: String },
    SetSystemPrompt { prompt: String },
    Init { messages: Vec<Message> },
    Exit,
}

pub struct App {
    messages: Vec<ChatMessage>,
    input: String,
    input_mode: InputMode,
    should_exit: bool,
    status: String,
    command_tx: channel::Sender<AppCommand>,
    event_rx: channel::Receiver<(usize, AgentEvent)>,
    scroll_offset: usize,
    follow_bottom: bool,
    picker_active: bool,
    picker_mode: PickerMode,
    picker_prefix: String,
    picker_query: String,
    picker_results: Vec<String>,
    picker_selection: usize,
    picker_files: Vec<String>,
    picker_files_loaded: bool,
    picker_dirs: Vec<String>,
    picker_dirs_loaded: bool,
    picker_models: Vec<String>,
    picker_providers: Vec<String>,
    model_picker_rx: Option<channel::Receiver<Vec<String>>>,
    /// When set, insert mode is replaced by API-key capture for this provider.
    pending_api_key_provider: Option<String>,
    pending_permission: Option<PendingPermission>,
    permission_rx: channel::Receiver<PendingPermission>,
    trust_store: TrustStore,
    #[allow(dead_code)]
    approved: bool,
    auto_mode: bool,
    token_used: usize,
    token_limit: usize,
    near_limit: bool,
    session_id: String,
    session_title: Option<String>,
    model_name: String,
    agent_mode: AgentMode,
    chat_area_y: u16,
    thinking_targets: Vec<(usize, usize)>,
    tool_targets: Vec<(usize, usize, usize)>,
    tree_breadcrumb: String,
    abort_flag: crate::agent::AbortFlag,
    steer_queue: crate::agent::SteerQueue,
    queued_steers: usize,
    input_history: Vec<String>,
    history_index: Option<usize>,
    rlm_depth: u32,
    palette: Palette,
    theme_name: ThemeName,
    keys: KeyMap,
    show_tree_panel: bool,
    tree_panel_text: String,
    show_repl_panel: bool,
    repl_panel: String,
    tool_in_progress: Option<(String, Instant)>,
    context_enabled: bool,
    provider: Arc<dyn Provider>,
    provider_name: String,
    timeout_secs: u64,
    /// Cycle list of `provider/model` display strings (pi Ctrl+P).
    model_cycle: Vec<String>,
    model_cycle_index: usize,
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: String, timeout_secs: u64, approve: bool, resume: Option<SessionData>, system_prompt: Option<String>, max_iterations: usize, auto_mode: bool, rlm_depth: u32, thinking_budget: Option<u32>) -> Self {
        let cfg = crate::config::Config::load();
        let theme_name = ThemeName::parse(cfg.theme.as_deref().unwrap_or("dark"));
        let palette = Palette::for_theme(theme_name);
        let keys = KeyMap::new(merge_keybindings(&cfg.keybindings));
        let provider_for_app = provider.clone();

        let (command_tx, command_rx) = channel::unbounded::<AppCommand>();
        let (event_tx, event_rx) = channel::unbounded::<(usize, AgentEvent)>();
        let (permission_tx, permission_rx) = channel::unbounded::<PendingPermission>();

        let abort_flag = crate::agent::AbortFlag::new();
        let steer_queue = crate::agent::SteerQueue::new();
        let abort_for_thread = abort_flag.clone();
        let steer_for_thread = steer_queue.clone();

        let provider_name = provider.name().to_string();
        let provider_name_for_banner = provider_name.clone();
        let provider2 = provider.clone();
        let model2 = model.clone();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let session_id =
            resume.as_ref().map(|s| s.id.clone()).unwrap_or_else(SessionStore::generate_id);
        let created_at = resume.as_ref().map(|s| s.created_at.clone()).unwrap_or_else(|| {
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
        });
        let resume_msgs = resume.as_ref().map(|s| s.messages.clone()).unwrap_or_default();
        let title = resume.as_ref().and_then(|s| s.title.clone());
        let session_id_for_thread = session_id.clone();
        let created_at_for_thread = created_at.clone();
        let title_for_thread = title.clone();
        let system_prompt_for_thread = system_prompt.clone();
        let max_rlm_depth = rlm_depth;

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let sp = system_prompt_for_thread.unwrap_or_else(|| {
                    crate::agent::default_system_prompt()
                });

                let mut state = AgentState::new(model2, provider_name)
                    .with_system_prompt(sp)
                    .with_thinking_budget(thinking_budget);

                for msg in &resume_msgs {
                    state.add_message(msg.clone());
                }

                let mut agent_loop = AgentLoop::new(provider2, state)
                    .with_max_iterations(max_iterations)
                    .with_abort(abort_for_thread.clone())
                    .with_steer(steer_for_thread.clone())
                    .with_rlm_depth(0, max_rlm_depth);
                if !approve {
                    agent_loop.set_permission_channel(permission_tx);
                }
                // Bridge tool-emitted events (REPL stdout) onto the TUI event channel.
                let (sink_tx, sink_rx) = channel::unbounded::<AgentEvent>();
                let event_tx_bridge = event_tx.clone();
                std::thread::spawn(move || {
                    while let Ok(ev) = sink_rx.recv() {
                        let _ = event_tx_bridge.send((0, ev));
                    }
                });
                agent_loop = agent_loop.with_event_sink(sink_tx);
                crate::tools::register_default_tools_with_rlm(&mut agent_loop, max_rlm_depth);

                let store = SessionStore::new();
                let mut session_id_local = session_id_for_thread.clone();
                let mut created_at_local = created_at_for_thread.clone();
                let mut title_local = title_for_thread.clone();

                loop {
                    let cmd = command_rx.recv().unwrap_or(AppCommand::Exit);
                    match cmd {
                        AppCommand::Exit => break,
                        AppCommand::Init { messages } => {
                            for msg in messages {
                                agent_loop.state_mut().add_message(msg);
                            }
                        }
                        AppCommand::Abort => {
                            abort_for_thread.abort();
                        }
                        AppCommand::Steer { text } => {
                            steer_for_thread.push(text);
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: "steer queued".to_string(),
                            }));
                        }
                        AppCommand::Compact => {
                            let _ = agent_loop.compact_now(&mut |event: AgentEvent| {
                                let _ = event_tx.send((0, event));
                            }).await;
                        }
                        AppCommand::NewSession => {
                            agent_loop.clear_messages();
                            abort_for_thread.clear();
                            steer_for_thread.clear();
                            session_id_local = SessionStore::generate_id();
                            created_at_local = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: format!("new session {}", session_id_local),
                            }));
                        }
                        AppCommand::SetModel { model } => {
                            agent_loop.set_model(model.clone());
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: format!("model set to {}", model),
                            }));
                        }
                        AppCommand::SetProvider { provider, model } => {
                            let pname = provider.name().to_string();
                            agent_loop.set_provider_and_model(provider, model.clone());
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: format!("provider {} · model {}", pname, model),
                            }));
                        }
                        AppCommand::SetMode { mode } => {
                            agent_loop.state_mut().set_mode(mode);
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: format!("mode set to {}", mode.as_str()),
                            }));
                        }
                        AppCommand::SetSystemPrompt { prompt } => {
                            agent_loop.state_mut().system_prompt = prompt;
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: "system prompt rebuilt".to_string(),
                            }));
                        }
                        AppCommand::SetTitle { title } => {
                            title_local = Some(title.clone());
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            let s = agent_loop.state();
                            let tree_snapshot =
                                serde_json::to_value(agent_loop.call_tree().snapshot()).ok();
                            let session_data = SessionData {
                                id: session_id_local.clone(),
                                title: title_local.clone(),
                                created_at: created_at_local.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                                call_tree: tree_snapshot,
                            };
                            let _ = store.save(&session_data);
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: format!("renamed to \"{}\"", title),
                            }));
                        }
                        AppCommand::Submit { text } => {
                            abort_for_thread.clear();
                            let result = tokio::time::timeout(
                                timeout,
                                agent_loop.run(&text, &mut |event: AgentEvent| {
                                    let _ = event_tx.send((0, event));
                                }),
                            )
                            .await;
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            match result {
                                Ok(Ok(())) => {
                                    let _ = event_tx.send((0, AgentEvent::TreeUpdate {
                                        tree: agent_loop.call_tree().clone(),
                                    }));
                                }
                                Ok(Err(e)) => {
                                    let _ = event_tx.send((0, AgentEvent::Error { message: e }));
                                }
                                Err(_) => {
                                    abort_for_thread.abort();
                                    let _ = event_tx.send((0, AgentEvent::Error {
                                        message: format!("Request timed out after {}s", timeout_secs),
                                    }));
                                }
                            }
                            let s = agent_loop.state();
                            let tree_snapshot =
                                serde_json::to_value(agent_loop.call_tree().snapshot()).ok();
                            let mut session_data = SessionData {
                                id: session_id_local.clone(),
                                title: title_local.clone(),
                                created_at: created_at_local.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                                call_tree: tree_snapshot,
                            };
                            session_data.ensure_title();
                            title_local = session_data.title.clone();
                            if let Some(ref t) = title_local {
                                let _ = event_tx.send((0, AgentEvent::TitleUpdate { title: t.clone() }));
                            }
                            let _ = store.save(&session_data);
                        }
                    }
                }
            });
        });

        let trust_store = TrustStore::new();

        let provider_banner = if provider_name_for_banner.contains("opencode-cli") {
            format!("{} (experimental)", provider_name_for_banner)
        } else {
            provider_name_for_banner.clone()
        };
        let mut initial_msgs = vec![ChatMessage {
            role: "system".to_string(),
            text: format!(
                "Rs Agent — RLM coding harness\nProvider: {}\nModel: {}\nSession: {}\n\nType a message to start.\ni: insert | Esc: normal | t: toggle thinking | G: bottom\nEnter while waiting: steer | Esc while waiting: abort\n/help for commands | ^C: quit",
                provider_banner, model, session_id
            ),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        }];

        if !crate::rlm::python3_available() {
            initial_msgs.push(ChatMessage {
                role: "system".to_string(),
                text: format!("⚠️ {} The `repl` tool (and RLM sub-agents) will fail until it's installed.", crate::rlm::PYTHON3_NOT_FOUND),
                thinking: None,
                show_thinking: false,
                tool_blocks: Vec::new(),
            });
        }

        if let Some(ref resume_data) = resume {
            for msg in &resume_data.messages {
                match &msg.role {
                    crate::ai::types::Role::User => {
                        let text = msg.content.first().and_then(|c| c.text.as_deref()).unwrap_or("");
                        if !text.is_empty() {
                            initial_msgs.push(ChatMessage { role: "user".to_string(), text: text.to_string(), thinking: None, show_thinking: false, tool_blocks: Vec::new() });
                        }
                    }
                    crate::ai::types::Role::Assistant => {
                        let mut text = String::new();
                        let mut thinking: Option<String> = None;
                        for c in &msg.content {
                            match c.content_type {
                                crate::ai::types::ContentType::Text => {
                                    if let Some(ref t) = c.text {
                                        text.push_str(t);
                                    }
                                }
                                crate::ai::types::ContentType::ToolUse => {
                                    let name = c.name.as_deref().unwrap_or("tool");
                                    let input = c.input.as_ref().map(|v| v.to_string()).unwrap_or_default();
                                    let preview: String = input.chars().take(120).collect();
                                    text.push_str(&format!("\n🛠 {} {}\n", name, preview));
                                }
                                crate::ai::types::ContentType::Thinking => {
                                    if let Some(ref t) = c.thinking {
                                        thinking = Some(t.clone());
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !text.is_empty() {
                            initial_msgs.push(ChatMessage { role: "assistant".to_string(), text, thinking: thinking.clone(), show_thinking: thinking.as_ref().map(|t| !t.is_empty()).unwrap_or(false), tool_blocks: Vec::new() });
                        }
                    }
                    crate::ai::types::Role::Tool => {
                        let name = msg.content.first().and_then(|c| c.name.as_deref()).unwrap_or("tool");
                        let result = msg.content.first().and_then(|c| c.text.as_deref()).unwrap_or("");
                        let preview: String = result.chars().take(200).collect();
                        if !preview.is_empty() {
                            initial_msgs.push(ChatMessage { role: "tool".to_string(), text: format!("✅ [{}] {}", name, preview), thinking: None, show_thinking: false, tool_blocks: Vec::new() });
                        }
                    }
                    _ => {}
                }
            }
        }

        Self {
            messages: initial_msgs,
            input: String::new(),
            input_mode: InputMode::Insert,
            should_exit: false,
            status: "ready".to_string(),
            command_tx,
            event_rx,
            scroll_offset: 0,
            follow_bottom: true,
            picker_active: false,
            picker_mode: PickerMode::File,
            picker_prefix: String::new(),
            picker_query: String::new(),
            picker_results: Vec::new(),
            picker_selection: 0,
            picker_files: Vec::new(),
            picker_files_loaded: false,
            picker_dirs: Vec::new(),
            picker_dirs_loaded: false,
            picker_models: Vec::new(),
            picker_providers: Vec::new(),
            model_picker_rx: None,
            pending_api_key_provider: None,
            pending_permission: None,
            permission_rx,
            trust_store,
            approved: approve,
            auto_mode,
            token_used: 0,
            token_limit: crate::ai::token_count::get_context_limit(&model),
            near_limit: false,
            session_id,
            session_title: title,
            model_name: model.clone(),
            agent_mode: AgentMode::Agent,
            chat_area_y: 0,
            thinking_targets: Vec::new(),
            tool_targets: Vec::new(),
            tree_breadcrumb: "idle".to_string(),
            abort_flag,
            steer_queue,
            queued_steers: 0,
            input_history: Vec::new(),
            history_index: None,
            rlm_depth,
            palette,
            theme_name,
            keys,
            show_tree_panel: false,
            tree_panel_text: "(no call tree yet)".to_string(),
            show_repl_panel: false,
            repl_panel: String::new(),
            tool_in_progress: None,
            context_enabled: true,
            provider: provider_for_app,
            provider_name: provider_name_for_banner.clone(),
            timeout_secs,
            model_cycle: {
                let mut cycle: Vec<String> = crate::ai::registry::available_model_refs()
                    .into_iter()
                    .map(|r| r.display())
                    .collect();
                let current = format!("{}/{}", provider_name_for_banner, model);
                if !cycle.iter().any(|c| c == &current) {
                    cycle.insert(0, current);
                }
                cycle
            },
            model_cycle_index: 0,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(&mut stdout))?;
        crossterm::execute!(
            io::stdout(),
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
            EnableBracketedPaste
        )?;

        loop {
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(10))? {
                self.handle_event(event::read()?)?;
            }

            while let Ok((_idx, event)) = self.event_rx.try_recv() {
                self.handle_agent_event(event);
            }

            self.poll_model_picker();

            if let Ok(pending) = self.permission_rx.try_recv() {
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let is_trusted = self.approved || self.trust_store.is_trusted(&cwd);
                if is_trusted || self.auto_allow(&pending.request.tool_name) {
                    let _ = pending.reply_tx.send(PermissionReply::AllowOnce);
                } else {
                    self.pending_permission = Some(pending);
                }
            }

            if self.should_exit {
                break;
            }
        }

        let _ = self.command_tx.send(AppCommand::Exit);
        terminal::disable_raw_mode()?;
        crossterm::execute!(
            io::stdout(),
            LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
            DisableBracketedPaste
        )?;
        Ok(())
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta { text } => {
                if self.messages.is_empty()
                    || self.messages.last().map(|m| m.role.as_str()) != Some("assistant")
                {
                    self.messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        text: String::new(),
                        thinking: None,
                        show_thinking: false,
                        tool_blocks: Vec::new(),
                    });
                }
                if let Some(last) = self.messages.last_mut() {
                    last.text.push_str(&text);
                }
                self.follow_bottom = true;
            }
            AgentEvent::ThinkingDelta { thinking } => {
                if self.messages.is_empty()
                    || self.messages.last().map(|m| m.role.as_str()) != Some("assistant")
                {
                    self.messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        text: String::new(),
                        thinking: None,
                        show_thinking: true,
                        tool_blocks: Vec::new(),
                    });
                }
                if let Some(last) = self.messages.last_mut() {
                    let was_empty = last.thinking.as_ref().map(|t| t.is_empty()).unwrap_or(true);
                    let prev = last.thinking.take().unwrap_or_default();
                    last.thinking = Some(prev + &thinking);
                    // Auto-expand only when thinking first appears; respect click-to-hide after that.
                    if was_empty {
                        last.show_thinking = true;
                    }
                }
                self.status = "thinking...".to_string();
                self.follow_bottom = true;
            }
            AgentEvent::ToolUseStart { id: _, name } => {
                self.status = format!("using {}...", name);
                self.follow_bottom = true;
                if name == "repl" {
                    self.repl_panel.clear();
                    self.show_repl_panel = true;
                }
                self.tool_in_progress = Some((name, Instant::now()));
            }
            AgentEvent::ToolResult { id: _, name, result } => {
                self.tool_in_progress = None;
                let mut full = result.content.clone();
                if full.starts_with("Exit code: ") {
                    if let Some(rest) = full.splitn(2, '\n').nth(1) {
                        full = rest.to_string();
                    }
                }
                let preview: String = full.chars().take(100).collect();
                if let Some(last) = self.messages.last_mut() {
                    last.tool_blocks.push(ToolBlock {
                        name,
                        preview,
                        full,
                        expanded: false,
                        is_error: result.is_error,
                    });
                }
                self.follow_bottom = true;
            }
            AgentEvent::Error { message } => {
                if let Some(last) = self.messages.last_mut() {
                    last.text.push_str(&format!("\n❌ Error: {}", message));
                }
                self.status = "error".to_string();
                self.input_mode = InputMode::Insert;
                self.tool_in_progress = None;
            }
            AgentEvent::TurnEnd { stop_reason: _ } => {
                self.status = "ready".to_string();
            }
            AgentEvent::Done => {
                self.status = "ready".to_string();
                self.input_mode = InputMode::Insert;
                self.input.clear();
                self.near_limit = false;
                self.queued_steers = 0;
                self.tool_in_progress = None;
            }
            AgentEvent::ToolUseDelta { input: _ } => {}
            AgentEvent::ReplOutput { stream, text } => {
                let prefix = if stream == "stderr" { "! " } else { "" };
                for line in text.lines() {
                    self.repl_panel.push_str(prefix);
                    self.repl_panel.push_str(line);
                    self.repl_panel.push('\n');
                }
                const REPL_PANEL_CAP: usize = 8000;
                if self.repl_panel.len() > REPL_PANEL_CAP {
                    let excess = self.repl_panel.len() - REPL_PANEL_CAP;
                    let cut = self.repl_panel[excess..]
                        .find('\n')
                        .map(|i| excess + i + 1)
                        .unwrap_or(excess);
                    self.repl_panel.drain(..cut);
                }
                self.show_repl_panel = true;
            }
            AgentEvent::ContextWarning { fraction: _, used, limit } => {
                self.token_used = used;
                self.token_limit = limit;
                self.near_limit = true;
            }
            AgentEvent::TokenUpdate { used, limit } => {
                self.token_used = used;
                self.token_limit = limit;
            }
            AgentEvent::Compacting => {
                self.status = "compacting...".to_string();
            }
            AgentEvent::Compacted { summary: _ } => {
                self.status = "compacted".to_string();
                self.near_limit = false;
            }
            AgentEvent::Status { message } => {
                self.status = message;
            }
            AgentEvent::Aborted => {
                if let Some(last) = self.messages.last_mut() {
                    last.text.push_str("\n⊘ aborted");
                }
                self.status = "aborted".to_string();
                self.input_mode = InputMode::Insert;
                self.tool_in_progress = None;
            }
            AgentEvent::TreeUpdate { tree } => {
                self.tree_breadcrumb = tree.breadcrumb();
                self.tree_panel_text = tree.render();
            }
            AgentEvent::TitleUpdate { title } => {
                self.session_title = Some(title);
            }
        }
    }

    fn handle_event(&mut self, event: Event) -> io::Result<()> {
        match event {
            Event::Mouse(mouse) => {
                self.follow_bottom = false;
                match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        self.scroll_offset = self.scroll_offset.saturating_add(3);
                    }
                    MouseEventKind::ScrollUp => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(3);
                    }
                    MouseEventKind::Down(button) if button == crossterm::event::MouseButton::Left => {
                        let screen_row = mouse.row as usize;
                        let visible_start = (self.chat_area_y + 1) as usize;
                        if screen_row >= visible_start {
                            let visible_idx = screen_row - visible_start;
                            let content_line = visible_idx + self.scroll_offset;
                            let mut handled = false;
                            for &(target_line, msg_idx) in &self.thinking_targets {
                                if target_line == content_line {
                                    if let Some(msg) = self.messages.get_mut(msg_idx) {
                                        msg.show_thinking = !msg.show_thinking;
                                    }
                                    handled = true;
                                    break;
                                }
                            }
                            if !handled {
                                for &(target_line, msg_idx, tool_idx) in &self.tool_targets {
                                    if target_line == content_line {
                                        if let Some(block) = self
                                            .messages
                                            .get_mut(msg_idx)
                                            .and_then(|m| m.tool_blocks.get_mut(tool_idx))
                                        {
                                            block.expanded = !block.expanded;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if self.pending_permission.is_some() {
                    self.handle_permission_key(key);
                } else {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.should_exit = true;
                        }
                        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.cycle_model();
                        }
                        _ => match self.input_mode {
                            InputMode::Waiting => self.handle_waiting_key(key),
                            InputMode::Normal => self.handle_normal_key(key),
                            InputMode::Insert => self.handle_insert_key(key),
                            InputMode::ApiKey => self.handle_api_key_input(key),
                        },
                    }
                }
            },
            Event::Paste(text) => {
                if self.pending_permission.is_some() {
                    // ignore paste while a permission prompt is up
                } else if self.picker_active {
                    self.picker_query.push_str(text.replace(['\n', '\r'], "").as_str());
                    self.update_picker_results();
                } else {
                    match self.input_mode {
                        InputMode::Insert | InputMode::Waiting | InputMode::ApiKey => {
                            self.input
                                .push_str(text.replace(['\n', '\r'], "").as_str());
                        }
                        InputMode::Normal => {}
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_waiting_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.abort_flag.abort();
                let _ = self.command_tx.send(AppCommand::Abort);
                self.status = "aborting...".to_string();
            }
            KeyCode::Enter => {
                if !self.input.trim().is_empty() {
                    let text = std::mem::take(&mut self.input);
                    self.queued_steers += 1;
                    self.steer_queue.push(text.clone());
                    let _ = self.command_tx.send(AppCommand::Steer { text: text.clone() });
                    self.messages.push(ChatMessage {
                        role: "user".to_string(),
                        text: format!("[steer] {}", text),
                        thinking: None,
                        show_thinking: false,
                        tool_blocks: Vec::new(),
                    });
                    self.status = format!("steer queued ({})", self.queued_steers);
                }
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            _ => {}
        }
    }

    fn push_system(&mut self, text: impl Into<String>) {
        self.messages.push(ChatMessage {
            role: "system".to_string(),
            text: text.into(),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        });
        self.follow_bottom = true;
    }

    fn handle_slash_command(&mut self, text: &str) -> bool {
        let trimmed = text.trim();
        if !trimmed.starts_with('/') {
            return false;
        }
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();
        match cmd {
            "/help" => {
                self.push_system(format!(
                    "Commands:\n\
                     /help /keys /clear /context [on|off] /commands /tree\n\
                     /skills  /skill <name>  /prompt|/p <name> [args]  /reload\n\
                     /mode plan|ask|agent  /model [provider/model]  /provider|/login [name]\n\
                     /theme [dark|light|forest]  /compact  /new  /sessions  /export\n\
                     /trust list|reset  /rename <title>  /history [query|n]\n\n\
                     Keys: {}\n\
                     Ctrl+P cycle model · Tab-complete /skill|/prompt · @ file · # dir",
                    self.keys.hint_line(),
                ));
            }
            "/keys" => {
                self.push_system(format!(
                    "Keymap\n\
                     {insert}          insert mode\n\
                     Esc        normal / abort when waiting\n\
                     Enter      send (insert) / steer (waiting)\n\
                     Up/Down    history (insert) / scroll (normal)\n\
                     {thinking}          toggle last thinking block\n\
                     {expand}          toggle last tool result block\n\
                     {tree}          toggle the live call-tree side panel\n\
                     {bottom}          jump to bottom\n\
                     @          file picker\n\
                     #          directory picker (attach dir summary)\n\
                     Tab        complete /skill or /prompt names\n\
                     ^P         cycle provider/model (ready providers)\n\
                     {once}/{deny}/{always}      allow once / deny / trust always (permission prompt)\n\
                     ^C         quit\n\n\
                     Remap single-key actions in ~/.rs-agent/config.toml under [keybindings].\n\
                     /model = model picker · /provider|/login = connect provider (paste key).",
                    insert = self.keys.binding("insert"),
                    thinking = self.keys.binding("toggle_thinking"),
                    expand = self.keys.binding("expand_tool"),
                    tree = self.keys.binding("toggle_tree"),
                    bottom = self.keys.binding("jump_bottom"),
                    once = self.keys.binding("perm_once"),
                    deny = self.keys.binding("perm_deny"),
                    always = self.keys.binding("perm_always"),
                ));
            }
            "/theme" => {
                if arg.is_empty() {
                    self.push_system(format!(
                        "Current theme: {}\nUsage: /theme dark|light|forest",
                        self.theme_name.as_str()
                    ));
                } else {
                    self.theme_name = ThemeName::parse(arg);
                    self.palette = Palette::for_theme(self.theme_name);
                    self.push_system(format!("Theme set to {}", self.theme_name.as_str()));
                }
            }
            "/history" => {
                if arg.is_empty() {
                    if self.input_history.is_empty() {
                        self.push_system("No input history yet.");
                    } else {
                        let total = self.input_history.len();
                        let start = total.saturating_sub(20);
                        let mut out = String::from("Input history (most recent, newest last):\n");
                        for (i, entry) in self.input_history.iter().enumerate().skip(start) {
                            let preview: String = entry.chars().take(100).collect();
                            out.push_str(&format!("  {:>3}  {}\n", i + 1, preview.replace('\n', " ⏎ ")));
                        }
                        out.push_str("\n/history <query> to search · /history <n> to edit that entry");
                        self.push_system(out);
                    }
                } else if let Ok(n) = arg.parse::<usize>() {
                    if n >= 1 && n <= self.input_history.len() {
                        self.input = self.input_history[n - 1].clone();
                        self.history_index = None;
                        self.status = format!("history #{} loaded into input — edit & Enter", n);
                    } else {
                        self.push_system(format!(
                            "No history entry #{} (have {}).",
                            n,
                            self.input_history.len()
                        ));
                    }
                } else {
                    let query = arg.to_lowercase();
                    let matches: Vec<(usize, &String)> = self
                        .input_history
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.to_lowercase().contains(&query))
                        .collect();
                    if matches.is_empty() {
                        self.push_system(format!("No history matches for `{}`.", arg));
                    } else {
                        let mut out = format!("History matches for `{}`:\n", arg);
                        for (i, entry) in matches.iter().take(20) {
                            let preview: String = entry.chars().take(100).collect();
                            out.push_str(&format!("  {:>3}  {}\n", i + 1, preview.replace('\n', " ⏎ ")));
                        }
                        out.push_str("\n/history <n> to edit an entry");
                        self.push_system(out);
                    }
                }
            }
            "/rename" => {
                if arg.is_empty() {
                    self.push_system(format!(
                        "Current title: {}\nUsage: /rename <title>",
                        self.session_title.clone().unwrap_or_else(|| "(untitled)".to_string())
                    ));
                } else {
                    self.session_title = Some(arg.to_string());
                    let _ = self.command_tx.send(AppCommand::SetTitle { title: arg.to_string() });
                    self.push_system(format!("Session renamed to \"{}\"", arg));
                }
            }
            "/clear" => {
                let banner = self.messages.first().cloned();
                self.messages.clear();
                if let Some(b) = banner {
                    if b.role == "system" {
                        self.messages.push(b);
                    }
                }
                self.push_system("Chat cleared (session kept). Use /new for a fresh session.");
            }
            "/context" => {
                match arg.to_lowercase().as_str() {
                    "on" | "off" | "toggle" => {
                        self.context_enabled = match arg.to_lowercase().as_str() {
                            "on" => true,
                            "off" => false,
                            _ => !self.context_enabled,
                        };
                        let prompt = self.rebuild_system_prompt();
                        let _ = self
                            .command_tx
                            .send(AppCommand::SetSystemPrompt { prompt });
                        self.push_system(format!(
                            "Project context inclusion: {}. System prompt rebuilt.",
                            if self.context_enabled { "on" } else { "off" }
                        ));
                    }
                    _ => {
                        let files = discover_context_files();
                        let cmds = discover_agent_commands();
                        let mut out = format!(
                            "Context inclusion: {} (/context on|off to toggle)\n\nLoaded context:\n",
                            if self.context_enabled { "on" } else { "off" }
                        );
                        if files.is_empty() {
                            out.push_str("  (no AGENTS.md / CLAUDE.md)\n");
                        } else {
                            for f in &files {
                                out.push_str(&format!(
                                    "  {} ({} chars)\n",
                                    f.path.display(),
                                    f.content.len()
                                ));
                            }
                        }
                        out.push_str("Commands (.rs-agent/commands):\n");
                        if cmds.is_empty() {
                            out.push_str("  (none)\n");
                        } else {
                            for c in &cmds {
                                out.push_str(&format!(
                                    "  {} ({} chars)\n",
                                    c.path.display(),
                                    c.content.len()
                                ));
                            }
                        }
                        self.push_system(out);
                    }
                }
            }
            "/commands" => {
                let cmds = discover_agent_commands();
                if cmds.is_empty() {
                    self.push_system("No project commands in .rs-agent/commands/");
                } else {
                    let mut out = String::from("Project commands:\n");
                    for c in &cmds {
                        let name = c
                            .path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("?");
                        out.push_str(&format!("  /{} — {}\n", name, c.path.display()));
                    }
                    self.push_system(out);
                }
            }
            "/skills" => {
                let skills = discover_skills();
                self.push_system(list_skills_summary(&skills));
            }
            "/skill" => {
                if arg.is_empty() {
                    self.push_system("Usage: /skill <name>");
                } else {
                    let skills = discover_skills();
                    match find_skill(&skills, arg) {
                        Some(skill) => {
                            let injection = format_skill_injection(skill);
                            self.push_system(format!("Loaded skill `{}`", skill.name));
                            self.submit_user_text(format!(
                                "{}\n\nApply this skill to the current task. If no task was given yet, briefly confirm you loaded it and wait for instructions.",
                                injection
                            ));
                        }
                        None => self.push_system(format!(
                            "Unknown skill `{}`. Try /skills.",
                            arg
                        )),
                    }
                }
            }
            "/prompt" | "/p" => {
                let mut rest = arg.splitn(2, char::is_whitespace);
                let name = rest.next().unwrap_or("").trim();
                let args = rest.next().unwrap_or("").trim();
                if name.is_empty() {
                    let templates = discover_templates();
                    if templates.is_empty() {
                        self.push_system(
                            "No templates. Add markdown under ~/.rs-agent/prompts/ or .rs-agent/prompts/",
                        );
                    } else {
                        let mut out = String::from("Templates:\n");
                        for t in &templates {
                            out.push_str(&format!("  {} — {}\n", t.name, t.description));
                        }
                        self.push_system(out);
                    }
                } else {
                    let templates = discover_templates();
                    match find_template(&templates, name) {
                        Some(t) => {
                            self.input = render_template(t, args);
                            self.input_mode = InputMode::Insert;
                            self.status = format!("template `{}` in input — edit & Enter", t.name);
                        }
                        None => self.push_system(format!("Unknown template `{}`", name)),
                    }
                }
            }
            "/reload" => {
                let n = discover_skills().len();
                let t = discover_templates().len();
                let prompt = self.rebuild_system_prompt();
                let _ = self
                    .command_tx
                    .send(AppCommand::SetSystemPrompt { prompt });
                self.push_system(format!(
                    "Reloaded {} skill(s), {} template(s); system prompt rebuilt.",
                    n, t
                ));
            }
            "/mode" => {
                if arg.is_empty() {
                    self.push_system(format!(
                        "Current mode: {}\nUsage: /mode plan|ask|agent",
                        self.agent_mode.as_str()
                    ));
                } else if let Some(mode) = AgentMode::parse(arg) {
                    self.agent_mode = mode;
                    let _ = self.command_tx.send(AppCommand::SetMode { mode });
                    self.push_system(format!("Mode set to {}", mode.as_str()));
                } else {
                    self.push_system("Usage: /mode plan|ask|agent");
                }
            }
            "/sessions" => match SessionStore::new().list_summaries() {
                Ok(mut summaries) => {
                    if summaries.is_empty() {
                        self.push_system("No saved sessions.");
                    } else {
                        summaries.sort_by(|a, b| b.id.cmp(&a.id));
                        let mut out = String::from("Sessions (newest first):\n");
                        for s in summaries.iter().take(20) {
                            let title = s.title.clone().unwrap_or_else(|| "(untitled)".to_string());
                            out.push_str(&format!(
                                "  {}  {} msgs  {}  — {}\n",
                                s.id, s.message_count, s.model, title
                            ));
                        }
                        out.push_str("Resume: rs-agent -r <id>");
                        self.push_system(out);
                    }
                }
                Err(e) => self.push_system(format!("Failed to list sessions: {}", e)),
            },
            "/export" => {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                let dir = std::path::Path::new(&home).join(".rs-agent").join("exports");
                let _ = std::fs::create_dir_all(&dir);
                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                let path = dir.join(format!("export_{}.md", ts));
                let md = match SessionStore::new().load(&self.session_id) {
                    Ok(data) => session::export_markdown(&data),
                    Err(_) => {
                        // Session not on disk yet (no turn saved this run) — fall
                        // back to exporting the in-memory chat transcript.
                        let mut md = format!("# rs-agent export {}\n\n", self.session_id);
                        for m in &self.messages {
                            md.push_str(&format!("## {}\n\n{}\n\n", m.role, m.text));
                            if let Some(ref th) = m.thinking {
                                if !th.is_empty() {
                                    md.push_str(&format!("<details><summary>thinking</summary>\n\n{}\n\n</details>\n\n", th));
                                }
                            }
                        }
                        md
                    }
                };
                match std::fs::write(&path, md) {
                    Ok(()) => self.push_system(format!("Exported to {}", path.display())),
                    Err(e) => self.push_system(format!("Export failed: {}", e)),
                }
            }
            "/trust" => {
                let mut parts = arg.splitn(2, char::is_whitespace);
                let sub = parts.next().unwrap_or("").trim();
                match sub {
                    "list" | "" => {
                        let paths = self.trust_store.list();
                        if paths.is_empty() {
                            self.push_system("No trusted projects.");
                        } else {
                            let mut out = String::from("Trust store:\n");
                            for (p, trusted) in paths {
                                out.push_str(&format!(
                                    "  {} {}\n",
                                    if trusted { "✓" } else { "·" },
                                    p
                                ));
                            }
                            self.push_system(out);
                        }
                    }
                    "reset" | "clear" => {
                        self.trust_store.clear();
                        self.push_system("Trust store cleared.");
                    }
                    _ => self.push_system("Usage: /trust list|reset"),
                }
            }
            "/compact" => {
                let _ = self.command_tx.send(AppCommand::Compact);
                self.status = "compacting...".to_string();
            }
            "/new" => {
                let _ = self.command_tx.send(AppCommand::NewSession);
                self.session_id = SessionStore::generate_id();
                self.push_system(format!("New session: {}", self.session_id));
            }
            "/model" => {
                if arg.is_empty() {
                    let cfg = crate::config::Config::load();
                    let mut names = crate::ai::registry::available_model_displays();
                    // Aliases as bare names (resolve against current provider).
                    for alias in cfg.model_aliases.keys() {
                        if !names.iter().any(|n| n == alias) {
                            names.push(alias.clone());
                        }
                    }
                    let current = format!("{}/{}", self.provider_name, self.model_name);
                    if !names.iter().any(|n| n == &current || n == &self.model_name) {
                        names.insert(0, current);
                    }
                    names.sort();
                    names.dedup();
                    let n = names.len();
                    self.picker_models = names;
                    self.model_cycle = self.picker_models.clone();
                    self.picker_prefix = String::new();
                    self.picker_query = String::new();
                    self.picker_mode = PickerMode::Model;
                    self.picker_selection = 0;
                    self.update_picker_results();
                    self.picker_active = true;
                    self.spawn_model_fetch();
                    let ready = crate::ai::registry::available_model_refs().len();
                    self.push_system(format!(
                        "Current: {}/{}\nCatalog picker: {} models from {} ready provider(s).\n\
                         (Only providers with API keys are listed — export OPENROUTER_API_KEY etc. for more.\n\
                         Live fetch merging in…)",
                        self.provider_name, self.model_name, n, ready
                    ));
                } else {
                    match self.apply_model_selection(arg) {
                        Ok(msg) => self.push_system(msg),
                        Err(e) => self.push_system(e),
                    }
                }
            }
            "/provider" | "/login" => {
                if arg.is_empty() {
                    self.start_provider_picker();
                } else if arg.eq_ignore_ascii_case("list") {
                    let mut lines = vec![
                        format!("Current provider: {}", self.provider_name),
                        "Providers:".to_string(),
                    ];
                    lines.extend(crate::ai::registry::provider_status_lines());
                    self.push_system(lines.join("\n"));
                } else {
                    self.select_or_connect_provider(&arg.to_lowercase());
                }
            }
            "/tree" => {
                self.show_tree_panel = !self.show_tree_panel;
                let panel_note = if self.show_tree_panel { "shown" } else { "hidden" };
                if self.tree_breadcrumb != "idle" {
                    self.push_system(format!(
                        "Call tree panel {}.\nCall tree: {}\n(depth max {})",
                        panel_note, self.tree_breadcrumb, self.rlm_depth
                    ));
                } else {
                    let saved_summary = SessionStore::new()
                        .load(&self.session_id)
                        .ok()
                        .and_then(|data| data.call_tree)
                        .and_then(|v| serde_json::from_value::<crate::rlm::tree::CallTreeInner>(v).ok())
                        .map(|inner| inner.summary());
                    match saved_summary {
                        Some(summary) => {
                            self.tree_panel_text = summary.clone();
                            self.push_system(format!(
                                "Call tree panel {}.\nCall tree: idle (no active run)\nLast saved snapshot — {}\n(depth max {})",
                                panel_note, summary, self.rlm_depth
                            ))
                        }
                        None => self.push_system(format!(
                            "Call tree panel {}.\nCall tree: {}\n(depth max {})",
                            panel_note, self.tree_breadcrumb, self.rlm_depth
                        )),
                    }
                }
            }
            _ => {
                self.push_system(format!("Unknown command: {} (try /help)", cmd));
            }
        }
        true
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) {
        if self.key_matches("insert", key) {
            self.input_mode = InputMode::Insert;
            return;
        }
        if self.key_matches("quit", key) {
            self.should_exit = true;
            return;
        }
        if self.key_matches("toggle_thinking", key) {
            if let Some(msg) = self
                .messages
                .iter_mut()
                .rev()
                .find(|m| m.role == "assistant" && m.thinking.as_ref().is_some_and(|t| !t.is_empty()))
            {
                msg.show_thinking = !msg.show_thinking;
            }
            return;
        }
        if self.key_matches("expand_tool", key) {
            if let Some(msg) = self.messages.iter_mut().rev().find(|m| !m.tool_blocks.is_empty()) {
                if let Some(block) = msg.tool_blocks.last_mut() {
                    block.expanded = !block.expanded;
                }
            }
            return;
        }
        if self.key_matches("toggle_tree", key) {
            self.show_tree_panel = !self.show_tree_panel;
            return;
        }
        if self.key_matches("jump_bottom", key) {
            self.follow_bottom = true;
            return;
        }
        // @ / # also work from normal mode (enter insert + open picker).
        if let KeyCode::Char('@') = key.code {
            self.input_mode = InputMode::Insert;
            self.start_picker();
            return;
        }
        if let KeyCode::Char('#') = key.code {
            self.input_mode = InputMode::Insert;
            self.start_dir_picker();
            return;
        }
        match key.code {
            KeyCode::Up => {
                self.follow_bottom = false;
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            KeyCode::PageUp => {
                self.follow_bottom = false;
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
            }
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: crossterm::event::KeyEvent) {
        if self.picker_active {
            match key.code {
                KeyCode::Up => {
                    self.picker_selection = self.picker_selection.saturating_sub(1);
                }
                KeyCode::Down => {
                    let max = self.picker_results.len().saturating_sub(1);
                    self.picker_selection = self.picker_selection.saturating_add(1).min(max);
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if let Some(selected) = self.picker_results.get(self.picker_selection).cloned() {
                        match self.picker_mode {
                            PickerMode::File => {
                                self.input = format!("{}{} ", self.picker_prefix, selected);
                            }
                            PickerMode::Dir => {
                                self.input = format!("{}#{} ", self.picker_prefix, selected);
                            }
                            PickerMode::Skill | PickerMode::Prompt => {
                                self.input = format!("{}{} ", self.picker_prefix, selected);
                            }
                            PickerMode::Model => {
                                match self.apply_model_selection(&selected) {
                                    Ok(msg) => {
                                        self.input.clear();
                                        self.push_system(msg);
                                    }
                                    Err(e) => self.push_system(e),
                                }
                            }
                            PickerMode::Provider => {
                                let pname =
                                    crate::ai::registry::provider_from_picker_row(&selected);
                                self.select_or_connect_provider(&pname);
                            }
                        }
                    }
                    self.picker_active = false;
                }
                KeyCode::Esc => {
                    self.input = self.picker_prefix.clone();
                    self.picker_active = false;
                }
                KeyCode::Backspace => {
                    if !self.picker_query.is_empty() {
                        self.picker_query.pop();
                        self.update_picker_results();
                    } else {
                        self.input = self.picker_prefix.clone();
                        self.picker_active = false;
                    }
                }
                KeyCode::Char(c) => {
                    self.picker_query.push(c);
                    self.update_picker_results();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Enter => {
                if !self.input.trim().is_empty() {
                    let text = std::mem::take(&mut self.input);
                    self.history_index = None;
                    if self.handle_slash_command(&text) {
                        self.input.clear();
                        return;
                    }
                    self.submit_user_text(text);
                }
            }
            KeyCode::Char('@') => {
                self.start_picker();
            }
            KeyCode::Char('#') => {
                self.start_dir_picker();
            }
            KeyCode::Tab => {
                self.try_start_completion_picker();
            }
            KeyCode::Up => {
                self.history_up();
            }
            KeyCode::Down => {
                self.history_down();
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => {
                self.picker_active = false;
                self.input_mode = InputMode::Normal;
            }
            _ => {}
        }
    }

    /// Push a non-slash user message to the agent: records it in
    /// `input_history`, appends chat bubbles, and dispatches `Submit`.
    fn submit_user_text(&mut self, text: String) {
        if self.input_history.last().map(|s| s.as_str()) != Some(text.as_str()) {
            self.input_history.push(text.clone());
        }
        self.history_index = None;

        let expanded = Self::expand_dir_tokens(&text);

        self.input_mode = InputMode::Waiting;
        self.queued_steers = 0;

        self.messages.push(ChatMessage {
            role: "user".to_string(),
            text: expanded.clone(),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        });
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            text: String::new(),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        });

        self.follow_bottom = true;
        self.status = "thinking...".to_string();
        let _ = self.command_tx.send(AppCommand::Submit { text: expanded });
    }

    /// Expands any `#somedir` tokens in `text` into a short, non-recursive
    /// directory listing wrapped in a `<dir>` block, so the agent gets a
    /// quick summary without a separate tool call. Tokens that aren't
    /// readable directories are left untouched.
    fn expand_dir_tokens(text: &str) -> String {
        let re = regex::Regex::new(r"#(\S+)").expect("static regex is valid");
        re.replace_all(text, |caps: &regex::Captures| {
            let path = &caps[1];
            Self::summarize_dir(path).unwrap_or_else(|| format!("#{}", path))
        })
        .to_string()
    }

    /// Reads up to `MAX_ENTRIES` non-recursive directory entries at `path`
    /// and renders them as a `<dir path="...">...</dir>` block. Returns
    /// `None` if `path` isn't a readable directory.
    fn summarize_dir(path: &str) -> Option<String> {
        const MAX_ENTRIES: usize = 40;
        let read_dir = std::fs::read_dir(path).ok()?;
        let mut names: Vec<String> = read_dir
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    format!("{}/", name)
                } else {
                    name
                }
            })
            .collect();
        names.sort();
        let truncated = names.len() > MAX_ENTRIES;
        names.truncate(MAX_ENTRIES);

        let mut out = format!("<dir path=\"{}\">\n", path);
        for name in &names {
            out.push_str(name);
            out.push('\n');
        }
        if truncated {
            out.push_str("...\n");
        }
        out.push_str("</dir>");
        Some(out)
    }

    /// Cycle backwards through `input_history` (older entries), only when the
    /// input is empty or already mid-navigation.
    fn history_up(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        if !self.input.is_empty() && self.history_index.is_none() {
            return;
        }
        let new_idx = match self.history_index {
            None => self.input_history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_index = Some(new_idx);
        self.input = self.input_history[new_idx].clone();
    }

    /// Cycle forwards through `input_history` (newer entries), clearing the
    /// input once past the most recent entry.
    fn history_down(&mut self) {
        match self.history_index {
            Some(i) if i + 1 < self.input_history.len() => {
                self.history_index = Some(i + 1);
                self.input = self.input_history[i + 1].clone();
            }
            Some(_) => {
                self.history_index = None;
                self.input.clear();
            }
            None => {}
        }
    }

    fn auto_allow(&self, tool_name: &str) -> bool {
        if !self.auto_mode {
            return false;
        }
        matches!(tool_name, "read" | "grep" | "ls" | "find" | "webfetch" | "websearch")
    }

    /// Rebuilds the system prompt from scratch (default prompt + optional
    /// project context sections), used by `/reload` and `/context on|off`.
    /// Custom `--system-prompt` / `--append-system-prompt` CLI overrides are
    /// intentionally not re-applied here — a full rebuild only restores the
    /// default prompt plus whatever context toggles are currently active.
    fn rebuild_system_prompt(&self) -> String {
        let mut sp = crate::agent::default_system_prompt();
        if self.context_enabled {
            let files = discover_context_files();
            sp.push_str(&build_context_section(&files));
            let cmds = discover_agent_commands();
            sp.push_str(&build_commands_section(&cmds));
        }
        sp
    }

    /// True if `key` matches the configured binding for `action`, trying
    /// both the exact case and the opposite case of a single-char binding
    /// (so e.g. a binding of "a" also matches Shift-A).
    fn key_matches(&self, action: &str, key: KeyEvent) -> bool {
        if self.keys.matches(action, key) {
            return true;
        }
        if let KeyCode::Char(c) = key.code {
            let toggled = if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            };
            if toggled != c {
                let toggled_key = KeyEvent::new(KeyCode::Char(toggled), key.modifiers);
                return self.keys.matches(action, toggled_key);
            }
        }
        false
    }

    /// Kicks off a background fetch of models from **all** providers with
    /// configured auth; results are `provider/model` strings.
    fn spawn_model_fetch(&mut self) {
        let (tx, rx) = channel::unbounded::<Vec<String>>();
        self.model_picker_rx = Some(rx);
        let timeout = self.timeout_secs;
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            let list = rt.block_on(crate::ai::registry::fetch_all_model_displays(timeout));
            let _ = tx.send(list);
        });
    }

    /// Apply a model selection string: alias, bare model, or `provider/model`.
    fn apply_model_selection(&mut self, raw: &str) -> Result<String, String> {
        let cfg = crate::config::Config::load();
        let resolved_arg = cfg.resolve_model_alias(raw);
        let mref = crate::ai::registry::ModelRef::parse(&resolved_arg, &self.provider_name);

        if !crate::ai::registry::is_known_provider(&mref.provider) {
            return Err(format!("Unknown provider `{}`", mref.provider));
        }
        if !crate::ai::registry::has_configured_auth(&mref.provider) {
            return Err(format!(
                "Provider `{}` has no credentials (export {}).",
                mref.provider,
                crate::ai::registry::api_key_env_for(&mref.provider)
            ));
        }

        let same_provider = mref.provider.eq_ignore_ascii_case(&self.provider_name);
        if same_provider {
            self.model_name = mref.model.clone();
            let _ = self.command_tx.send(AppCommand::SetModel {
                model: mref.model.clone(),
            });
            self.token_limit = crate::ai::token_count::get_context_limit(&mref.model);
            self.note_cycle_entry(&mref.display());
            Self::remember_selection(&mref.provider, &mref.model);
            if resolved_arg != raw {
                return Ok(format!(
                    "Model set to {}/{} (alias `{}`)",
                    mref.provider, mref.model, raw
                ));
            }
            return Ok(format!("Model set to {}/{}", mref.provider, mref.model));
        }

        if mref.provider.eq_ignore_ascii_case("opencode-cli")
            && std::env::var("OPENCODE_API_KEY").is_err()
        {
            std::env::set_var("OPENCODE_API_KEY", "cli-mode-no-key-needed");
        }
        if mref.provider.eq_ignore_ascii_case("bedrock")
            && std::env::var("AWS_ACCESS_KEY_ID").is_err()
        {
            crate::ai::bedrock::export_credentials_from_file();
        }

        let provider = crate::ai::registry::create_provider(
            &mref.provider,
            crate::ai::registry::CreateProviderOpts {
                default_model: Some(mref.model.clone()),
                timeout_secs: self.timeout_secs,
                ..Default::default()
            },
        )?;
        self.provider = provider.clone();
        self.provider_name = mref.provider.clone();
        self.model_name = mref.model.clone();
        self.token_limit = crate::ai::token_count::get_context_limit(&mref.model);
        let _ = self.command_tx.send(AppCommand::SetProvider {
            provider,
            model: mref.model.clone(),
        });
        self.note_cycle_entry(&mref.display());
        Self::remember_selection(&mref.provider, &mref.model);
        Ok(format!(
            "Switched to {}/{} (provider + model)",
            mref.provider, mref.model
        ))
    }

    /// Best-effort write last provider/model to ~/.rs-agent/config.toml.
    fn remember_selection(provider: &str, model: &str) {
        if let Err(e) = crate::config::Config::persist_last_selection(provider, model) {
            eprintln!("Warning: could not remember provider/model: {e}");
        }
    }

    fn note_cycle_entry(&mut self, display: &str) {
        if !self.model_cycle.iter().any(|c| c == display) {
            self.model_cycle.push(display.to_string());
        }
        if let Some(i) = self.model_cycle.iter().position(|c| c == display) {
            self.model_cycle_index = i;
        }
    }

    /// Cycle to the next ready provider/model (pi Ctrl+P).
    fn cycle_model(&mut self) {
        if self.model_cycle.is_empty() {
            self.model_cycle = crate::ai::registry::available_model_refs()
                .into_iter()
                .map(|r| r.display())
                .collect();
        }
        if self.model_cycle.is_empty() {
            self.push_system("No providers with credentials to cycle.");
            return;
        }
        self.model_cycle_index = (self.model_cycle_index + 1) % self.model_cycle.len();
        let next = self.model_cycle[self.model_cycle_index].clone();
        match self.apply_model_selection(&next) {
            Ok(msg) => self.push_system(format!("Cycled — {}", msg)),
            Err(e) => self.push_system(e),
        }
    }

    /// Drains a completed model fetch (if any) into `picker_models`, and
    /// refreshes the active picker's results if it's currently showing models.
    fn poll_model_picker(&mut self) {
        let Some(rx) = &self.model_picker_rx else {
            return;
        };
        let Ok(list) = rx.try_recv() else {
            return;
        };
        for m in list {
            if !self.picker_models.contains(&m) {
                self.picker_models.push(m.clone());
            }
            if !self.model_cycle.contains(&m) {
                self.model_cycle.push(m);
            }
        }
        self.picker_models.sort();
        if self.picker_active && self.picker_mode == PickerMode::Model {
            self.update_picker_results();
        }
        self.model_picker_rx = None;
    }

    /// If the input is `/skill <partial>` or `/prompt|/p <partial>` (no
    /// trailing args yet), either completes the single matching name in
    /// place or opens a picker over the matches. No-op otherwise.
    fn try_start_completion_picker(&mut self) {
        let text = self.input.clone();
        let candidates: [(&str, bool); 3] =
            [("/skill ", true), ("/prompt ", false), ("/p ", false)];
        for (cmd, is_skill) in candidates {
            let Some(rest) = text.strip_prefix(cmd) else {
                continue;
            };
            if rest.contains(char::is_whitespace) {
                continue;
            }
            let prefix = cmd.to_string();
            let query = rest.to_string();
            let names: Vec<String> = if is_skill {
                discover_skills().into_iter().map(|s| s.name).collect()
            } else {
                discover_templates().into_iter().map(|t| t.name).collect()
            };
            let query_lower = query.to_lowercase();
            let filtered: Vec<String> = if query_lower.is_empty() {
                names
            } else {
                Self::rank_and_filter(&names, &query_lower, 50)
            };
            match filtered.len() {
                0 => {}
                1 => {
                    self.input = format!("{}{} ", prefix, filtered[0]);
                }
                _ => {
                    self.picker_prefix = prefix;
                    self.picker_query = query;
                    self.picker_mode = if is_skill { PickerMode::Skill } else { PickerMode::Prompt };
                    self.picker_results = filtered;
                    self.picker_selection = 0;
                    self.picker_active = true;
                }
            }
            return;
        }
    }

    const PICKER_SKIP_DIRS: [&'static str; 4] = ["target", "node_modules", ".git", "__pycache__"];
    const PICKER_RESULT_CAP: usize = 50;

    fn start_picker(&mut self) {
        self.picker_prefix = self.input.clone();
        self.picker_query = String::new();
        self.picker_selection = 0;
        self.picker_mode = PickerMode::File;
        // Always rescan — cwd may have changed / first open may have raced.
        self.picker_files_loaded = false;
        self.picker_active = true;
        self.update_picker_results();
        self.status = format!(
            "file picker: {} file(s) — type to filter, Enter to insert path",
            self.picker_files.len()
        );
    }

    fn start_dir_picker(&mut self) {
        self.picker_prefix = self.input.clone();
        self.picker_query = String::new();
        self.picker_selection = 0;
        self.picker_mode = PickerMode::Dir;
        self.picker_dirs_loaded = false;
        self.picker_active = true;
        self.update_picker_results();
        self.status = format!(
            "dir picker: {} dir(s) — type to filter, Enter to attach #path",
            self.picker_dirs.len()
        );
    }

    fn start_provider_picker(&mut self) {
        self.picker_prefix = String::new();
        self.picker_query = String::new();
        self.picker_selection = 0;
        self.picker_providers = crate::ai::registry::provider_picker_rows();
        // Put current provider near the top if present.
        if let Some(idx) = self
            .picker_providers
            .iter()
            .position(|r| r.starts_with(&format!("{} ", self.provider_name)))
        {
            let cur = self.picker_providers.remove(idx);
            self.picker_providers.insert(0, cur);
        }
        self.picker_mode = PickerMode::Provider;
        self.picker_active = true;
        self.input_mode = InputMode::Insert;
        self.update_picker_results();
        self.push_system(format!(
            "Provider picker — current: {}.\n\
             Select a [ready] provider to switch, or a [needs key] row to open its signup page and paste an API key.\n\
             Keys are saved to ~/.rs-agent/secrets.toml",
            self.provider_name
        ));
        self.status = format!(
            "provider picker: {} — Enter to select / connect",
            self.picker_providers.len()
        );
    }

    /// Switch to a ready provider, or start the connect/key-paste flow.
    fn select_or_connect_provider(&mut self, pname: &str) {
        if !crate::ai::registry::is_known_provider(pname)
            && !crate::ai::registry::supports_runtime(pname)
        {
            self.push_system(format!("Unknown provider `{pname}`"));
            return;
        }
        if !crate::ai::registry::supports_runtime(pname) {
            self.push_system(format!(
                "Provider `{pname}` is in the catalog but not runnable yet."
            ));
            return;
        }
        // Reload secrets into env in case they were just written.
        crate::config::export_secrets_to_env();
        if crate::ai::registry::has_configured_auth(pname) {
            let model = if self.provider_name.eq_ignore_ascii_case(pname) {
                self.model_name.clone()
            } else {
                crate::ai::registry::default_model_for(pname)
            };
            match self.apply_model_selection(&format!("{pname}/{model}")) {
                Ok(msg) => self.push_system(msg),
                Err(e) => self.push_system(e),
            }
            return;
        }
        // Needs a key: open browser + enter capture mode.
        let env = crate::ai::registry::api_key_env_for(pname);
        match crate::ai::registry::open_provider_connect_url(pname) {
            Ok(url) => self.push_system(format!(
                "Opening {url}\nCreate an API key, then paste it below and press Enter.\n\
                 (Or Esc to cancel. Stored as {env} in ~/.rs-agent/secrets.toml)"
            )),
            Err(e) => self.push_system(format!(
                "{e}\nPaste your API key for `{pname}` ({env}) and press Enter.\nEsc to cancel."
            )),
        }
        self.pending_api_key_provider = Some(pname.to_string());
        self.input.clear();
        self.input_mode = InputMode::ApiKey;
        self.picker_active = false;
        self.status = format!("paste API key for {pname} · Enter save · Esc cancel");
    }

    fn handle_api_key_input(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.pending_api_key_provider = None;
                self.input.clear();
                self.input_mode = InputMode::Insert;
                self.status = "cancelled API key entry".to_string();
            }
            KeyCode::Enter => {
                let Some(pname) = self.pending_api_key_provider.take() else {
                    self.input_mode = InputMode::Insert;
                    return;
                };
                let key_text = std::mem::take(&mut self.input);
                match crate::config::store_api_key(&pname, &key_text) {
                    Ok(()) => {
                        self.input_mode = InputMode::Insert;
                        self.push_system(format!(
                            "Saved API key for `{pname}` → {} (also exported to env).",
                            crate::ai::registry::api_key_env_for(&pname)
                        ));
                        let model = crate::ai::registry::default_model_for(&pname);
                        match self.apply_model_selection(&format!("{pname}/{model}")) {
                            Ok(msg) => {
                                self.push_system(msg);
                                self.status = format!("connected {pname}");
                            }
                            Err(e) => {
                                self.push_system(format!(
                                    "Key saved, but switch failed: {e}. Try /model."
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        self.pending_api_key_provider = Some(pname);
                        self.push_system(format!("Could not save key: {e}"));
                        self.input_mode = InputMode::ApiKey;
                    }
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn update_picker_results(&mut self) {
        match self.picker_mode {
            PickerMode::File => {
                if !self.picker_files_loaded {
                    self.load_picker_files();
                }
                self.picker_results =
                    Self::rank_and_filter(&self.picker_files, &self.picker_query, Self::PICKER_RESULT_CAP);
            }
            PickerMode::Dir => {
                if !self.picker_dirs_loaded {
                    self.load_picker_dirs();
                }
                self.picker_results =
                    Self::rank_and_filter(&self.picker_dirs, &self.picker_query, Self::PICKER_RESULT_CAP);
            }
            PickerMode::Skill => {
                let names: Vec<String> = discover_skills().into_iter().map(|s| s.name).collect();
                self.picker_results =
                    Self::rank_and_filter(&names, &self.picker_query, Self::PICKER_RESULT_CAP);
            }
            PickerMode::Prompt => {
                let names: Vec<String> =
                    discover_templates().into_iter().map(|t| t.name).collect();
                self.picker_results =
                    Self::rank_and_filter(&names, &self.picker_query, Self::PICKER_RESULT_CAP);
            }
            PickerMode::Model => {
                self.picker_results = Self::rank_and_filter(
                    &self.picker_models,
                    &self.picker_query,
                    Self::PICKER_RESULT_CAP,
                );
            }
            PickerMode::Provider => {
                self.picker_results = Self::rank_and_filter(
                    &self.picker_providers,
                    &self.picker_query,
                    Self::PICKER_RESULT_CAP,
                );
            }
        }
        self.picker_selection = self
            .picker_selection
            .min(self.picker_results.len().saturating_sub(1));
    }

    /// Ranks `items` against `query` with fuzzy (subsequence) matching.
    /// Spaces split the query into tokens that must all match. Lower score
    /// is better: exact / prefix / contiguous / fuzzy. Caps at `cap`.
    fn rank_and_filter(items: &[String], query: &str, cap: usize) -> Vec<String> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            let mut out: Vec<String> = items.iter().cloned().collect();
            out.truncate(cap);
            return out;
        }
        let tokens: Vec<&str> = query.split_whitespace().filter(|t| !t.is_empty()).collect();
        let mut scored: Vec<(i32, usize, &String)> = items
            .iter()
            .filter_map(|item| {
                let lower = item.to_lowercase();
                let mut total = 0i32;
                for tok in &tokens {
                    let Some(s) = Self::fuzzy_score(&lower, tok) else {
                        return None;
                    };
                    total += s;
                }
                Some((total, item.len(), item))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
        scored
            .into_iter()
            .take(cap)
            .map(|(_, _, s)| s.clone())
            .collect()
    }

    /// Fuzzy score for `haystack` against `needle` (both already lowercased).
    /// `None` if needle chars are not a subsequence of haystack.
    /// Lower is better.
    fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
        if needle.is_empty() {
            return Some(0);
        }
        if haystack == needle {
            return Some(0);
        }
        // Prefer matches on the final path segment (model id).
        let base = haystack.rsplit('/').next().unwrap_or(haystack);
        if let Some(s) = Self::fuzzy_score_one(base, needle) {
            return Some(s);
        }
        Self::fuzzy_score_one(haystack, needle).map(|s| s + 20)
    }

    fn fuzzy_score_one(haystack: &str, needle: &str) -> Option<i32> {
        if haystack == needle {
            return Some(0);
        }
        if haystack.starts_with(needle) {
            return Some(1);
        }
        if let Some(pos) = haystack.find(needle) {
            // Contiguous substring: earlier + word-ish boundary is better.
            let mut score = 10 + pos as i32;
            if pos > 0 {
                let prev = haystack.as_bytes()[pos - 1] as char;
                if matches!(prev, '/' | '-' | '_' | '.' | ' ') {
                    score -= 5;
                }
            }
            return Some(score);
        }

        // Subsequence: every needle char appears in order in haystack.
        let h: Vec<char> = haystack.chars().collect();
        let n: Vec<char> = needle.chars().collect();
        let mut hi = 0usize;
        let mut score = 40i32;
        let mut prev_match = None::<usize>;
        let mut gaps = 0i32;
        for &nc in &n {
            let mut found = false;
            while hi < h.len() {
                if h[hi] == nc {
                    if let Some(p) = prev_match {
                        let gap = (hi - p) as i32 - 1;
                        gaps += gap;
                        // Bonus for consecutive matches.
                        if gap == 0 {
                            score -= 2;
                        }
                    } else if hi == 0 || matches!(h[hi.saturating_sub(1)], '/' | '-' | '_' | '.' | ' ')
                    {
                        score -= 3; // start-of-token bonus
                    }
                    prev_match = Some(hi);
                    hi += 1;
                    found = true;
                    break;
                }
                hi += 1;
            }
            if !found {
                return None;
            }
        }
        Some(score + gaps)
    }

    /// Builds a rough gitignore-style [`GlobSet`] from a hardcoded list of
    /// common noisy directories plus a best-effort parse of `.gitignore` at
    /// `cwd` (negations and complex patterns are not supported).
    fn build_ignore_globset(cwd: &std::path::Path) -> GlobSet {
        let mut builder = GlobSetBuilder::new();
        for pat in ["**/target/**", "**/node_modules/**", "**/.git/**", "**/__pycache__/**", "**/*.o"] {
            if let Ok(g) = Glob::new(pat) {
                builder.add(g);
            }
        }
        if let Ok(content) = std::fs::read_to_string(cwd.join(".gitignore")) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                    continue;
                }
                let mut pat = line.trim_end_matches('/').to_string();
                if let Some(stripped) = pat.strip_prefix('/') {
                    pat = stripped.to_string();
                } else if !pat.contains('/') {
                    pat = format!("**/{}", pat);
                }
                for variant in [pat.clone(), format!("{}/**", pat)] {
                    if let Ok(g) = Glob::new(&variant) {
                        builder.add(g);
                    }
                }
            }
        }
        builder.build().unwrap_or_else(|_| GlobSetBuilder::new().build().unwrap())
    }

    fn picker_filter_entry(entry: &walkdir::DirEntry) -> bool {
        let name = entry.file_name().to_string_lossy();
        if entry.depth() > 0 && name.starts_with('.') {
            return false;
        }
        if entry.file_type().is_dir() && Self::PICKER_SKIP_DIRS.contains(&name.as_ref()) {
            return false;
        }
        true
    }

    fn load_picker_files(&mut self) {
        self.picker_files.clear();
        let cwd = std::env::current_dir().unwrap_or_default();
        let ignore_set = Self::build_ignore_globset(&cwd);
        const MAX_FILES: usize = 8_000;
        const MAX_DEPTH: usize = 12;
        for entry in WalkDir::new(&cwd)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(Self::picker_filter_entry)
        {
            if self.picker_files.len() >= MAX_FILES {
                break;
            }
            if let Ok(entry) = entry {
                if entry.file_type().is_file() {
                    if let Ok(relative) = entry.path().strip_prefix(&cwd) {
                        let path = relative.to_string_lossy().replace('\\', "/");
                        if path.is_empty() || ignore_set.is_match(path.as_str()) {
                            continue;
                        }
                        self.picker_files.push(path);
                    }
                }
            }
        }
        self.picker_files.sort();
        self.picker_files_loaded = true;
    }

    fn load_picker_dirs(&mut self) {
        self.picker_dirs.clear();
        let cwd = std::env::current_dir().unwrap_or_default();
        let ignore_set = Self::build_ignore_globset(&cwd);
        const MAX_DIRS: usize = 4_000;
        const MAX_DEPTH: usize = 12;
        for entry in WalkDir::new(&cwd)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(Self::picker_filter_entry)
        {
            if self.picker_dirs.len() >= MAX_DIRS {
                break;
            }
            if let Ok(entry) = entry {
                if entry.file_type().is_dir() && entry.depth() > 0 {
                    if let Ok(relative) = entry.path().strip_prefix(&cwd) {
                        let path = relative.to_string_lossy().replace('\\', "/");
                        if path.is_empty()
                            || ignore_set.is_match(path.as_str())
                            || ignore_set.is_match(format!("{}/", path).as_str())
                        {
                            continue;
                        }
                        self.picker_dirs.push(path);
                    }
                }
            }
        }
        self.picker_dirs.sort();
        self.picker_dirs_loaded = true;
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let mut constraints = vec![Constraint::Min(3)];
        if self.show_repl_panel {
            constraints.push(Constraint::Length(6));
        }
        constraints.push(Constraint::Length(3));
        constraints.push(Constraint::Length(1));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let mut idx = 0;
        let chat_area = chunks[idx];
        idx += 1;
        let repl_area = if self.show_repl_panel {
            let a = chunks[idx];
            idx += 1;
            Some(a)
        } else {
            None
        };
        let input_area = chunks[idx];
        idx += 1;
        let status_area = chunks[idx];

        if self.show_tree_panel {
            let hchunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
                .split(chat_area);
            self.render_messages(frame, hchunks[0]);
            self.render_tree_panel(frame, hchunks[1]);
        } else {
            self.render_messages(frame, chat_area);
        }
        if let Some(repl_area) = repl_area {
            self.render_repl_panel(frame, repl_area);
        }
        self.render_input(frame, input_area);
        self.render_status(frame, status_area);
        if self.picker_active {
            self.render_picker(frame, area);
        }
        if self.pending_permission.is_some() {
            self.render_permission_prompt(frame, area);
        }
    }

    fn render_tree_panel(&mut self, frame: &mut Frame, area: Rect) {
        let style = Style::default().fg(self.palette.tool);
        let lines: Vec<Line> = self
            .tree_panel_text
            .lines()
            .map(|l| Line::from(Span::styled(l.to_string(), style)))
            .collect();
        let panel = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Call Tree ")
                .border_style(style),
        );
        frame.render_widget(panel, area);
    }

    fn render_repl_panel(&mut self, frame: &mut Frame, area: Rect) {
        let style = Style::default().fg(self.palette.muted);
        let visible_rows = (area.height as usize).saturating_sub(2).max(1);
        let all_lines: Vec<&str> = self.repl_panel.lines().collect();
        let start = all_lines.len().saturating_sub(visible_rows);
        let lines: Vec<Line> = all_lines[start..]
            .iter()
            .map(|l| Line::from(Span::styled(l.to_string(), style)))
            .collect();
        let panel = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" REPL ")
                .border_style(Style::default().fg(self.palette.tool)),
        );
        frame.render_widget(panel, area);
    }

    fn wrap_line<'a>(line: &Line<'a>, max_width: usize) -> Vec<Line<'a>> {
        let text_len: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        if text_len <= max_width {
            return vec![line.clone()];
        }
        let mut result = Vec::new();
        let mut current_spans: Vec<Span> = Vec::new();
        let mut current_chars = 0usize;
        for span in &line.spans {
            let span_chars: Vec<char> = span.content.chars().collect();
            let remaining = max_width.saturating_sub(current_chars);
            if span_chars.len() <= remaining {
                current_spans.push(span.clone());
                current_chars += span_chars.len();
            } else {
                let mut i = 0;
                while i < span_chars.len() {
                    let take = max_width.saturating_sub(current_chars).min(span_chars.len() - i);
                    let segment: String = span_chars[i..i + take].iter().collect();
                    current_spans.push(Span::styled(segment, span.style.clone()));
                    current_chars += take;
                    i += take;
                    if current_chars >= max_width {
                        result.push(Line::from(std::mem::take(&mut current_spans)));
                        current_chars = 0;
                    }
                }
            }
        }
        if !current_spans.is_empty() {
            result.push(Line::from(current_spans));
        }
        result
    }

    fn render_messages(&mut self, frame: &mut Frame, area: Rect) {
        self.chat_area_y = area.y;
        let max_width = (area.width as usize).saturating_sub(2).max(20);
        let mut lines: Vec<Line> = Vec::new();
        self.thinking_targets.clear();
        self.tool_targets.clear();
        for (msg_idx, msg) in self.messages.iter().enumerate() {
            let (prefix, color) = match msg.role.as_str() {
                "system" => ("◆ ", self.palette.system),
                "user" => ("▶ ", self.palette.user),
                "assistant" => ("▸ ", self.palette.assistant),
                "tool" => ("⚙ ", self.palette.tool),
                _ => ("  ", self.palette.text),
            };
            let bold_prefix = Style::default().fg(color).add_modifier(Modifier::BOLD);

            let rendered = render_markdown(&msg.text, self.theme_name.syntect_theme());
            for (i, raw_line) in rendered.into_iter().enumerate() {
                let mut line = raw_line;
                if i == 0 {
                    line.spans.insert(0, Span::styled(prefix, bold_prefix));
                }
                lines.extend(Self::wrap_line(&line, max_width));
            }

            if let Some(ref thinking) = msg.thinking {
                if !thinking.is_empty() {
                    if msg.show_thinking {
                        let think_style = Style::default()
                            .fg(self.palette.muted)
                            .add_modifier(Modifier::ITALIC | Modifier::UNDERLINED);
                        let line_idx = lines.len();
                        self.thinking_targets.push((line_idx, msg_idx));
                        lines.push(Line::from(Span::styled(
                            "🧠 thinking (click to hide)",
                            think_style,
                        )));
                        let body_style = Style::default().fg(self.palette.muted).add_modifier(Modifier::ITALIC);
                        for raw_line in render_markdown(thinking, self.theme_name.syntect_theme()) {
                            let mut styled_spans = Vec::new();
                            for span in raw_line.spans {
                                styled_spans.push(Span::styled(span.content, body_style));
                            }
                            lines.extend(Self::wrap_line(&Line::from(styled_spans), max_width));
                        }
                    } else {
                        let clickable = Style::default().fg(self.palette.muted).add_modifier(Modifier::UNDERLINED);
                        let line_idx = lines.len();
                        self.thinking_targets.push((line_idx, msg_idx));
                        lines.push(Line::from(vec![Span::styled(
                            format!("💭 {}", thinking.chars().take(60).collect::<String>()),
                            clickable,
                        )]));
                    }
                }
            }

            for (tool_idx, block) in msg.tool_blocks.iter().enumerate() {
                let icon = if block.is_error { "⚠" } else { "⚙" };
                let color = if block.is_error { self.palette.danger } else { self.palette.tool };
                if block.expanded {
                    let header_style = Style::default().fg(color).add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                    let line_idx = lines.len();
                    self.tool_targets.push((line_idx, msg_idx, tool_idx));
                    lines.push(Line::from(Span::styled(
                        format!("{} {} (click/e to collapse)", icon, block.name),
                        header_style,
                    )));
                    let body_style = Style::default().fg(if block.is_error { self.palette.danger } else { self.palette.muted });
                    for raw_line in render_markdown(&block.full, self.theme_name.syntect_theme()) {
                        let mut styled_spans = Vec::new();
                        for span in raw_line.spans {
                            styled_spans.push(Span::styled(span.content, body_style));
                        }
                        lines.extend(Self::wrap_line(&Line::from(styled_spans), max_width));
                    }
                } else {
                    let clickable = Style::default().fg(color).add_modifier(Modifier::UNDERLINED);
                    let line_idx = lines.len();
                    self.tool_targets.push((line_idx, msg_idx, tool_idx));
                    lines.push(Line::from(vec![Span::styled(
                        format!("{} {} — {}… (click/e to expand)", icon, block.name, block.preview),
                        clickable,
                    )]));
                }
            }

            lines.push(Line::from(""));
        }

        let inner_height = (area.height as usize).saturating_sub(1);
        let total = lines.len();
        if self.follow_bottom || self.scroll_offset + inner_height > total {
            self.scroll_offset = total.saturating_sub(inner_height);
        }
        let start = self.scroll_offset.min(total.saturating_sub(inner_height));
        let visible_lines: Vec<Line> = lines.into_iter().skip(start).collect();

        let chat = Paragraph::new(visible_lines)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(" Chat ")
                    .title_alignment(ratatui::layout::Alignment::Center),
            );

        frame.render_widget(chat, area);
    }

    fn render_input(&mut self, frame: &mut Frame, area: Rect) {
        let mode_indicator = match self.input_mode {
            InputMode::Normal => " NORMAL ",
            InputMode::Insert => " INSERT ",
            InputMode::Waiting => " WAITING ",
            InputMode::ApiKey => " API KEY ",
        };

        let border_style = match self.input_mode {
            InputMode::Insert => Style::default().fg(self.palette.user),
            InputMode::Waiting | InputMode::ApiKey => Style::default().fg(self.palette.warn),
            _ => Style::default().fg(self.palette.border),
        };

        let display_text = if self.input_mode == InputMode::ApiKey {
            // Mask the key in the input bar (dots), keep length for cursor.
            "•".repeat(self.input.chars().count())
        } else if self.picker_active {
            match self.picker_mode {
                PickerMode::File => format!("{}@{}", self.picker_prefix, self.picker_query),
                PickerMode::Dir => format!("{}#{}", self.picker_prefix, self.picker_query),
                PickerMode::Skill
                | PickerMode::Prompt
                | PickerMode::Model
                | PickerMode::Provider => {
                    format!("{}{}", self.picker_prefix, self.picker_query)
                }
            }
        } else {
            self.input.clone()
        };

        let input = Paragraph::new(display_text.as_str())
            .style(match self.input_mode {
                InputMode::Waiting | InputMode::ApiKey => Style::default().fg(self.palette.warn),
                _ => Style::default().fg(self.palette.text),
            })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(mode_indicator)
                    .border_style(border_style),
            )
            ;

        frame.render_widget(input, area);

        if matches!(
            self.input_mode,
            InputMode::Insert | InputMode::Waiting | InputMode::ApiKey
        ) {
            let cursor_len = if self.picker_active {
                match self.picker_mode {
                    PickerMode::File | PickerMode::Dir => {
                        self.picker_prefix.len() + 1 + self.picker_query.len()
                    }
                    PickerMode::Skill
                    | PickerMode::Prompt
                    | PickerMode::Model
                    | PickerMode::Provider => {
                        self.picker_prefix.len() + self.picker_query.len()
                    }
                }
            } else {
                self.input.len()
            };
            let x = (cursor_len as u16 + 1).min(area.width.max(1).saturating_sub(2));
            frame.set_cursor_position(ratatui::layout::Position::new(area.x + x, area.y + 1));
        }
    }

    fn render_status(&mut self, frame: &mut Frame, area: Rect) {
        let status_color = if self.near_limit {
            self.palette.danger
        } else if self.status == "ready" {
            self.palette.ok
        } else {
            self.palette.warn
        };

        let token_str = if self.token_limit > 0 {
            let pct = self.token_used as f64 / self.token_limit as f64 * 100.0;
            if self.near_limit {
                format!(" ⚠ {:.0}%", pct)
            } else {
                format!(" {:.1}K/{}K", self.token_used as f64 / 1000.0, self.token_limit / 1000)
            }
        } else {
            String::new()
        };

        let mut hints = String::from(" ^C quit");
        if self.input_mode == InputMode::Waiting {
            hints.push_str(" · Esc abort · Enter steer");
        }
        let yolo = if self.approved || self.auto_mode {
            " YOLO"
        } else {
            ""
        };
        let title_str = self
            .session_title
            .as_ref()
            .map(|t| format!(" \"{}\"", t.chars().take(40).collect::<String>()))
            .unwrap_or_default();
        let meta = format!(
            " {}/{} · {} · d{}{} [{}]{} | {}",
            self.provider_name,
            self.model_name,
            self.agent_mode.as_str(),
            self.rlm_depth,
            yolo,
            self.session_id,
            title_str,
            self.tree_breadcrumb
        );

        let spinner = self
            .tool_in_progress
            .as_ref()
            .map(|(name, started)| {
                const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
                let elapsed = started.elapsed();
                let frame = FRAMES[(elapsed.as_millis() / 120) as usize % FRAMES.len()];
                format!(" {} {} ({:.1}s)", frame, name, elapsed.as_secs_f64())
            })
            .unwrap_or_default();

        let status = Line::from(vec![
            Span::styled(hints, Style::default().fg(self.palette.muted)),
            Span::styled(meta, Style::default().fg(self.palette.accent)),
            Span::styled(" | ", Style::default().fg(self.palette.muted)),
            Span::styled(&self.status, Style::default().fg(status_color)),
            Span::styled(
                spinner,
                Style::default().fg(self.palette.warn).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                token_str,
                Style::default().fg(if self.near_limit {
                    self.palette.danger
                } else {
                    self.palette.muted
                }),
            ),
        ]);
        frame.render_widget(Paragraph::new(status), area);
    }

    fn handle_permission_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(pending) = self.pending_permission.take() {
            if key.code == KeyCode::Enter || self.key_matches("perm_once", key) {
                let tool = pending.request.tool_name.clone();
                let _ = pending.reply_tx.send(PermissionReply::AllowOnce);
                self.status = format!("allowed {} (once)", tool);
            } else if self.key_matches("perm_always", key) {
                let tool = pending.request.tool_name.clone();
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.trust_store.set_trusted(&cwd, true);
                let _ = pending.reply_tx.send(PermissionReply::AllowAlways);
                self.status = format!("trusted project, allowed {}", tool);
            } else if key.code == KeyCode::Esc || self.key_matches("perm_deny", key) {
                let _ = pending.reply_tx.send(PermissionReply::Deny);
                self.status = "denied".to_string();
            } else {
                self.pending_permission = Some(pending);
            }
        }
    }

    fn render_picker(&mut self, frame: &mut Frame, area: Rect) {
        let empty = self.picker_results.is_empty();
        let picker_height = if empty {
            1
        } else {
            (self.picker_results.len() as u16).min(10).max(1)
        };
        let picker_y = area.height.saturating_sub(4 + picker_height + 1);
        let picker_area = Rect {
            x: area.x + 1,
            y: area.y + picker_y,
            width: area.width.saturating_sub(2).min(if self.picker_mode == PickerMode::Provider {
                96
            } else {
                72
            }),
            height: picker_height + 2,
        };

        let items: Vec<ListItem> = if empty {
            let hint = match self.picker_mode {
                PickerMode::File => "(no files found — check cwd / .gitignore)",
                PickerMode::Dir => "(no directories found)",
                PickerMode::Model => "(no models — export API keys, or wait for fetch)",
                PickerMode::Provider => "(no providers — catalog empty?)",
                PickerMode::Skill => "(no skills found)",
                PickerMode::Prompt => "(no templates found)",
            };
            vec![ListItem::new(hint).style(Style::default().fg(self.palette.muted))]
        } else {
            self.picker_results
                .iter()
                .enumerate()
                .map(|(i, path)| {
                    let style = if i == self.picker_selection {
                        Style::default()
                            .fg(self.palette.highlight_fg)
                            .bg(self.palette.highlight_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.palette.accent)
                    };
                    ListItem::new(path.as_str()).style(style)
                })
                .collect()
        };

        let title = match self.picker_mode {
            PickerMode::File => " Files (@)  type to filter · Enter select · Esc cancel ",
            PickerMode::Dir => " Directories (#)  type to filter · Enter select · Esc cancel ",
            PickerMode::Skill => " Skills ",
            PickerMode::Prompt => " Templates ",
            PickerMode::Model => " Models ",
            PickerMode::Provider => {
                " Providers  Enter = switch or connect · paste key if needed · Esc cancel "
            }
        };
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(self.palette.accent)),
        );

        frame.render_widget(list, picker_area);
    }

    /// Pretty-prints `raw` as JSON when it parses, otherwise returns it
    /// unchanged; then truncates to at most `max_lines` lines of at most
    /// `max_line_chars` characters each, appending a summary marker for
    /// anything dropped.
    fn format_permission_input(raw: &str, max_lines: usize, max_line_chars: usize) -> Vec<String> {
        let pretty = serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| raw.to_string());

        let mut lines: Vec<String> = pretty
            .lines()
            .map(|l| {
                if l.chars().count() > max_line_chars {
                    let truncated: String = l.chars().take(max_line_chars).collect();
                    format!("{}…", truncated)
                } else {
                    l.to_string()
                }
            })
            .collect();

        if lines.len() > max_lines {
            let dropped = lines.len() - max_lines;
            lines.truncate(max_lines);
            lines.push(format!("… ({} more line{})", dropped, if dropped == 1 { "" } else { "s" }));
        }
        lines
    }

    fn render_permission_prompt(&mut self, frame: &mut Frame, area: Rect) {
        let pending = match self.pending_permission.as_ref() {
            Some(p) => p,
            None => return,
        };

        let tool_name = &pending.request.tool_name;
        let danger_reason = pending.request.danger_reason.as_deref();
        let is_dangerous = danger_reason.is_some();
        let input_lines = Self::format_permission_input(&pending.request.tool_input, 10, 100);

        let (border_color, title) = if is_dangerous {
            (self.palette.danger, " ⚠ DANGEROUS — Permission ")
        } else {
            (self.palette.warn, " Permission ")
        };

        let mut text = vec![Line::from(Span::styled(
            format!(" ⚠  {} requires approval", tool_name),
            Style::default()
                .fg(if is_dangerous { self.palette.danger } else { self.palette.warn })
                .add_modifier(Modifier::BOLD),
        ))];

        if let Some(reason) = danger_reason {
            text.push(Line::from(Span::styled(
                format!(" ⚠ DANGEROUS: {}", reason),
                Style::default().fg(self.palette.danger).add_modifier(Modifier::BOLD),
            )));
        }

        text.push(Line::from(""));
        for line in &input_lines {
            text.push(Line::from(Span::styled(
                format!(" {}", line),
                Style::default().fg(self.palette.text),
            )));
        }
        text.push(Line::from(""));
        text.push(Line::from(Span::styled(
            format!(
                " [{}] once   [{}] always (trust project)   [{}] deny ",
                self.keys.binding("perm_once"),
                self.keys.binding("perm_always"),
                self.keys.binding("perm_deny"),
            ),
            Style::default().fg(self.palette.muted),
        )));

        let prompt_height = (text.len() as u16 + 2).min(area.height.saturating_sub(4).max(3));
        let prompt_y = area.height.saturating_sub(4 + prompt_height + 2);
        let prompt_area = Rect {
            x: area.x + 2,
            y: area.y + prompt_y,
            width: area.width.saturating_sub(4).min(90),
            height: prompt_height,
        };

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(border_color)),
            );

        frame.render_widget(paragraph, prompt_area);
    }
}

#[cfg(test)]
mod fuzzy_tests {
    use super::App;

    #[test]
    fn fuzzy_matches_subsequence() {
        let items = vec![
            "opencode-cli/opencode/claude-sonnet-4-6".into(),
            "opencode-cli/opencode/deepseek-v4-flash-free".into(),
            "anthropic/claude-haiku-4-5".into(),
        ];
        let hit = App::rank_and_filter(&items, "sonnet", 10);
        assert!(hit.iter().any(|s| s.contains("sonnet")));
        let hit = App::rank_and_filter(&items, "ocs4", 10);
        assert!(
            hit.iter().any(|s| s.contains("claude-sonnet")),
            "expected subsequence match, got {:?}",
            hit
        );
        let hit = App::rank_and_filter(&items, "deep flash", 10);
        assert!(hit.iter().any(|s| s.contains("deepseek")));
    }

    #[test]
    fn fuzzy_prefers_prefix() {
        let items = vec![
            "claude-sonnet-4".into(),
            "x-claude-extra".into(),
        ];
        let hit = App::rank_and_filter(&items, "claude", 10);
        assert_eq!(hit[0], "claude-sonnet-4");
    }
}
