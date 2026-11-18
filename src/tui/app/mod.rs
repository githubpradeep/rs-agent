//! TUI application core shell — App state, types, constructor, event loop.
//! Behavior lives in themed submodules (keys, commands, pickers, render, …).

pub(crate) use crate::agent::r#loop::AgentEvent;
pub(crate) use crate::agent::AgentMode;
pub(crate) use crate::ai::provider::Provider;
pub(crate) use crate::ai::types::Message;
pub(crate) use crate::context::{
    build_commands_section, build_context_section, discover_agent_commands, discover_context_files,
};
pub(crate) use crate::permission::{
    extract_tool_path, path_allow_prefix, PathAllowStore, PendingPermission, PermissionReply,
    TrustStore,
};
pub(crate) use crate::session::{self, SessionData, SessionStore};
pub(crate) use crate::skills::{
    discover_skills, discover_templates, find_skill, find_template, format_skill_injection,
    list_skills_summary, render_template,
};
pub(crate) use crate::tools::question::{PendingQuestion, QuestionReply};
pub(crate) use crossbeam_channel as channel;
pub(crate) use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEventKind,
};
pub(crate) use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
pub(crate) use globset::{Glob, GlobSet, GlobSetBuilder};
pub(crate) use ratatui::layout::Rect;
pub(crate) use ratatui::style::{Modifier, Style};
pub(crate) use ratatui::text::{Line, Span};
pub(crate) use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
pub(crate) use ratatui::Frame;

pub(crate) use super::help::{self, HelpOverlay};
pub(crate) use super::hit::{HitMap, HitTarget};
pub(crate) use super::keys::{merge_keybindings, KeyMap};
pub(crate) use super::layout::{compute_view, LayoutOpts};
pub(crate) use super::renderer::{render_markdown, MarkdownStyle};
pub(crate) use super::sessions_panel::{self, SessionRow, SessionsPanelState};
pub(crate) use super::settings::{self, SettingsState};
pub(crate) use super::status::{self, SessionUiState};
pub(crate) use super::theme::{Palette, ThemeName};
pub(crate) use super::toast::{self, Toast};
pub(crate) use super::tree_view::{self, SidePanelMode};
pub(crate) use super::ui::FocusZone;
pub(crate) use super::widgets;
pub(crate) use std::collections::HashMap;
pub(crate) use std::io;
pub(crate) use std::sync::Arc;
pub(crate) use std::time::{Duration, Instant};
pub(crate) use walkdir::WalkDir;

mod session_runtime;
pub(crate) use session_runtime::{
    spawn_session_runtime, turn_active_from_event, SessionRuntime, SessionRuntimeConfig,
};

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

/// Live bash/repl/task output tab (herdr-style switchable buffers, not a mux).
#[derive(Clone)]
struct ToolOutputTab {
    id: String,
    name: String,
    label: String,
    buffer: String,
    done: bool,
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    Insert,
    Waiting,
    /// Capturing an API key for `pending_api_key_provider`.
    ApiKey,
    /// Answering a `question` tool prompt.
    Question,
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
/// Cross-thread commands for a session runtime. `SwitchSession` carries a full
/// transcript, hence the allow for the size delta between variants.
#[allow(clippy::large_enum_variant)]
pub(crate) enum AppCommand {
    Submit {
        text: String,
    },
    Steer {
        text: String,
    },
    Abort,
    Compact,
    NewSession,
    ForkSession {
        label: Option<String>,
        /// Truncate to first N API messages before forking (timeline).
        at: Option<usize>,
    },
    /// Refresh `/timeline` entries from the live agent transcript.
    RequestTimeline,
    SetModel {
        model: String,
    },
    /// Mid-session provider+model swap (pi parity).
    SetProvider {
        provider: Arc<dyn Provider>,
        model: String,
    },
    SetMode {
        mode: AgentMode,
    },
    SetSkillTools {
        tools: Vec<String>,
    },
    SetTitle {
        title: String,
    },
    SetSystemPrompt {
        prompt: String,
    },
    Init {
        messages: Vec<Message>,
    },
    GoalSet {
        condition: String,
    },
    GoalClear,
    GoalPause,
    GoalResume,
    GoalStatus,
    /// Persist the current session, then load another (atomic session switch).
    SwitchSession {
        data: SessionData,
    },
    /// Force-save current session to disk.
    PersistSession,
    Exit,
}

mod agent_events;
mod commands;
mod helpers;
mod keys;
mod permissions;
mod pickers;
mod render;
mod sessions;
use helpers::*;

pub struct App {
    messages: Vec<ChatMessage>,
    input: String,
    input_mode: InputMode,
    should_exit: bool,
    status: String,
    /// Clones of the focused session runtime's command channel / abort.
    command_tx: channel::Sender<AppCommand>,
    /// One OS-thread agent loop per session id.
    runtimes: HashMap<String, SessionRuntime>,
    /// Shared permission requests from every runtime (cloned Sender at spawn).
    permission_tx: channel::Sender<PendingPermission>,
    /// LSP diagnostics / status (not tied to a session runtime).
    lsp_event_rx: channel::Receiver<(usize, AgentEvent)>,
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
    pending_question: Option<PendingQuestion>,
    question_rx: channel::Receiver<PendingQuestion>,
    trust_store: TrustStore,
    path_allows: PathAllowStore,
    #[allow(dead_code)]
    approved: bool,
    auto_mode: bool,
    token_used: usize,
    token_limit: usize,
    session_input_tokens: usize,
    session_output_tokens: usize,
    /// Cumulative prompt-cache hits (shown as a hit-rate chip).
    session_cache_read_tokens: usize,
    near_limit: bool,
    session_id: String,
    session_title: Option<String>,
    /// Footer chip while `/goal` is active/paused (Claude Code `◎ /goal`).
    goal_indicator: String,
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
    /// Console scrollback: lines up from the tail (0 = follow live output).
    console_scroll: usize,
    /// Last rendered chat viewport height (for page-scroll sizing).
    chat_rows: usize,
    /// Where the console panel was last drawn (mouse-wheel routing).
    console_area: Option<Rect>,
    /// Switchable live tool outputs (Tab cycles while waiting).
    tool_output_tabs: Vec<ToolOutputTab>,
    tool_output_tab: usize,
    tool_in_progress: Option<(String, Instant)>,
    context_enabled: bool,
    provider: Arc<dyn Provider>,
    provider_name: String,
    timeout_secs: u64,
    /// Auto "continue" attempts after transport/timeout hiccups (Error path).
    provider_auto_continues: u8,
    /// Cycle list of `provider/model` display strings (pi Ctrl+P).
    model_cycle: Vec<String>,
    model_cycle_index: usize,
    /// When false, skip mouse capture so the terminal can do native text selection.
    mouse_enabled: bool,
    show_timeline_panel: bool,
    timeline_selection: usize,
    /// `(api_index, summary)` for the timeline side panel.
    timeline_entries: Vec<(usize, String)>,
    lsp_summary: String,
    lsp_cmd_tx: Option<channel::Sender<LspCmd>>,
    /// Herdr-style "done but unseen" — cleared when the user interacts.
    unseen_done: bool,
    toast: Option<Toast>,
    toast_enabled: bool,
    toast_sound: bool,
    help_overlay: Option<HelpOverlay>,
    settings: Option<SettingsState>,
    /// Ctrl+K command palette.
    palette_open: bool,
    palette_query: String,
    palette_selection: usize,
    palette_items: Vec<String>,
    focus_zone: FocusZone,
    show_sessions_panel: bool,
    sessions_panel: SessionsPanelState,
    /// Last sessions-panel refresh.
    sessions_refresh_at: Instant,
    side_mode: SidePanelMode,
    tree_nodes: Vec<tree_view::CallTreeNode>,
    hit_map: HitMap,
    diagnostic_banner: Option<String>,
    /// Session id the focused runtime is bound to (kept equal to `session_id`).
    agent_session_id: String,
    /// Focused runtime is mid-turn.
    agent_turn_active: bool,
    /// Another runtime still running after UI focused elsewhere.
    bg_running_session: Option<String>,
    /// Live chat per session (includes user bubbles) — survives switch better than disk reload.
    ui_snapshots: HashMap<String, Vec<ChatMessage>>,
    /// Spawn knobs for additional parallel runtimes.
    max_iterations: usize,
    thinking_budget: Option<u32>,
    system_prompt_opt: Option<String>,
}

enum LspCmd {
    Start,
    DidSave { path: String, text: String },
    Stop,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn Provider>,
        model: String,
        timeout_secs: u64,
        approve: bool,
        resume: Option<SessionData>,
        system_prompt: Option<String>,
        max_iterations: usize,
        auto_mode: bool,
        rlm_depth: u32,
        thinking_budget: Option<u32>,
    ) -> Self {
        let cfg = crate::config::Config::load();
        let theme_name = match cfg.theme.as_deref() {
            None | Some("auto") => ThemeName::from_host(),
            Some(s) => ThemeName::parse(s),
        };
        let palette = Palette::for_theme(theme_name);
        let keys = KeyMap::new(merge_keybindings(&cfg.keybindings));
        let mouse_enabled = !cfg.disable_mouse.unwrap_or(false);
        let provider_for_app = provider.clone();

        let (permission_tx, permission_rx) = channel::unbounded::<PendingPermission>();
        let (question_tx, question_rx) = channel::unbounded::<PendingQuestion>();
        crate::tools::question::set_question_channel(question_tx);

        let (lsp_event_tx, lsp_event_rx) = channel::unbounded::<(usize, AgentEvent)>();
        let (lsp_cmd_tx, lsp_cmd_rx) = channel::unbounded::<LspCmd>();
        let event_tx_lsp = lsp_event_tx;
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            rt.block_on(async move {
                let mut client: Option<crate::lsp::LspClient> = None;
                let mut _reader: Option<tokio::task::JoinHandle<()>> = None;
                loop {
                    match lsp_cmd_rx.recv_timeout(Duration::from_millis(400)) {
                        Ok(LspCmd::Start) => {
                            let root = std::env::current_dir()
                                .unwrap_or_else(|_| std::path::PathBuf::from("."));
                            match crate::lsp::LspClient::start_rust_analyzer(root).await {
                                Ok((c, handle)) => {
                                    crate::tools::post_mutation::register_shared_diagnostics(
                                        c.diagnostics.clone(),
                                    );
                                    crate::tools::post_mutation::DiagnosticsBridge::global()
                                        .set_snapshot(c.snapshot());
                                    let summary = c.snapshot().summary_line();
                                    client = Some(c);
                                    _reader = Some(handle);
                                    let msg = if summary.is_empty() {
                                        " LSP…".into()
                                    } else {
                                        summary
                                    };
                                    let _ = event_tx_lsp
                                        .send((0, AgentEvent::LspUpdate { summary: msg }));
                                }
                                Err(e) => {
                                    let _ = event_tx_lsp.send((
                                        0,
                                        AgentEvent::Status {
                                            message: format!("lsp start failed: {e}"),
                                        },
                                    ));
                                }
                            }
                        }
                        Ok(LspCmd::DidSave { path, text }) => {
                            if let Some(ref c) = client {
                                let p = std::path::Path::new(&path);
                                if let Some(lang) = crate::lsp::language_id_for(p) {
                                    let _ = c.did_open(p, &text, lang).await;
                                    let _ = c.did_save(p, Some(&text)).await;
                                }
                                crate::tools::post_mutation::DiagnosticsBridge::global()
                                    .set_snapshot(c.snapshot());
                            }
                        }
                        Ok(LspCmd::Stop) => {
                            client = None;
                            _reader = None;
                            let _ = event_tx_lsp.send((
                                0,
                                AgentEvent::LspUpdate {
                                    summary: String::new(),
                                },
                            ));
                        }
                        Err(channel::RecvTimeoutError::Timeout) => {
                            if let Some(ref c) = client {
                                let snap = c.snapshot();
                                crate::tools::post_mutation::DiagnosticsBridge::global()
                                    .set_snapshot(snap.clone());
                                let summary = snap.summary_line();
                                let _ = event_tx_lsp.send((0, AgentEvent::LspUpdate { summary }));
                            }
                        }
                        Err(channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        });

        let provider_name = provider.name().to_string();
        let provider_name_for_banner = provider_name.clone();
        let session_id = resume
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(SessionStore::generate_id);
        crate::tools::turn_snapshot::set_session(&session_id);
        crate::agent::trajectory::set_session(&session_id);
        let created_at = resume
            .as_ref()
            .map(|s| s.created_at.clone())
            .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
        let title = resume.as_ref().and_then(|s| s.title.clone());

        let runtime_cfg = if let Some(ref data) = resume {
            SessionRuntimeConfig::from_session_data(
                data,
                provider.clone(),
                model.clone(),
                timeout_secs,
                approve,
                max_iterations,
                rlm_depth,
                thinking_budget,
                system_prompt.clone(),
                permission_tx.clone(),
            )
        } else {
            SessionRuntimeConfig {
                provider: provider.clone(),
                model: model.clone(),
                timeout_secs,
                approve,
                max_iterations,
                rlm_depth,
                thinking_budget,
                system_prompt: system_prompt.clone(),
                permission_tx: permission_tx.clone(),
                id: session_id.clone(),
                created_at: created_at.clone(),
                title: title.clone(),
                parent_id: None,
                branch_label: None,
                messages: Vec::new(),
                goal: None,
                todos: None,
            }
        };
        let first_runtime = spawn_session_runtime(runtime_cfg);
        let command_tx = first_runtime.command_tx.clone();
        let abort_flag = first_runtime.abort_flag.clone();
        let steer_queue = first_runtime.steer_queue.clone();
        let mut runtimes = HashMap::new();
        runtimes.insert(session_id.clone(), first_runtime);

        let trust_store = TrustStore::new();
        let path_allows = PathAllowStore::new();

        let provider_banner = provider_name_for_banner.clone();
        let mut initial_msgs = vec![ChatMessage {
            role: "system".to_string(),
            text: format!(
                "**rs-agent** · Deep Context coding agent\n\
                 `{provider}` / `{model}` · session `{session}`\n\n\
                 Huge files stay in `/tree` — they don't blow the window\n\
                 Type to start · `/` commands · `@` files · Esc abort · Enter steer",
                provider = provider_banner,
                model = model,
                session = SessionStore::short_id(&session_id),
            ),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        }];

        if let Some(warn) = crate::agent::weak_model_user_warning(&model) {
            initial_msgs.push(ChatMessage {
                role: "system".to_string(),
                text: warn,
                thinking: None,
                show_thinking: false,
                tool_blocks: Vec::new(),
            });
        }

        if !crate::rlm::python3_available() {
            initial_msgs.push(ChatMessage {
                role: "system".to_string(),
                text: format!(
                    "⚠️ {} The `repl` tool (Deep Context) will fail until it's installed.",
                    crate::rlm::PYTHON3_NOT_FOUND
                ),
                thinking: None,
                show_thinking: false,
                tool_blocks: Vec::new(),
            });
        }

        if let Some(ref resume_data) = resume {
            if let Some(ref todos) = resume_data.todos {
                crate::tools::todowrite::restore(todos.clone());
            } else {
                crate::tools::todowrite::clear();
            }
            for msg in &resume_data.messages {
                match &msg.role {
                    crate::ai::types::Role::User => {
                        let text = msg
                            .content
                            .first()
                            .and_then(|c| c.text.as_deref())
                            .unwrap_or("");
                        if !text.is_empty() {
                            initial_msgs.push(ChatMessage {
                                role: "user".to_string(),
                                text: text.to_string(),
                                thinking: None,
                                show_thinking: false,
                                tool_blocks: Vec::new(),
                            });
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
                                    let input =
                                        c.input.as_ref().map(|v| v.to_string()).unwrap_or_default();
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
                            initial_msgs.push(ChatMessage {
                                role: "assistant".to_string(),
                                text,
                                thinking: thinking.clone(),
                                show_thinking: thinking
                                    .as_ref()
                                    .map(|t| !t.is_empty())
                                    .unwrap_or(false),
                                tool_blocks: Vec::new(),
                            });
                        }
                    }
                    crate::ai::types::Role::Tool => {
                        let name = msg
                            .content
                            .first()
                            .and_then(|c| c.name.as_deref())
                            .unwrap_or("tool");
                        let result = msg
                            .content
                            .first()
                            .and_then(|c| c.text.as_deref())
                            .unwrap_or("");
                        let preview: String = result.chars().take(200).collect();
                        if !preview.is_empty() {
                            initial_msgs.push(ChatMessage {
                                role: "tool".to_string(),
                                text: format!("✅ [{}] {}", name, preview),
                                thinking: None,
                                show_thinking: false,
                                tool_blocks: Vec::new(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        } else {
            crate::tools::todowrite::clear();
        }

        let agent_session_id = session_id.clone();
        Self {
            messages: initial_msgs,
            input: String::new(),
            input_mode: InputMode::Insert,
            should_exit: false,
            status: "ready".to_string(),
            command_tx,
            runtimes,
            permission_tx,
            lsp_event_rx,
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
            pending_question: None,
            question_rx,
            trust_store,
            path_allows,
            approved: approve,
            auto_mode,
            token_used: 0,
            token_limit: crate::ai::token_count::get_context_limit(&model),
            session_input_tokens: resume.as_ref().map(|s| s.total_input_tokens).unwrap_or(0),
            session_output_tokens: resume.as_ref().map(|s| s.total_output_tokens).unwrap_or(0),
            session_cache_read_tokens: 0,
            near_limit: false,
            session_id,
            session_title: title,
            goal_indicator: resume
                .as_ref()
                .and_then(|s| s.goal.as_ref())
                .filter(|g| {
                    matches!(
                        g.status,
                        crate::agent::goal::GoalStatus::Active
                            | crate::agent::goal::GoalStatus::Paused
                    )
                })
                .map(|g| format!("◎ /goal {}", g.status.as_str()))
                .unwrap_or_default(),
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
            console_scroll: 0,
            chat_rows: 0,
            console_area: None,
            tool_output_tabs: Vec::new(),
            tool_output_tab: 0,
            tool_in_progress: None,
            context_enabled: true,
            provider: provider_for_app,
            provider_name: provider_name_for_banner.clone(),
            timeout_secs,
            provider_auto_continues: 0,
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
            mouse_enabled,
            show_timeline_panel: false,
            timeline_selection: 0,
            timeline_entries: {
                if let Some(ref resume_data) = resume {
                    summarize_api_messages(&resume_data.messages)
                } else {
                    Vec::new()
                }
            },
            lsp_summary: String::new(),
            lsp_cmd_tx: Some(lsp_cmd_tx),
            unseen_done: false,
            toast: None,
            toast_enabled: cfg.toast.unwrap_or(true),
            toast_sound: cfg.toast_sound.unwrap_or(false),
            help_overlay: None,
            settings: None,
            palette_open: false,
            palette_query: String::new(),
            palette_selection: 0,
            palette_items: Vec::new(),
            focus_zone: FocusZone::Chat,
            show_sessions_panel: false,
            sessions_panel: SessionsPanelState::default(),
            sessions_refresh_at: Instant::now() - Duration::from_secs(60),
            side_mode: SidePanelMode::Tree,
            tree_nodes: Vec::new(),
            hit_map: HitMap::default(),
            diagnostic_banner: {
                let mut notes: Vec<String> = Vec::new();
                if !crate::rlm::python3_available() {
                    notes.push("python3 missing — Deep Context repl disabled".into());
                }
                if notes.is_empty() {
                    None
                } else {
                    Some(notes.join(" · "))
                }
            },
            agent_session_id,
            agent_turn_active: false,
            bg_running_session: None,
            ui_snapshots: HashMap::new(),
            max_iterations,
            thinking_budget,
            system_prompt_opt: system_prompt,
        }
    }

    fn push_toast(&mut self, toast: Toast) {
        if !self.toast_enabled {
            return;
        }
        if self.toast_sound {
            toast::play_sound(toast.kind);
        }
        self.toast = Some(toast);
    }

    fn dismiss_overlays(&mut self) {
        self.help_overlay = None;
        self.settings = None;
        self.palette_open = false;
        self.palette_query.clear();
    }

    fn open_command_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_selection = 0;
        self.refresh_palette_items();
    }

    fn refresh_palette_items(&mut self) {
        self.palette_items = super::command_catalog::filter_commands(&self.palette_query);
        if self.palette_selection >= self.palette_items.len() {
            self.palette_selection = self.palette_items.len().saturating_sub(1);
        }
    }

    /// Toast when a turn ends while the user isn't looking.
    fn notify_done(&mut self, detail: &str) {
        if self.unseen_done {
            self.push_toast(Toast::finished("done", detail));
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(&mut stdout))?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        if self.mouse_enabled {
            crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;
        }

        loop {
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(10))? {
                self.handle_event(event::read()?)?;
            }

            // Drain every session runtime; only paint events for the focused one.
            for ev in self.pump_runtime_events() {
                self.handle_agent_event(ev);
            }

            while let Ok((_idx, event)) = self.lsp_event_rx.try_recv() {
                self.handle_agent_event(event);
            }

            self.poll_model_picker();

            if let Ok(pending) = self.permission_rx.try_recv() {
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let is_trusted = self.approved || self.trust_store.is_trusted(&cwd);
                let path_ok = {
                    let target = extract_tool_path(&pending.request.tool_input);
                    self.path_allows
                        .allows(&cwd, &pending.request.tool_name, target.as_deref())
                };
                if is_trusted || path_ok || self.auto_allow(&pending.request.tool_name) {
                    let _ = pending.reply_tx.send(PermissionReply::AllowOnce);
                } else {
                    let tool = pending.request.tool_name.clone();
                    self.pending_permission = Some(pending);
                    self.push_toast(Toast::blocked(
                        "needs attention",
                        format!("permission: {tool}"),
                    ));
                }
            }

            if let Ok(pending) = self.question_rx.try_recv() {
                self.input.clear();
                self.input_mode = InputMode::Question;
                self.status = "awaiting answer...".to_string();
                let q = pending.request.question.clone();
                self.pending_question = Some(pending);
                self.push_toast(Toast::blocked("needs attention", format!("question: {q}")));
            }

            if let Some(t) = self.toast.as_ref() {
                if t.expired() {
                    self.toast = None;
                }
            }
            if self.show_sessions_panel
                && self.sessions_refresh_at.elapsed() >= Duration::from_secs(2)
            {
                self.sessions_panel
                    .refresh(&self.session_id, self.bg_running_session.as_deref());
                self.sessions_refresh_at = Instant::now();
            }

            if self.should_exit {
                break;
            }
        }

        for rt in self.runtimes.values() {
            let _ = rt.command_tx.send(AppCommand::Exit);
        }
        terminal::disable_raw_mode()?;
        crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableBracketedPaste)?;
        if self.mouse_enabled {
            crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
        }
        // Alternate screen is gone — print a durable resume hint.
        let short = SessionStore::short_id(&self.session_id);
        let title = self.session_title.as_deref().unwrap_or("untitled");
        eprintln!("Session saved: {} ({title})", self.session_id);
        eprintln!("Resume:  rs-agent -r {short}");
        eprintln!("Or:      rs-agent -r latest");
        eprintln!("List:    rs-agent --list-sessions");
        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> io::Result<()> {
        match event {
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                        // Wheel over the console scrolls the console, not the chat.
                        let over_console = self
                            .console_area
                            .map(|a| mouse.row >= a.y && mouse.row < a.y.saturating_add(a.height))
                            .unwrap_or(false);
                        if over_console {
                            if mouse.kind == MouseEventKind::ScrollUp {
                                self.console_scroll = self.console_scroll.saturating_add(3);
                            } else {
                                self.console_scroll = self.console_scroll.saturating_sub(3);
                            }
                        } else {
                            self.follow_bottom = false;
                            if mouse.kind == MouseEventKind::ScrollDown {
                                self.scroll_offset = self.scroll_offset.saturating_add(3);
                            } else {
                                self.scroll_offset = self.scroll_offset.saturating_sub(3);
                            }
                        }
                    }
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        if let Some(target) = self.hit_map.hit_at(mouse.column, mouse.row).cloned()
                        {
                            match target {
                                HitTarget::Thinking { msg_idx } => {
                                    if let Some(msg) = self.messages.get_mut(msg_idx) {
                                        msg.show_thinking = !msg.show_thinking;
                                    }
                                }
                                HitTarget::Tool { msg_idx, tool_idx } => {
                                    if let Some(block) = self
                                        .messages
                                        .get_mut(msg_idx)
                                        .and_then(|m| m.tool_blocks.get_mut(tool_idx))
                                    {
                                        block.expanded = !block.expanded;
                                    }
                                }
                                HitTarget::Toast => self.toast = None,
                                HitTarget::ModalDismiss => {
                                    if self.help_overlay.is_some()
                                        || self.settings.is_some()
                                        || self.palette_open
                                    {
                                        self.dismiss_overlays();
                                    }
                                }
                                HitTarget::SessionRow { index } => {
                                    if index < self.sessions_panel.rows.len()
                                        && self.sessions_panel.rows[index].selectable()
                                    {
                                        self.sessions_panel.selection = index;
                                        self.activate_sessions_selection();
                                    }
                                }
                                HitTarget::OutputTab { index } => {
                                    if index < self.tool_output_tabs.len() {
                                        self.tool_output_tab = index;
                                        self.show_repl_panel = true;
                                    }
                                }
                                HitTarget::Help | HitTarget::PaletteItem { .. } => {}
                            }
                            return Ok(());
                        }
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
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.should_exit = true;
                } else if key.code == KeyCode::Char('k')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    if self.palette_open {
                        self.dismiss_overlays();
                    } else {
                        self.open_command_palette();
                    }
                } else if self.help_overlay.is_some() {
                    self.handle_help_key(key);
                } else if self.settings.is_some() {
                    self.handle_settings_key(key);
                } else if self.palette_open {
                    self.handle_palette_key(key);
                } else if self.pending_permission.is_some() {
                    self.handle_permission_key(key);
                } else if self.pending_question.is_some() {
                    self.handle_question_key(key);
                } else {
                    match key.code {
                        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.cycle_model();
                        }
                        KeyCode::Char('?') if self.input_mode == InputMode::Normal => {
                            self.help_overlay = Some(HelpOverlay::default());
                        }
                        _ => match self.input_mode {
                            InputMode::Waiting => self.handle_waiting_key(key),
                            InputMode::Normal => self.handle_normal_key(key),
                            InputMode::Insert => self.handle_insert_key(key),
                            InputMode::ApiKey => self.handle_api_key_input(key),
                            InputMode::Question => self.handle_question_key(key),
                        },
                    }
                }
            }
            Event::Paste(text) => {
                if self.pending_permission.is_some() {
                    // ignore paste while a permission prompt is up
                } else if self.picker_active {
                    self.picker_query
                        .push_str(text.replace(['\n', '\r'], "").as_str());
                    self.update_picker_results();
                } else {
                    match self.input_mode {
                        InputMode::Insert
                        | InputMode::Waiting
                        | InputMode::ApiKey
                        | InputMode::Question => {
                            self.input.push_str(text.replace(['\n', '\r'], "").as_str());
                        }
                        InputMode::Normal => {}
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Strip ANSI / control chars so ratatui never paints mid-escape garbage.
    fn sanitize_console_text(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                // CSI / OSC-ish sequences: skip until letter or BEL
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for d in chars.by_ref() {
                        if d.is_ascii_alphabetic() || d == 'm' {
                            break;
                        }
                    }
                } else if chars.peek() == Some(&']') {
                    chars.next();
                    for d in chars.by_ref() {
                        if d == '\u{7}' || d == '\n' {
                            break;
                        }
                    }
                }
                continue;
            }
            if c == '\r' {
                continue;
            }
            if c == '\t' {
                out.push_str("    ");
                continue;
            }
            if c == '\n' || (c >= ' ' && c != '\u{7f}') {
                out.push(c);
            }
        }
        out
    }

    fn clip_console_line(s: &str, max_cols: usize) -> String {
        if max_cols == 0 {
            return String::new();
        }
        let count = s.chars().count();
        if count <= max_cols {
            return s.to_string();
        }
        let mut out: String = s.chars().take(max_cols.saturating_sub(1)).collect();
        out.push('…');
        out
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

    /// Drain all session runtime event queues (update turn_active / offline buffers).
    /// Returns focused-session events to paint via `handle_agent_event`.
    fn pump_runtime_events(&mut self) -> Vec<AgentEvent> {
        let focused = self.session_id.clone();
        let mut focused_events = Vec::new();
        let mut bg_stream: Vec<(String, AgentEvent)> = Vec::new();
        let mut bg_done: Vec<(String, AgentEvent)> = Vec::new();
        let mut bg_ids = Vec::new();
        for (id, rt) in self.runtimes.iter_mut() {
            while let Ok((_idx, ev)) = rt.event_rx.try_recv() {
                if let Some(active) = turn_active_from_event(&ev) {
                    rt.turn_active = active;
                    if !active {
                        rt.offline_events.clear();
                    }
                }
                let is_focused = id == &focused;
                if is_focused {
                    focused_events.push(ev);
                } else {
                    let is_stream = matches!(
                        ev,
                        AgentEvent::TextDelta { .. }
                            | AgentEvent::ThinkingDelta { .. }
                            | AgentEvent::ToolUseStart { .. }
                            | AgentEvent::ToolUseDelta { .. }
                            | AgentEvent::ToolResult { .. }
                            | AgentEvent::ToolOutput { .. }
                            | AgentEvent::ReplOutput { .. }
                    );
                    if is_stream {
                        if self.ui_snapshots.contains_key(id) {
                            bg_stream.push((id.clone(), ev.clone()));
                        } else if rt.turn_active {
                            rt.offline_events.push(ev.clone());
                        }
                    }
                    if matches!(
                        ev,
                        AgentEvent::Done | AgentEvent::Aborted | AgentEvent::Error { .. }
                    ) {
                        bg_done.push((id.clone(), ev));
                    }
                }
            }
            if rt.turn_active && id != &focused {
                bg_ids.push(id.clone());
            }
        }
        self.bg_running_session = bg_ids.first().cloned();

        for (id, ev) in bg_stream {
            if let Some(snap) = self.ui_snapshots.get_mut(&id) {
                match ev {
                    AgentEvent::TextDelta { text } => {
                        Self::append_text_delta_to_snapshot(snap, &text);
                    }
                    AgentEvent::ThinkingDelta { thinking } => {
                        Self::append_thinking_delta_to_snapshot(snap, &thinking);
                    }
                    _ => {}
                }
            }
        }

        for (id, event) in bg_done {
            let short = SessionStore::short_id(&id);
            match &event {
                AgentEvent::Error { message } => {
                    self.push_toast(Toast::blocked(
                        "background error",
                        format!("{short}: {message}"),
                    ));
                }
                AgentEvent::Aborted => {
                    self.push_system(format!("Background session {short} aborted."));
                }
                _ => {
                    self.push_toast(Toast::finished(
                        "background done",
                        format!("{short} finished"),
                    ));
                }
            }
            if self.show_sessions_panel {
                self.sessions_panel
                    .refresh(&self.session_id, self.bg_running_session.as_deref());
            }
        }
        focused_events
    }
}
