use crate::agent::r#loop::AgentEvent;
use crate::agent::state::AgentState;
use crate::agent::{AgentLoop, AgentMode};
use crate::ai::provider::Provider;
use crate::ai::types::Message;
use crate::context::{
    build_commands_section, build_context_section, discover_agent_commands,
    discover_context_files,
};
use crate::permission::{
    extract_tool_path, path_allow_prefix, PathAllowStore, PendingPermission, PermissionReply,
    TrustStore,
};
use crate::tools::question::{PendingQuestion, QuestionReply};
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
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use super::fleet_panel::{self, CityRow, FleetPanelState};
use super::help::{self, HelpOverlay};
use super::hit::{HitMap, HitTarget};
use super::keys::{merge_keybindings, KeyMap};
use super::layout::{compute_view, LayoutOpts};
use super::renderer::{render_markdown, MarkdownStyle};
use super::settings::{self, SettingsState};
use super::status::{self, SessionUiState};
use super::theme::{Palette, ThemeName};
use super::toast::{self, Toast};
use super::tree_view::{self, SidePanelMode};
use super::widgets;
use crate::lifecycle::{self, Lifecycle};
use crate::notify::{self, NotifyMode};
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
enum AppCommand {
    Submit { text: String },
    Steer { text: String },
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
    SetModel { model: String },
    /// Mid-session provider+model swap (pi parity).
    SetProvider {
        provider: Arc<dyn Provider>,
        model: String,
    },
    SetMode { mode: AgentMode },
    SetSkillTools { tools: Vec<String> },
    SetTitle { title: String },
    SetSystemPrompt { prompt: String },
    Init { messages: Vec<Message> },
    GoalSet { condition: String },
    GoalClear,
    GoalPause,
    GoalResume,
    GoalStatus,
    /// Request a consenting handoff (injects user message).
    HandoffRequest,
    /// Bind or clear seat identity (`None` = clear).
    SetSeat { name: Option<String> },
    /// Replace agent state with a worker/TUI session (fleet attach).
    LoadSession { data: SessionData },
    /// Force-save current session to disk (fleet detach).
    PersistSession,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FleetAttachPhase {
    /// Live log follow only (worker keeps running).
    Follow,
    /// Sent pause; waiting for worker `state=paused`.
    Attaching,
    /// Human owns the seat session; worker is paused.
    Attached,
    /// Read-only session inspect; worker keeps running (no chat takeover).
    Inspect,
}

struct FleetAttachState {
    seat: String,
    phase: FleetAttachPhase,
    follower: crate::fleet::LogFollower,
    /// Session id we adopted from the worker (Attached).
    worker_session_id: Option<String>,
    /// TUI session id before attach (reserved for future restore-on-detach).
    #[allow(dead_code)]
    prior_session_id: Option<String>,
    attach_started: Instant,
}

mod helpers;
use helpers::*;

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
    pending_kitty_images: Vec<String>,
    lsp_summary: String,
    lsp_cmd_tx: Option<channel::Sender<LspCmd>>,
    /// Live fleet follow / attach takeover.
    fleet_attach: Option<FleetAttachState>,
    /// Herdr-style "done but unseen" — cleared when the user interacts.
    unseen_done: bool,
    /// Cached beads ready count (avoid disk I/O every frame).
    beads_ready_cache: (Instant, usize),
    toast: Option<Toast>,
    toast_enabled: bool,
    toast_sound: bool,
    notify_mode: NotifyMode,
    help_overlay: Option<HelpOverlay>,
    settings: Option<SettingsState>,
    /// Ctrl+K command palette.
    palette_open: bool,
    palette_query: String,
    palette_selection: usize,
    palette_items: Vec<String>,
    show_fleet_panel: bool,
    fleet_panel: FleetPanelState,
    side_mode: SidePanelMode,
    tree_nodes: Vec<tree_view::CallTreeNode>,
    hit_map: HitMap,
    diagnostic_banner: Option<String>,
    allowed_transitions: Vec<String>,
    /// Last fleet refresh.
    fleet_refresh_at: Instant,
}

enum LspCmd {
    Start,
    DidSave { path: String, text: String },
    Stop,
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: String, timeout_secs: u64, approve: bool, resume: Option<SessionData>, system_prompt: Option<String>, max_iterations: usize, auto_mode: bool, rlm_depth: u32, thinking_budget: Option<u32>) -> Self {
        let cfg = crate::config::Config::load();
        let theme_name = match cfg.theme.as_deref() {
            None | Some("auto") => ThemeName::from_host(),
            Some(s) => ThemeName::parse(s),
        };
        let palette = Palette::for_theme(theme_name);
        let keys = KeyMap::new(merge_keybindings(&cfg.keybindings));
        let mouse_enabled = !cfg.disable_mouse.unwrap_or(false);
        let provider_for_app = provider.clone();

        let (command_tx, command_rx) = channel::unbounded::<AppCommand>();
        let (event_tx, event_rx) = channel::unbounded::<(usize, AgentEvent)>();
        let (permission_tx, permission_rx) = channel::unbounded::<PendingPermission>();
        let (question_tx, question_rx) = channel::unbounded::<PendingQuestion>();
        crate::tools::question::set_question_channel(question_tx);

        let (lsp_cmd_tx, lsp_cmd_rx) = channel::unbounded::<LspCmd>();
        let event_tx_lsp = event_tx.clone();
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
                                    let _ =
                                        event_tx_lsp.send((0, AgentEvent::LspUpdate { summary: msg }));
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
        crate::tools::turn_snapshot::set_session(&session_id);
        let created_at = resume.as_ref().map(|s| s.created_at.clone()).unwrap_or_else(|| {
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
        });
        let resume_msgs = resume.as_ref().map(|s| s.messages.clone()).unwrap_or_default();
        let resume_goal = resume.as_ref().and_then(|s| s.goal.clone()).filter(|g| {
            matches!(
                g.status,
                crate::agent::goal::GoalStatus::Active | crate::agent::goal::GoalStatus::Paused
            )
        });
        let resume_seat = resume.as_ref().and_then(|s| s.seat.clone());
        let resume_handoff = resume.as_ref().and_then(|s| s.handoff.clone());
        let title = resume.as_ref().and_then(|s| s.title.clone());
        let parent_id_resume = resume.as_ref().and_then(|s| s.parent_id.clone());
        let branch_label_resume = resume.as_ref().and_then(|s| s.branch_label.clone());
        let session_id_for_thread = session_id.clone();
        let created_at_for_thread = created_at.clone();
        let title_for_thread = title.clone();
        let parent_for_thread = parent_id_resume;
        let branch_for_thread = branch_label_resume;
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
                state.goal = resume_goal;
                state.seat = resume_seat.clone();
                state.handoff = resume_handoff.clone();
                crate::agent::handoff::restore(resume_handoff);
                if let Some(ref seat_name) = resume_seat {
                    crate::tools::handoff::set_active_seat(Some(seat_name.clone()));
                }
                // Wake with purpose after resume (or when seat/handoff present).
                state.pending_wake = state.seat.is_some()
                    || state.handoff.is_some()
                    || state.goal.is_some();

                for msg in &resume_msgs {
                    state.add_message(msg.clone());
                }

                let goal_verify = crate::config::Config::load()
                    .goal_verify
                    .unwrap_or(true);
                let mut agent_loop = AgentLoop::new(provider2, state)
                    .with_max_iterations(max_iterations)
                    .with_abort(abort_for_thread.clone())
                    .with_steer(steer_for_thread.clone())
                    .with_rlm_depth(0, max_rlm_depth)
                    .with_goal_verify(goal_verify);
                if !approve {
                    agent_loop.set_permission_channel(permission_tx);
                }
                // Bridge tool-emitted events (REPL stdout / bash stream) onto the TUI event channel.
                let (sink_tx, sink_rx) = channel::unbounded::<AgentEvent>();
                crate::tools::output_sink::set_tool_output_sink(sink_tx.clone());
                let event_tx_bridge = event_tx.clone();
                std::thread::spawn(move || {
                    while let Ok(ev) = sink_rx.recv() {
                        let _ = event_tx_bridge.send((0, ev));
                    }
                });
                agent_loop = agent_loop.with_event_sink(sink_tx);
                crate::tools::register_default_tools_with_rlm(&mut agent_loop, max_rlm_depth);
                {
                    let mcp_cfg = crate::config::Config::load().mcp;
                    if !mcp_cfg.servers.is_empty() {
                        let lines =
                            crate::mcp::attach_mcp_from_config(&mut agent_loop, &mcp_cfg).await;
                        for line in lines {
                            let _ = event_tx.send((0, AgentEvent::Status { message: line }));
                        }
                    }
                }

                let store = SessionStore::new();
                let mut session_id_local = session_id_for_thread.clone();
                let mut created_at_local = created_at_for_thread.clone();
                let mut title_local = title_for_thread.clone();
                let mut parent_id_local = parent_for_thread.clone();
                let mut branch_label_local = branch_for_thread.clone();

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
                            agent_loop.clear_goal();
                            agent_loop.state_mut().seat = None;
                            agent_loop.state_mut().handoff = None;
                            agent_loop.state_mut().pending_wake = false;
                            crate::agent::handoff::clear();
                            crate::tools::handoff::set_active_seat(None);
                            abort_for_thread.clear();
                            steer_for_thread.clear();
                            crate::tools::todowrite::clear();
                            session_id_local = SessionStore::generate_id();
                            created_at_local = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            title_local = None;
                            parent_id_local = None;
                            branch_label_local = None;
                            let _ = event_tx.send((0, AgentEvent::SessionMeta {
                                id: session_id_local.clone(),
                                title: None,
                            }));
                            let _ = event_tx.send((0, AgentEvent::GoalUpdate {
                                summary: String::new(),
                            }));
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: format!("new session {}", session_id_local),
                            }));
                        }
                        AppCommand::ForkSession { label, at } => {
                            // Persist current tip, then fork into a new id.
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            let s = agent_loop.state();
                            let tree_snapshot =
                                serde_json::to_value(agent_loop.call_tree().snapshot()).ok();
                            let mut current = SessionData {
                                id: session_id_local.clone(),
                                title: title_local.clone(),
                                parent_id: parent_id_local.clone(),
                                branch_label: branch_label_local.clone(),
                                created_at: created_at_local.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                                call_tree: tree_snapshot,
                                todos: Some(crate::tools::todowrite::snapshot()),
                                goal: s.goal.clone(),
                                seat: s.seat.clone(),
                                handoff: s.handoff.clone().or_else(crate::agent::handoff::snapshot),
                            };
                            current.ensure_title();
                            let _ = store.save(&current);
                            match store.fork_at(&session_id_local, at, label) {
                                Ok(forked) => {
                                    session_id_local = forked.id.clone();
                                    created_at_local = forked.created_at.clone();
                                    title_local = forked.title.clone();
                                    parent_id_local = forked.parent_id.clone();
                                    branch_label_local = forked.branch_label.clone();
                                    agent_loop.state_mut().messages = forked.messages.clone();
                                    agent_loop.state_mut().goal = forked.goal.clone();
                                    agent_loop.state_mut().seat = forked.seat.clone();
                                    agent_loop.state_mut().handoff = forked.handoff.clone();
                                    crate::agent::handoff::restore(forked.handoff.clone());
                                    crate::tools::handoff::set_active_seat(forked.seat.clone());
                                    agent_loop.state_mut().pending_wake = true;
                                    let _ = event_tx.send((0, AgentEvent::SessionMeta {
                                        id: session_id_local.clone(),
                                        title: title_local.clone(),
                                    }));
                                    if let Some(ref g) = forked.goal {
                                        let _ = event_tx.send((0, AgentEvent::GoalUpdate {
                                            summary: format!("{}: {}", g.status.as_str(), g.condition),
                                        }));
                                    }
                                    let _ = event_tx.send((0, AgentEvent::ReloadTranscript {
                                        messages: forked.messages.clone(),
                                    }));
                                    let _ = event_tx.send((0, AgentEvent::TimelineSnapshot {
                                        entries: summarize_api_messages(&forked.messages),
                                    }));
                                    let _ = event_tx.send((0, AgentEvent::Status {
                                        message: format!(
                                            "forked → {} (parent {}){}",
                                            session_id_local,
                                            parent_id_local.as_deref().unwrap_or("?"),
                                            at.map(|n| format!(" @{}", n)).unwrap_or_default()
                                        ),
                                    }));
                                }
                                Err(e) => {
                                    let _ = event_tx.send((0, AgentEvent::Error {
                                        message: format!("fork failed: {e}"),
                                    }));
                                }
                            }
                        }
                        AppCommand::RequestTimeline => {
                            let entries = summarize_api_messages(&agent_loop.state().messages);
                            let _ = event_tx.send((0, AgentEvent::TimelineSnapshot { entries }));
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
                        AppCommand::SetSkillTools { tools } => {
                            let note = if tools.is_empty() {
                                agent_loop.state_mut().clear_skill_tools();
                                "skill tools cleared".to_string()
                            } else {
                                let joined = tools.join(", ");
                                agent_loop.state_mut().set_skill_tools(tools);
                                format!("skill tools: [{joined}]")
                            };
                            let _ = event_tx.send((0, AgentEvent::Status { message: note }));
                        }
                        AppCommand::SetSystemPrompt { prompt } => {
                            agent_loop.state_mut().system_prompt = prompt;
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: "system prompt rebuilt".to_string(),
                            }));
                        }
                        AppCommand::GoalSet { condition } => {
                            agent_loop.set_goal(condition.clone());
                            let _ = event_tx.send((0, AgentEvent::GoalUpdate {
                                summary: format!("active: {condition}"),
                            }));
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: "◎ /goal set — starting turn".into(),
                            }));
                        }
                        AppCommand::GoalClear => {
                            let msg = match agent_loop.clear_goal() {
                                Some(g) => format!("Goal cleared: {}", g.condition),
                                None => "No goal set".into(),
                            };
                            let _ = event_tx.send((0, AgentEvent::GoalUpdate {
                                summary: String::new(),
                            }));
                            let _ = event_tx.send((0, AgentEvent::Status { message: msg }));
                        }
                        AppCommand::GoalPause => {
                            let msg = if agent_loop.pause_goal() {
                                "Goal paused".into()
                            } else {
                                "No active goal to pause".into()
                            };
                            let _ = event_tx.send((0, AgentEvent::GoalUpdate {
                                summary: "paused".into(),
                            }));
                            let _ = event_tx.send((0, AgentEvent::Status { message: msg }));
                        }
                        AppCommand::GoalResume => {
                            let msg = if agent_loop.resume_goal() {
                                "Goal resumed".into()
                            } else {
                                "No paused goal to resume".into()
                            };
                            let summary = agent_loop
                                .state()
                                .goal
                                .as_ref()
                                .map(|g| format!("{}: {}", g.status.as_str(), g.condition))
                                .unwrap_or_default();
                            let _ = event_tx.send((0, AgentEvent::GoalUpdate { summary }));
                            let _ = event_tx.send((0, AgentEvent::Status { message: msg }));
                        }
                        AppCommand::GoalStatus => {
                            let s = agent_loop.state();
                            let msg = match &s.goal {
                                Some(g) => g.status_line(s.total_input_tokens, s.total_output_tokens),
                                None => "No goal set. Usage: /goal <condition>".into(),
                            };
                            let _ = event_tx.send((0, AgentEvent::Status { message: msg.clone() }));
                            let _ = event_tx.send((0, AgentEvent::GoalUpdate {
                                summary: format!("STATUS\n{msg}"),
                            }));
                        }
                        AppCommand::HandoffRequest => {
                            let text = crate::agent::handoff::handoff_request_message();
                            let mut prompt = text;
                            let mut wall_attempt = 0u32;
                            const MAX_WALL_RETRIES: u32 = 3;
                            loop {
                                abort_for_thread.clear();
                                let event_tx2 = event_tx.clone();
                                let mut cb = move |event: AgentEvent| {
                                    let _ = event_tx2.send((0, event));
                                };
                                let result = tokio::time::timeout(
                                    timeout,
                                    agent_loop.run(&prompt, &mut cb),
                                )
                                .await;
                                match result {
                                    Ok(Ok(())) => {
                                        let _ = event_tx.send((0, AgentEvent::TreeUpdate {
                                            tree: agent_loop.call_tree().clone(),
                                        }));
                                        break;
                                    }
                                    Ok(Err(e)) => {
                                        let _ = event_tx.send((0, AgentEvent::Error { message: e }));
                                        break;
                                    }
                                    Err(_) => {
                                        abort_for_thread.abort();
                                        let _ = agent_loop.state_mut().settle_dangling_tools();
                                        let _ = agent_loop.state_mut().repair_tool_pairing();
                                        wall_attempt += 1;
                                        if wall_attempt <= MAX_WALL_RETRIES {
                                            let delay = crate::orchestration::backoff_delay(
                                                wall_attempt.saturating_sub(1),
                                                2_000,
                                                30_000,
                                                500,
                                            );
                                            let _ = event_tx.send((0, AgentEvent::Status {
                                                message: format!(
                                                    "wall timeout after {timeout_secs}s — auto-continuing ({wall_attempt}/{MAX_WALL_RETRIES}) in {}s…",
                                                    delay.as_secs()
                                                ),
                                            }));
                                            tokio::time::sleep(delay).await;
                                            prompt = "continue".to_string();
                                            continue;
                                        }
                                        let _ = event_tx.send((0, AgentEvent::Error {
                                            message: format!(
                                                "Request timed out after {timeout_secs}s (auto-retry exhausted)"
                                            ),
                                        }));
                                        break;
                                    }
                                }
                            }
                            // Persist after handoff turn
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            let s = agent_loop.state();
                            let tree_snapshot =
                                serde_json::to_value(agent_loop.call_tree().snapshot()).ok();
                            let mut session_data = SessionData {
                                id: session_id_local.clone(),
                                title: title_local.clone(),
                                parent_id: parent_id_local.clone(),
                                branch_label: branch_label_local.clone(),
                                created_at: created_at_local.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                                call_tree: tree_snapshot,
                                todos: Some(crate::tools::todowrite::snapshot()),
                                goal: s.goal.clone(),
                                seat: s.seat.clone(),
                                handoff: s.handoff.clone().or_else(crate::agent::handoff::snapshot),
                            };
                            session_data.ensure_title();
                            title_local = session_data.title.clone();
                            let _ = store.save(&session_data);
                        }
                        AppCommand::SetSeat { name } => {
                            agent_loop.state_mut().seat = name.clone();
                            crate::tools::handoff::set_active_seat(name.clone());
                            agent_loop.state_mut().pending_wake = true;
                            let msg = match name {
                                Some(n) => format!("Seat bound: {n} — wake packet armed"),
                                None => "Seat cleared".to_string(),
                            };
                            let _ = event_tx.send((0, AgentEvent::Status { message: msg }));
                        }
                        AppCommand::LoadSession { data } => {
                            abort_for_thread.clear();
                            steer_for_thread.clear();
                            agent_loop.clear_messages();
                            session_id_local = data.id.clone();
                            created_at_local = data.created_at.clone();
                            title_local = data.title.clone();
                            parent_id_local = data.parent_id.clone();
                            branch_label_local = data.branch_label.clone();
                            if !data.system_prompt.trim().is_empty() {
                                agent_loop.state_mut().system_prompt = data.system_prompt.clone();
                            }
                            agent_loop.state_mut().seat = data.seat.clone();
                            agent_loop.state_mut().handoff = data.handoff.clone();
                            agent_loop.state_mut().goal = data.goal.clone();
                            agent_loop.state_mut().pending_wake = true;
                            crate::agent::handoff::restore(data.handoff.clone());
                            crate::tools::handoff::set_active_seat(data.seat.clone());
                            if let Some(todos) = data.todos.clone() {
                                crate::tools::todowrite::restore(todos);
                            } else {
                                crate::tools::todowrite::clear();
                            }
                            for msg in &data.messages {
                                agent_loop.state_mut().add_message(msg.clone());
                            }
                            let _ = event_tx.send((
                                0,
                                AgentEvent::SessionMeta {
                                    id: data.id.clone(),
                                    title: data.title.clone(),
                                },
                            ));
                            let _ = event_tx.send((
                                0,
                                AgentEvent::ReloadTranscript {
                                    messages: data.messages.clone(),
                                },
                            ));
                            if let Some(ref g) = data.goal {
                                let _ = event_tx.send((
                                    0,
                                    AgentEvent::GoalUpdate {
                                        summary: format!("◎ /goal {}", g.status.as_str()),
                                    },
                                ));
                            }
                            let _ = event_tx.send((
                                0,
                                AgentEvent::Status {
                                    message: format!(
                                        "loaded session {} ({} msgs) seat={:?}",
                                        data.id,
                                        data.messages.len(),
                                        data.seat
                                    ),
                                },
                            ));
                        }
                        AppCommand::PersistSession => {
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            let s = agent_loop.state();
                            let tree_snapshot =
                                serde_json::to_value(agent_loop.call_tree().snapshot()).ok();
                            let mut session_data = SessionData {
                                id: session_id_local.clone(),
                                title: title_local.clone(),
                                parent_id: parent_id_local.clone(),
                                branch_label: branch_label_local.clone(),
                                created_at: created_at_local.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                                call_tree: tree_snapshot,
                                todos: Some(crate::tools::todowrite::snapshot()),
                                goal: s.goal.clone(),
                                seat: s.seat.clone(),
                                handoff: s.handoff.clone().or_else(crate::agent::handoff::snapshot),
                            };
                            session_data.ensure_title();
                            title_local = session_data.title.clone();
                            let _ = store.save(&session_data);
                            let _ = event_tx.send((
                                0,
                                AgentEvent::Status {
                                    message: format!("persisted session {}", session_id_local),
                                },
                            ));
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
                                parent_id: parent_id_local.clone(),
                                branch_label: branch_label_local.clone(),
                                created_at: created_at_local.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                                call_tree: tree_snapshot,
                                todos: Some(crate::tools::todowrite::snapshot()),
                                goal: s.goal.clone(),
                                seat: s.seat.clone(),
                                handoff: s.handoff.clone().or_else(crate::agent::handoff::snapshot),
                            };
                            let _ = store.save(&session_data);
                            let _ = event_tx.send((0, AgentEvent::Status {
                                message: format!("renamed to \"{}\"", title),
                            }));
                        }
                        AppCommand::Submit { text } => {
                            // Wall-clock timeout on the whole turn: auto-continue a few
                            // times instead of wedging the user on "type continue".
                            const MAX_WALL_RETRIES: u32 = 3;
                            let mut prompt = text;
                            let mut wall_attempt = 0u32;
                            loop {
                                abort_for_thread.clear();
                                let result = tokio::time::timeout(
                                    timeout,
                                    agent_loop.run(&prompt, &mut |event: AgentEvent| {
                                        let _ = event_tx.send((0, event));
                                    }),
                                )
                                .await;
                                match result {
                                    Ok(Ok(())) => {
                                        let _ = event_tx.send((0, AgentEvent::TreeUpdate {
                                            tree: agent_loop.call_tree().clone(),
                                        }));
                                        break;
                                    }
                                    Ok(Err(e)) => {
                                        let transport =
                                            crate::agent::AgentLoop::is_transport_failure_msg(&e);
                                        if transport && wall_attempt < MAX_WALL_RETRIES {
                                            wall_attempt += 1;
                                            let _ = agent_loop.state_mut().settle_dangling_tools();
                                            let _ = agent_loop.state_mut().repair_tool_pairing();
                                            let delay = crate::orchestration::backoff_delay(
                                                wall_attempt.saturating_sub(1),
                                                2_000,
                                                30_000,
                                                500,
                                            );
                                            let _ = event_tx.send((0, AgentEvent::Status {
                                                message: format!(
                                                    "provider error — auto-continuing ({wall_attempt}/{MAX_WALL_RETRIES}) in {}s… ({e})",
                                                    delay.as_secs()
                                                ),
                                            }));
                                            tokio::time::sleep(delay).await;
                                            prompt = "continue".to_string();
                                            continue;
                                        }
                                        let _ = event_tx.send((0, AgentEvent::Error {
                                            message: format!("{e} (auto-retry exhausted)"),
                                        }));
                                        break;
                                    }
                                    Err(_) => {
                                        abort_for_thread.abort();
                                        let _ = agent_loop.state_mut().settle_dangling_tools();
                                        let _ = agent_loop.state_mut().repair_tool_pairing();
                                        wall_attempt += 1;
                                        if wall_attempt <= MAX_WALL_RETRIES {
                                            let delay = crate::orchestration::backoff_delay(
                                                wall_attempt.saturating_sub(1),
                                                2_000,
                                                30_000,
                                                500,
                                            );
                                            let _ = event_tx.send((0, AgentEvent::Status {
                                                message: format!(
                                                    "wall timeout after {timeout_secs}s — auto-continuing ({wall_attempt}/{MAX_WALL_RETRIES}) in {}s…",
                                                    delay.as_secs()
                                                ),
                                            }));
                                            tokio::time::sleep(delay).await;
                                            prompt = "continue".to_string();
                                            continue;
                                        }
                                        let _ = event_tx.send((0, AgentEvent::Error {
                                            message: format!(
                                                "Request timed out after {timeout_secs}s (auto-retry exhausted)"
                                            ),
                                        }));
                                        break;
                                    }
                                }
                            }
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            let s = agent_loop.state();
                            let tree_snapshot =
                                serde_json::to_value(agent_loop.call_tree().snapshot()).ok();
                            let mut session_data = SessionData {
                                id: session_id_local.clone(),
                                title: title_local.clone(),
                                parent_id: parent_id_local.clone(),
                                branch_label: branch_label_local.clone(),
                                created_at: created_at_local.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                                call_tree: tree_snapshot,
                                todos: Some(crate::tools::todowrite::snapshot()),
                                goal: s.goal.clone(),
                                seat: s.seat.clone(),
                                handoff: s.handoff.clone().or_else(crate::agent::handoff::snapshot),
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
        let path_allows = PathAllowStore::new();

        let provider_banner = if provider_name_for_banner.contains("opencode-cli") {
            format!("{} (experimental)", provider_name_for_banner)
        } else {
            provider_name_for_banner.clone()
        };
        let mut initial_msgs = vec![ChatMessage {
            role: "system".to_string(),
            text: format!(
                "**rs-agent** · Deep Context coding agent\n\
                 `{provider}` / `{model}` · session `{session}`\n\n\
                 Type to start · `/` commands · `@` files · `#` dirs\n\
                 Header chip: ○ idle · ◐ working · ● blocked · ✓ done\n\
                     Esc abort (kills bash) · Tab cycle tool outputs · Enter steer while working · `/tree` call graph",
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
        } else {
            crate::tools::todowrite::clear();
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
            pending_question: None,
            question_rx,
            trust_store,
            path_allows,
            approved: approve,
            auto_mode,
            token_used: 0,
            token_limit: crate::ai::token_count::get_context_limit(&model),
            session_input_tokens: resume
                .as_ref()
                .map(|s| s.total_input_tokens)
                .unwrap_or(0),
            session_output_tokens: resume
                .as_ref()
                .map(|s| s.total_output_tokens)
                .unwrap_or(0),
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
            pending_kitty_images: Vec::new(),
            lsp_summary: String::new(),
            lsp_cmd_tx: Some(lsp_cmd_tx),
            fleet_attach: None,
            unseen_done: false,
            beads_ready_cache: (Instant::now() - Duration::from_secs(60), 0),
            toast: None,
            toast_enabled: cfg.toast.unwrap_or(true),
            toast_sound: cfg.toast_sound.unwrap_or(false),
            notify_mode: NotifyMode::parse(cfg.notify.as_deref().unwrap_or("off")),
            help_overlay: None,
            settings: None,
            palette_open: false,
            palette_query: String::new(),
            palette_selection: 0,
            palette_items: Vec::new(),
            show_fleet_panel: false,
            fleet_panel: FleetPanelState::default(),
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
            allowed_transitions: cfg.allowed_transitions.clone(),
            fleet_refresh_at: Instant::now() - Duration::from_secs(60),
        }
    }

    fn push_toast(&mut self, toast: Toast) {
        if !self.toast_enabled {
            return;
        }
        if self.toast_sound {
            toast::play_sound(toast.kind);
        }
        let life = match toast.kind {
            toast::ToastKind::NeedsAttention => Lifecycle::Blocked,
            toast::ToastKind::Finished => Lifecycle::Done,
        };
        notify::on_lifecycle(self.notify_mode, life, &toast.body);
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

    fn publish_lifecycle(&mut self, life: Lifecycle, detail: &str) {
        let changed = lifecycle::publish(life, detail);
        lifecycle::set_session(Some(self.session_id.clone()), None);
        if changed {
            match life {
                Lifecycle::Blocked => {
                    self.push_toast(Toast::blocked("needs attention", detail));
                }
                Lifecycle::Done => {
                    if self.unseen_done {
                        self.push_toast(Toast::finished("done", detail));
                    }
                }
                _ => {}
            }
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
            self.flush_pending_kitty_images();

            if event::poll(Duration::from_millis(10))? {
                self.handle_event(event::read()?)?;
            }

            while let Ok((_idx, event)) = self.event_rx.try_recv() {
                self.handle_agent_event(event);
            }

            self.poll_fleet_attach();

            self.poll_model_picker();

            if let Ok(pending) = self.permission_rx.try_recv() {
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let is_trusted = self.approved || self.trust_store.is_trusted(&cwd);
                let path_ok = {
                    let target = extract_tool_path(&pending.request.tool_input);
                    self.path_allows.allows(
                        &cwd,
                        &pending.request.tool_name,
                        target.as_deref(),
                    )
                };
                if is_trusted || path_ok || self.auto_allow(&pending.request.tool_name) {
                    let _ = pending.reply_tx.send(PermissionReply::AllowOnce);
                } else {
                    let tool = pending.request.tool_name.clone();
                    self.pending_permission = Some(pending);
                    self.publish_lifecycle(Lifecycle::Blocked, &format!("permission: {tool}"));
                }
            }

            if let Ok(pending) = self.question_rx.try_recv() {
                self.input.clear();
                self.input_mode = InputMode::Question;
                self.status = "awaiting answer...".to_string();
                let q = pending.request.question.clone();
                self.pending_question = Some(pending);
                self.publish_lifecycle(Lifecycle::Blocked, &format!("question: {q}"));
            }

            if let Some(t) = self.toast.as_ref() {
                if t.expired() {
                    self.toast = None;
                }
            }
            if self.show_fleet_panel && self.fleet_refresh_at.elapsed() >= Duration::from_secs(1) {
                self.fleet_panel.refresh();
                self.fleet_refresh_at = Instant::now();
            }

            if self.should_exit {
                break;
            }
        }

        // Resume any paused fleet seat so workers are not wedged.
        self.fleet_detach(true);

        let _ = self.command_tx.send(AppCommand::Exit);
        terminal::disable_raw_mode()?;
        crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableBracketedPaste)?;
        if self.mouse_enabled {
            crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
        }
        // Alternate screen is gone — print a durable resume hint.
        let short = SessionStore::short_id(&self.session_id);
        let title = self
            .session_title
            .as_deref()
            .unwrap_or("untitled");
        eprintln!("Session saved: {} ({title})", self.session_id);
        eprintln!("Resume:  rs-agent -r {short}");
        eprintln!("Or:      rs-agent -r latest");
        eprintln!("List:    rs-agent --list-sessions");
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
                self.publish_lifecycle(Lifecycle::Working, "thinking");
            }
            AgentEvent::ToolUseStart { id, name } => {
                self.status = format!("using {}...", name);
                self.follow_bottom = true;
                self.publish_lifecycle(Lifecycle::Working, &format!("tool:{name}"));
                if name == "repl" || name == "bash" || name == "task" {
                    let n = self.tool_output_tabs.len() + 1;
                    self.tool_output_tabs.push(ToolOutputTab {
                        id: id.clone(),
                        name: name.clone(),
                        label: format!("{name}#{n}"),
                        buffer: String::new(),
                        done: false,
                    });
                    self.tool_output_tab = self.tool_output_tabs.len().saturating_sub(1);
                    // Bottom console only — never steal chat width mid-turn.
                    self.show_repl_panel = true;
                }
                self.tool_in_progress = Some((name, Instant::now()));
            }
            AgentEvent::ToolResult { id, name, result } => {
                self.tool_in_progress = None;
                if let Some(tab) = self.tool_output_tabs.iter_mut().find(|t| t.id == id) {
                    tab.done = true;
                    if !result.content.is_empty() && tab.buffer.is_empty() {
                        tab.buffer = result.content.chars().take(4000).collect();
                    }
                }
                let mut full = result.content.clone();
                if full.starts_with("Exit code: ") {
                    if let Some(rest) = full.splitn(2, '\n').nth(1) {
                        full = rest.to_string();
                    }
                }
                for img in super::kitty::find_image_paths(&full) {
                    if !self.pending_kitty_images.iter().any(|p| p == &img) {
                        self.pending_kitty_images.push(img);
                    }
                }
                if matches!(name.as_str(), "write" | "edit" | "apply_patch") {
                    if let Some(path) = extract_saved_file_path(&full) {
                        if let Ok(text) = std::fs::read_to_string(&path) {
                            if let Some(ref tx) = self.lsp_cmd_tx {
                                let _ = tx.send(LspCmd::DidSave { path, text });
                            }
                        }
                    }
                }
                let preview: String = if full.contains("<diagnostics") {
                    let first_diag = full
                        .lines()
                        .find(|l| {
                            let t = l.trim_start();
                            t.starts_with("error ") || t.starts_with("warn ")
                        })
                        .unwrap_or("LSP diagnostics");
                    format!("⚠ {}", first_diag.chars().take(90).collect::<String>())
                } else if full.contains(crate::tools::truncate_store::SPILL_MARKER) {
                    let path_line = full
                        .lines()
                        .find(|l| l.contains(crate::tools::truncate_store::SPILL_MARKER))
                        .unwrap_or("full output spilled");
                    format!("… {}", path_line.chars().take(90).collect::<String>())
                } else if matches!(name.as_str(), "bash" | "repl" | "task") {
                    // Keep chat clean — live stream lives in the bottom console.
                    let exit = full
                        .lines()
                        .rev()
                        .find(|l| l.starts_with("Exit code:"))
                        .unwrap_or("done");
                    format!("↗ console · {exit}")
                } else if matches!(name.as_str(), "write" | "edit" | "apply_patch") {
                    let head: String = full.lines().next().unwrap_or("").chars().take(80).collect();
                    let lsp = if full.contains("<diagnostics") {
                        " · LSP!"
                    } else {
                        ""
                    };
                    format!("{head}{lsp}")
                } else {
                    full.chars().take(100).collect()
                };
                let expand_diag = full.contains("<diagnostics");
                let expand_spill = full.contains(crate::tools::truncate_store::SPILL_MARKER);
                // Tools belong on the assistant turn. If the model jumped
                // straight to a tool with no prior text/thinking, open one.
                if self.messages.last().map(|m| m.role.as_str()) != Some("assistant") {
                    self.messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        text: String::new(),
                        thinking: None,
                        show_thinking: false,
                        tool_blocks: Vec::new(),
                    });
                }
                if let Some(last) = self.messages.last_mut() {
                    last.tool_blocks.push(ToolBlock {
                        name,
                        preview,
                        full: full.clone(),
                        expanded: expand_diag || expand_spill || result.is_error,
                        is_error: result.is_error || expand_diag,
                    });
                }
                self.follow_bottom = true;
                if result.is_error && full.contains("STUCK:") {
                    self.status = "STUCK".to_string();
                }
            }
            AgentEvent::Error { message } => {
                let transport = message.to_lowercase().contains("stream error")
                    || message.to_lowercase().contains("error sending request")
                    || message.to_lowercase().contains("timed out")
                    || message.to_lowercase().contains("timeout")
                    || message.to_lowercase().contains("connection");
                let exhausted = message.to_lowercase().contains("auto-retry exhausted");
                if transport && !exhausted && self.provider_auto_continues < 3 {
                    self.provider_auto_continues =
                        self.provider_auto_continues.saturating_add(1);
                    let n = self.provider_auto_continues;
                    self.push_system(format!(
                        "Provider hiccup: {message}\nAuto-continuing ({n}/3)…"
                    ));
                    self.status = "thinking...".to_string();
                    self.tool_in_progress = None;
                    self.input_mode = InputMode::Waiting;
                    let _ = self.command_tx.send(AppCommand::Submit {
                        text: "continue".into(),
                    });
                } else if transport {
                    self.provider_auto_continues = 0;
                    self.push_system(format!(
                        "Provider hiccup: {message}\n\
                         Auto-retry exhausted — type continue or /handoff (session stayed open)."
                    ));
                    self.status = "recover".to_string();
                    self.input_mode = InputMode::Insert;
                    self.tool_in_progress = None;
                } else if let Some(last) = self.messages.last_mut() {
                    last.text.push_str(&format!("\n❌ Error: {}", message));
                    self.status = "error".to_string();
                    self.input_mode = InputMode::Insert;
                    self.tool_in_progress = None;
                } else {
                    self.status = "error".to_string();
                    self.input_mode = InputMode::Insert;
                    self.tool_in_progress = None;
                }
            }
            AgentEvent::TurnEnd { stop_reason: _ } => {
                self.status = "ready".to_string();
                self.unseen_done = true;
                self.publish_lifecycle(Lifecycle::Done, "turn end");
            }
            AgentEvent::Done => {
                self.provider_auto_continues = 0;
                self.status = "ready".to_string();
                self.unseen_done = true;
                self.input_mode = InputMode::Insert;
                self.input.clear();
                self.near_limit = false;
                self.queued_steers = 0;
                self.tool_in_progress = None;
                self.publish_lifecycle(Lifecycle::Done, "ready");
            }
            AgentEvent::GoalUpdate { summary } => {
                if summary.starts_with("STATUS\n") {
                    self.push_system(summary.trim_start_matches("STATUS\n").to_string());
                } else if summary.is_empty() {
                    self.goal_indicator.clear();
                } else if summary.starts_with("achieved") {
                    self.goal_indicator = "◎ goal achieved".into();
                    self.push_system(format!("◎ {summary}"));
                } else if summary.starts_with("paused") {
                    self.goal_indicator = "◎ /goal paused".into();
                } else if summary.starts_with("active") {
                    self.goal_indicator = "◎ /goal active".into();
                } else {
                    self.goal_indicator = format!("◎ /goal {summary}");
                }
            }
            AgentEvent::ToolUseDelta { input: _ } => {}
            AgentEvent::ReplOutput { stream, text } => {
                let cleaned = Self::sanitize_console_text(&text);
                let mut chunk = String::new();
                for line in cleaned.lines() {
                    if stream == "stderr" {
                        chunk.push_str("! ");
                    }
                    chunk.push_str(line);
                    chunk.push('\n');
                }
                self.append_tool_tab_output("repl", &chunk);
                self.show_repl_panel = true;
            }
            AgentEvent::ToolOutput { name, stream, text } => {
                let cleaned = Self::sanitize_console_text(&text);
                let mut chunk = String::new();
                for line in cleaned.lines() {
                    if stream == "stderr" {
                        chunk.push_str("! ");
                    }
                    chunk.push_str(line);
                    chunk.push('\n');
                }
                self.append_tool_tab_output(&name, &chunk);
                self.show_repl_panel = true;
            }
            AgentEvent::ContextWarning { fraction: _, used, limit } => {
                self.token_used = used;
                self.token_limit = limit;
                self.near_limit = true;
            }
            AgentEvent::TokenUpdate {
                used,
                limit,
                input_tokens,
                output_tokens,
            } => {
                self.token_used = used;
                self.token_limit = limit;
                self.session_input_tokens = input_tokens;
                self.session_output_tokens = output_tokens;
            }
            AgentEvent::Compacting => {
                self.status = "compacting...".to_string();
            }
            AgentEvent::Compacted { summary: _ } => {
                self.status = "compacted".to_string();
                self.near_limit = false;
            }
            AgentEvent::Status { message } => {
                if message.contains("stream failed") || message.contains("recovering") {
                    if message.contains("stream failed") && message.contains("session ready") {
                        if self.provider_auto_continues < 3 {
                            self.provider_auto_continues =
                                self.provider_auto_continues.saturating_add(1);
                            let n = self.provider_auto_continues;
                            self.push_system(format!(
                                "stream failed — auto-continuing ({n}/3)…"
                            ));
                            self.input_mode = InputMode::Waiting;
                            self.tool_in_progress = None;
                            self.status = "thinking...".to_string();
                            let _ = self.command_tx.send(AppCommand::Submit {
                                text: "continue".into(),
                            });
                        } else {
                            self.provider_auto_continues = 0;
                            self.push_system(
                                "stream failed — auto-retry exhausted; type continue or /handoff",
                            );
                            self.input_mode = InputMode::Insert;
                            self.tool_in_progress = None;
                        }
                    } else {
                        self.input_mode = InputMode::Waiting;
                        self.tool_in_progress = None;
                    }
                }
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
                self.tree_nodes = tree_view::parse_text_tree(&self.tree_panel_text);
            }
            AgentEvent::TitleUpdate { title } => {
                if title.is_empty() {
                    self.session_title = None;
                } else {
                    self.session_title = Some(title);
                }
            }
            AgentEvent::SessionMeta { id, title } => {
                self.session_id = id;
                self.session_title = title.filter(|t| !t.is_empty());
                self.session_input_tokens = 0;
                self.session_output_tokens = 0;
            }
            AgentEvent::ReloadTranscript { messages } => {
                let opener = self
                    .messages
                    .first()
                    .filter(|m| m.role == "system")
                    .cloned();
                let mut rebuilt = Vec::new();
                if let Some(sys) = opener {
                    rebuilt.push(sys);
                }
                rebuilt.extend(api_messages_to_chat(&messages));
                self.messages = rebuilt;
                self.timeline_entries = summarize_api_messages(&messages);
                if self.timeline_selection >= self.timeline_entries.len() {
                    self.timeline_selection = self.timeline_entries.len().saturating_sub(1);
                }
                self.follow_bottom = true;
            }
            AgentEvent::TimelineSnapshot { entries } => {
                self.timeline_entries = entries;
                if self.timeline_selection >= self.timeline_entries.len() {
                    self.timeline_selection = self.timeline_entries.len().saturating_sub(1);
                }
            }
            AgentEvent::LspUpdate { summary } => {
                self.lsp_summary = summary;
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
                        if let Some(target) = self.hit_map.hit_at(mouse.column, mouse.row).cloned() {
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
                                HitTarget::FleetRow { index } => {
                                    if index < self.fleet_panel.rows.len()
                                        && self.fleet_panel.rows[index].selectable()
                                    {
                                        self.fleet_panel.selection = index;
                                        self.fleet_panel.expanded = true;
                                        self.activate_city_selection();
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
            },
            Event::Paste(text) => {
                if self.pending_permission.is_some() {
                    // ignore paste while a permission prompt is up
                } else if self.picker_active {
                    self.picker_query.push_str(text.replace(['\n', '\r'], "").as_str());
                    self.update_picker_results();
                } else {
                    match self.input_mode {
                        InputMode::Insert
                        | InputMode::Waiting
                        | InputMode::ApiKey
                        | InputMode::Question => {
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
            KeyCode::Tab => {
                self.cycle_tool_output_tab(1);
            }
            KeyCode::BackTab => {
                self.cycle_tool_output_tab(-1);
            }
            KeyCode::Up | KeyCode::Char('k') if self.show_repl_panel => {
                self.cycle_tool_output_tab(-1);
            }
            KeyCode::Down | KeyCode::Char('j') if self.show_repl_panel => {
                self.cycle_tool_output_tab(1);
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

    fn cycle_tool_output_tab(&mut self, delta: i32) {
        if self.tool_output_tabs.is_empty() {
            self.show_repl_panel = true;
            self.status = "no tool outputs yet".into();
            return;
        }
        self.show_repl_panel = true;
        let n = self.tool_output_tabs.len() as i32;
        let cur = self.tool_output_tab as i32;
        self.tool_output_tab = ((cur + delta).rem_euclid(n)) as usize;
        self.status = format!(
            "console {}/{} · {}",
            self.tool_output_tab + 1,
            self.tool_output_tabs.len(),
            self.tool_output_tabs[self.tool_output_tab].label
        );
    }

    fn append_tool_tab_output(&mut self, name: &str, chunk: &str) {
        if let Some(idx) = self
            .tool_output_tabs
            .iter()
            .rposition(|t| t.name == name && !t.done)
            .or_else(|| self.tool_output_tabs.iter().rposition(|t| t.name == name))
        {
            self.tool_output_tabs[idx].buffer.push_str(chunk);
            Self::trim_panel_utf8(&mut self.tool_output_tabs[idx].buffer, 16_000);
            self.tool_output_tab = idx;
            self.show_repl_panel = true;
        } else {
            // Fallback single buffer if a tab was not opened.
            self.repl_panel.push_str(chunk);
            Self::trim_panel_utf8(&mut self.repl_panel, 16_000);
            self.show_repl_panel = true;
        }
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

    fn push_fleet_log_line(&mut self, line: &crate::fleet::ParsedLogLine) {
        use crate::fleet::LogKind;
        let prefix = match line.kind {
            LogKind::Tool => "⚙",
            LogKind::ToolResult => "↳",
            LogKind::Say => "💬",
            LogKind::Heartbeat => "♥",
            LogKind::Claimed => "▶",
            LogKind::Closed => "✓",
            LogKind::Session => "◉",
            LogKind::Error => "✗",
            LogKind::Status => "·",
            LogKind::Raw => "·",
        };
        let ts = line
            .timestamp
            .as_deref()
            .map(|t| format!("[{t}] "))
            .unwrap_or_default();
        self.messages.push(ChatMessage {
            role: match line.kind {
                LogKind::Say => "assistant".to_string(),
                LogKind::Tool | LogKind::ToolResult => "tool".to_string(),
                LogKind::Error => "system".to_string(),
                _ => "system".to_string(),
            },
            text: format!("{prefix} {ts}{}", line.body),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        });
        self.follow_bottom = true;
    }

    fn fleet_start_follow(&mut self, seat: &str) {
        if self.fleet_attach.is_some() {
            self.fleet_detach(true);
        }
        let (follower, initial) = crate::fleet::LogFollower::from_tail(seat, 40);
        self.push_system(format!(
            "Following `{seat}` — live log (worker keeps running).\n\
             Steer: /seat steer <text>   Abort turn: /seat abort   Stop: /seat detach\n\
             Take over: /seat attach {seat}"
        ));
        for line in &initial {
            self.push_fleet_log_line(line);
        }
        self.fleet_attach = Some(FleetAttachState {
            seat: seat.to_string(),
            phase: FleetAttachPhase::Follow,
            follower,
            worker_session_id: None,
            prior_session_id: None,
            attach_started: Instant::now(),
        });
        self.status = format!("following {seat}");
        self.push_system(crate::fleet::format_city_board(Some(seat)));
    }

    fn fleet_start_attach(&mut self, seat: &str) {
        if self.fleet_attach.is_some() {
            self.fleet_detach(true);
        }
        let st = crate::fleet::read_seat_status(seat);
        if st
            .as_ref()
            .map(|s| !s.running && s.state == "stopped")
            .unwrap_or(false)
        {
            self.push_system(format!(
                "Seat `{seat}` does not look running. Start with /fleet up first."
            ));
        }
        crate::fleet::append_control(seat, crate::fleet::ControlOp::Pause, Some("tui attach"));
        let (follower, initial) = crate::fleet::LogFollower::from_tail(seat, 30);
        self.push_system(format!(
            "Attaching to `{seat}` — pausing worker… (Esc abort once attached; /seat detach to return)"
        ));
        for line in &initial {
            self.push_fleet_log_line(line);
        }
        self.fleet_attach = Some(FleetAttachState {
            seat: seat.to_string(),
            phase: FleetAttachPhase::Attaching,
            follower,
            worker_session_id: st.and_then(|s| s.session_id),
            prior_session_id: Some(self.session_id.clone()),
            attach_started: Instant::now(),
        });
        self.status = format!("attaching {seat}…");
    }

    /// Load a worker session, retrying briefly (file may appear just after status updates).
    fn load_worker_session_retry(sid: &str) -> Result<SessionData, String> {
        let store = SessionStore::new();
        let mut last = String::new();
        for attempt in 0..6 {
            match store.load(sid) {
                Ok(data) => return Ok(data),
                Err(e) => {
                    last = e;
                    if attempt + 1 < 6 {
                        std::thread::sleep(Duration::from_millis(400));
                    }
                }
            }
        }
        Err(format!(
            "{last}\n\
             Tip: worker advertises session_id before the file exists on old binaries — \
             `fleet down` + `fleet up` with the latest rs-agent, or wait for the turn to flush. \
             Path: {}",
            store.session_path(sid)
        ))
    }

    /// Inspect session without pausing the worker (read-only).
    fn fleet_open_inspect(&mut self, seat: &str) {
        if self.fleet_attach.as_ref().map(|a| a.phase) == Some(FleetAttachPhase::Attached)
            || self.fleet_attach.as_ref().map(|a| a.phase) == Some(FleetAttachPhase::Attaching)
        {
            self.push_system("Detach first (`/seat detach`) before opening another seat.");
            return;
        }
        if self.fleet_attach.is_some() {
            self.fleet_detach(true);
        }
        self.push_system(crate::fleet::format_seat_card(seat));
        let st = crate::fleet::read_seat_status(seat);
        let sid = st.as_ref().and_then(|s| s.session_id.clone());
        let Some(sid) = sid else {
            self.push_system(format!(
                "No session_id for `{seat}` yet — follow logs with `/seat follow {seat}`."
            ));
            return;
        };
        match Self::load_worker_session_retry(&sid) {
            Ok(data) => {
                let (follower, _) = crate::fleet::LogFollower::from_tail(seat, 0);
                self.fleet_attach = Some(FleetAttachState {
                    seat: seat.to_string(),
                    phase: FleetAttachPhase::Inspect,
                    follower,
                    worker_session_id: Some(sid.clone()),
                    prior_session_id: Some(self.session_id.clone()),
                    attach_started: Instant::now(),
                });
                self.push_system(format!(
                    "INSPECT `{seat}` session {sid} (worker still running — chat disabled).\n\
                     /seat follow {seat} · /seat attach {seat} · /seat detach"
                ));
                let _ = self.command_tx.send(AppCommand::LoadSession { data });
                self.status = format!("inspect {seat}");
            }
            Err(e) => self.push_system(format!("Failed to load session {sid}: {e}")),
        }
    }

    fn fleet_steer_remote(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            self.push_system("Usage: /seat steer <instruction>");
            return;
        }
        let Some(attach) = &self.fleet_attach else {
            self.push_system("Follow or attach a seat first: /seat follow <seat>");
            return;
        };
        let seat = attach.seat.clone();
        match attach.phase {
            FleetAttachPhase::Follow | FleetAttachPhase::Attaching => {
                crate::fleet::append_control(
                    &seat,
                    crate::fleet::ControlOp::Steer,
                    Some(text),
                );
                self.push_system(format!("Steer → `{seat}`: {text}"));
            }
            FleetAttachPhase::Attached => {
                // Local steer queue (same as Waiting-mode Enter).
                self.steer_queue.push(text.to_string());
                let _ = self.command_tx.send(AppCommand::Steer {
                    text: text.to_string(),
                });
                self.push_system(format!("Steer (attached): {text}"));
            }
            FleetAttachPhase::Inspect => {
                self.push_system(
                    "Inspect is read-only. `/seat follow` + `/seat steer …`, or `/seat attach`.",
                );
            }
        }
    }

    fn fleet_abort_remote(&mut self) {
        let Some(attach) = &self.fleet_attach else {
            self.push_system("Follow or attach a seat first: /seat follow <seat>");
            return;
        };
        let seat = attach.seat.clone();
        match attach.phase {
            FleetAttachPhase::Follow | FleetAttachPhase::Attaching => {
                crate::fleet::append_control(&seat, crate::fleet::ControlOp::Abort, None);
                self.push_system(format!("Abort → `{seat}` (worker turn)"));
            }
            FleetAttachPhase::Attached => {
                self.abort_flag.abort();
                let _ = self.command_tx.send(AppCommand::Abort);
                self.push_system("Abort (attached turn)");
            }
            FleetAttachPhase::Inspect => {
                self.push_system("Inspect is read-only. `/seat follow` then `/seat abort`.");
            }
        }
    }

    fn fleet_complete_attach(&mut self) {
        let Some(attach) = self.fleet_attach.as_mut() else {
            return;
        };
        if attach.phase != FleetAttachPhase::Attaching {
            return;
        }
        let seat = attach.seat.clone();
        let st = crate::fleet::read_seat_status(&seat);
        let session_id = st
            .as_ref()
            .and_then(|s| s.session_id.clone())
            .or_else(|| attach.worker_session_id.clone());
        let Some(sid) = session_id else {
            if attach.attach_started.elapsed() > Duration::from_secs(90) {
                self.push_system(format!(
                    "Attach to `{seat}` timed out waiting for session_id (is the worker alive?)."
                ));
                self.fleet_attach = None;
                crate::fleet::append_control(&seat, crate::fleet::ControlOp::Resume, None);
            }
            return;
        };
        let paused = st
            .as_ref()
            .map(|s| s.state == "paused" || s.state == "attached")
            .unwrap_or(false);
        if !paused {
            if attach.attach_started.elapsed() > Duration::from_secs(120) {
                self.push_system(format!(
                    "Attach to `{seat}` timed out waiting for pause. Sent resume."
                ));
                crate::fleet::append_control(&seat, crate::fleet::ControlOp::Resume, None);
                self.fleet_attach = None;
            }
            return;
        }
        match Self::load_worker_session_retry(&sid) {
            Ok(data) => {
                attach.phase = FleetAttachPhase::Attached;
                attach.worker_session_id = Some(sid.clone());
                if let Some(mut status) = crate::fleet::read_seat_status(&seat) {
                    status.state = "attached".into();
                    status.paused_reason = Some("tui attached".into());
                    crate::fleet::write_seat_status(&status);
                }
                let bead = st
                    .as_ref()
                    .and_then(|s| s.last_bead.clone())
                    .unwrap_or_else(|| "-".into());
                self.push_system(format!(
                    "ATTACHED `{seat}` · bead {bead} · session {sid}\n\
                     Chat to continue as this seat. Esc aborts the turn. /seat detach returns control."
                ));
                let _ = self.command_tx.send(AppCommand::LoadSession { data });
                self.status = format!("ATTACHED {seat}");
                self.input_mode = InputMode::Insert;
            }
            Err(e) => {
                self.push_system(format!(
                    "Paused `{seat}` but failed to load session `{sid}`: {e}. Detaching."
                ));
                crate::fleet::append_control(&seat, crate::fleet::ControlOp::Resume, None);
                self.fleet_attach = None;
            }
        }
    }

    /// `quiet`: skip chatter when switching follow→attach.
    fn fleet_detach(&mut self, quiet: bool) {
        let Some(attach) = self.fleet_attach.take() else {
            if !quiet {
                self.push_system("Not attached to a fleet seat.");
            }
            return;
        };
        let seat = attach.seat;
        match attach.phase {
            FleetAttachPhase::Attached | FleetAttachPhase::Attaching => {
                let _ = self.command_tx.send(AppCommand::PersistSession);
                crate::fleet::append_control(&seat, crate::fleet::ControlOp::Resume, None);
                if let Some(mut st) = crate::fleet::read_seat_status(&seat) {
                    crate::fleet::clear_paused(&mut st);
                    st.state = "idle".into();
                    crate::fleet::write_seat_status(&st);
                }
                if !quiet {
                    self.push_system(format!(
                        "Detached from `{seat}` — worker resume signaled. Session saved."
                    ));
                }
            }
            FleetAttachPhase::Follow => {
                if !quiet {
                    self.push_system(format!("Stopped following `{seat}`."));
                }
            }
            FleetAttachPhase::Inspect => {
                if !quiet {
                    self.push_system(format!("Closed inspect view for `{seat}`."));
                }
            }
        }
        self.status = "ready".to_string();
    }

    fn city_highlight_seat(&self) -> Option<&str> {
        self.fleet_attach.as_ref().map(|a| a.seat.as_str())
    }

    fn show_city_board(&mut self) {
        let highlight = self.city_highlight_seat().map(|s| s.to_string());
        self.push_system(crate::fleet::format_city_board(highlight.as_deref()));
    }

    fn toggle_city_panel(&mut self) {
        self.show_fleet_panel = !self.show_fleet_panel;
        if self.show_fleet_panel {
            self.show_timeline_panel = false;
            self.show_tree_panel = false;
            self.fleet_panel.refresh();
            self.push_system(
                "City panel — WORKERS / WISHES / READY\n\
                 ↑↓ select · Enter follow worker or inspect bead · x expand · /fleet up to spawn",
            );
        } else {
            self.push_system("City panel hidden");
        }
    }

    fn activate_city_selection(&mut self) {
        match self.fleet_panel.selected_row().cloned() {
            Some(CityRow::Worker { seat }) => {
                let _ = self.handle_slash_command(&format!("/seat follow {}", seat.seat));
            }
            Some(CityRow::Wish { bead }) | Some(CityRow::Ready { bead }) => {
                self.push_system(format!(
                    "Bead {} [{}] {}\nstatus={} priority={}\n{}",
                    bead.id,
                    bead.kind.as_str(),
                    bead.title,
                    bead.status.as_str(),
                    bead.priority,
                    bead.notes.chars().take(240).collect::<String>()
                ));
            }
            _ => {
                self.push_system("Nothing selected — ↑↓ to a worker, wish, or ready bead.");
            }
        }
    }

    /// Handle `/seat …` and shared fleet aliases. Returns true if handled.
    fn handle_seat_ops(&mut self, raw: &str) -> bool {
        let raw = raw.trim();
        let lower = raw.to_lowercase();
        if lower.is_empty() || lower == "status" || lower == "board" {
            self.show_city_board();
            return true;
        }
        if lower == "detach" {
            self.fleet_detach(false);
            return true;
        }
        if let Some(rest) = lower.strip_prefix("follow ") {
            let seat = raw.split_whitespace().nth(1).unwrap_or(rest.trim());
            if seat.is_empty() {
                self.push_system("Usage: /seat follow <seat>");
            } else {
                self.fleet_start_follow(seat);
            }
            return true;
        }
        if let Some(rest) = lower.strip_prefix("attach ") {
            let seat = raw.split_whitespace().nth(1).unwrap_or(rest.trim());
            if seat.is_empty() {
                self.push_system("Usage: /seat attach <seat>");
            } else {
                self.fleet_start_attach(seat);
            }
            return true;
        }
        if let Some(rest) = lower
            .strip_prefix("open ")
            .or_else(|| lower.strip_prefix("inspect "))
        {
            let seat = raw.split_whitespace().nth(1).unwrap_or(rest.trim());
            if seat.is_empty() {
                self.push_system("Usage: /seat open <seat>");
            } else {
                self.fleet_open_inspect(seat);
            }
            return true;
        }
        if let Some(rest) = lower.strip_prefix("steer ") {
            // Preserve original casing for steer text from raw after first token.
            let text = raw
                .split_once(char::is_whitespace)
                .map(|(_, r)| r.trim())
                .unwrap_or(rest.trim());
            self.fleet_steer_remote(text);
            return true;
        }
        if lower == "abort" {
            self.fleet_abort_remote();
            return true;
        }
        if let Some(rest) = lower.strip_prefix("logs ").or_else(|| lower.strip_prefix("log ")) {
            let seat = raw.split_whitespace().nth(1).unwrap_or(rest.trim());
            if seat.is_empty() {
                self.push_system("Usage: /seat logs <seat>");
            } else {
                self.push_system(crate::fleet::fleet_logs(seat, 60));
            }
            return true;
        }
        self.push_system(
            "Usage: /seat | /seat follow|attach|detach|open|steer|abort|logs <seat>\n\
             Also: /city   (board)   /fleet up|down   aliases work for follow/attach/…",
        );
        true
    }

    fn poll_fleet_attach(&mut self) {
        // Drain log follower.
        let lines = {
            let Some(attach) = self.fleet_attach.as_mut() else {
                return;
            };
            attach.follower.poll()
        };
        for line in &lines {
            self.push_fleet_log_line(line);
        }
        let phase = self.fleet_attach.as_ref().map(|a| a.phase);
        if phase == Some(FleetAttachPhase::Attaching) {
            self.fleet_complete_attach();
        }
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
                     /help /keys /settings /clear /context [on|off] /commands /tree\n\
                     /skills  /skill <name>  /prompt|/p <name> [args]  /reload\n\
                     /mode plan|ask|agent  /model [provider/model]  /provider|/login [name]\n\
                     /goal [condition|clear|pause|resume]  /theme [dark|light|forest]\n\
                     /handoff  /route <seat>  /seat […]  /beads [ready]  /laurel <text>\n\
                     /worker [seat]  /marshal  /city  /fleet [panel|status|up|down]\n\
                     /detach  /mail [send|ack]  /wish …  /moot …  /brain remember|falsify <…>\n\
                     /compact  /new  /fork [@N] [label]  /timeline  /sessions\n\
                     /export [md|json|html]  /image [path]  /lsp [start|stop|status]  /skill-pack export|import\n\
                     /revert  /trust list|reset  /rename <title>  /history [query|n]\n\n\
                     Keys: {}\n\
                     Ctrl+P cycle model · Tab-complete /skill|/prompt|/model|/theme|/mode|/provider · @ file · # dir",
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
                     Tab        complete /skill /prompt /model /theme /mode /provider\n\
                     ^P         cycle provider/model (ready providers)\n\
                     {once}/{deny}/{always}/{path}  once / deny / trust project / allow path\n\
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
                    path = self.keys.binding("perm_path"),
                ));
            }
            "/theme" => {
                if arg.is_empty() {
                    self.push_system(format!(
                        "Current theme: {}\nUsage: /theme dark|light|forest|auto",
                        self.theme_name.as_str()
                    ));
                } else {
                    self.theme_name = ThemeName::parse(arg);
                    self.palette = Palette::for_theme(self.theme_name);
                    self.push_system(format!("Theme set to {}", self.theme_name.as_str()));
                }
            }
            "/settings" => {
                self.settings = Some(SettingsState::from_app(
                    self.theme_name,
                    self.mouse_enabled,
                    self.toast_enabled,
                    self.toast_sound,
                    self.notify_mode.as_str(),
                ));
            }
            "/route" | "/handoff-route" => {
                // Continuity notes stay on /handoff; this routes control to another seat.
                let mut parts = arg.split_whitespace();
                let to = parts.next().unwrap_or("");
                let reason = parts.collect::<Vec<_>>().join(" ");
                if to.is_empty() {
                    self.push_system("Usage: /route <seat> [reason]");
                } else {
                    match crate::agent::handoff::route_to_seat(
                        None,
                        to,
                        if reason.is_empty() { "routed from TUI" } else { &reason },
                        &self.allowed_transitions,
                    ) {
                        Ok(rec) => self.push_system(rec.format_block()),
                        Err(e) => self.push_system(e),
                    }
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
                self.goal_indicator.clear();
                let _ = self.command_tx.send(AppCommand::GoalClear);
                self.push_system("Chat cleared (session kept). Use /new for a fresh session.");
            }
            "/goal" => {
                match crate::agent::parse_goal_arg(arg) {
                    Ok(crate::agent::GoalCommand::Status) => {
                        let _ = self.command_tx.send(AppCommand::GoalStatus);
                    }
                    Ok(crate::agent::GoalCommand::Clear) => {
                        self.goal_indicator.clear();
                        let _ = self.command_tx.send(AppCommand::GoalClear);
                    }
                    Ok(crate::agent::GoalCommand::Pause) => {
                        let _ = self.command_tx.send(AppCommand::GoalPause);
                    }
                    Ok(crate::agent::GoalCommand::Resume) => {
                        let _ = self.command_tx.send(AppCommand::GoalResume);
                    }
                    Ok(crate::agent::GoalCommand::Set(condition)) => {
                        self.goal_indicator = "◎ /goal active".into();
                        let _ = self.command_tx.send(AppCommand::GoalSet {
                            condition: condition.clone(),
                        });
                        self.push_system(format!("◎ /goal set — {condition}"));
                        // Claude Code: setting a goal starts a turn immediately.
                        self.submit_user_text(condition);
                    }
                    Err(e) => self.push_system(e),
                }
            }
            "/handoff" => {
                self.push_system(
                    "Requesting handoff — agent will write notes then end the turn.",
                );
                let _ = self.command_tx.send(AppCommand::HandoffRequest);
            }
            "/seat" => {
                let raw = arg.trim();
                let lower = raw.to_lowercase();
                let is_ops = lower.is_empty()
                    || lower == "status"
                    || lower == "board"
                    || lower == "detach"
                    || lower == "abort"
                    || lower.starts_with("follow ")
                    || lower.starts_with("attach ")
                    || lower.starts_with("open ")
                    || lower.starts_with("inspect ")
                    || lower.starts_with("steer ")
                    || lower.starts_with("logs ")
                    || lower.starts_with("log ");
                if is_ops {
                    let _ = self.handle_seat_ops(raw);
                } else {
                match crate::agent::parse_seat_arg(arg) {
                    Ok(crate::agent::SeatCommand::Status) => {
                        self.push_system(
                            "Usage: /seat <name> | clear | list | pronouns … | role … | caste … | orders … | model … | rename …\n\
                             Ops: /city | /seat follow|attach|detach|steer|abort|open <seat>",
                        );
                    }
                    Ok(crate::agent::SeatCommand::Clear) => {
                        let _ = self.command_tx.send(AppCommand::SetSeat { name: None });
                        self.push_system("Seat cleared");
                    }
                    Ok(crate::agent::SeatCommand::List) => {
                        let names = crate::agent::seat::list_names();
                        if names.is_empty() {
                            self.push_system("No seats yet. Create with /seat <name>");
                        } else {
                            self.push_system(format!("Seats: {}", names.join(", ")));
                        }
                    }
                    Ok(crate::agent::SeatCommand::Bind(name)) => {
                        match crate::agent::seat::load_or_create(&name) {
                            Ok(seat) => {
                                let _ = self.command_tx.send(AppCommand::SetSeat {
                                    name: Some(seat.name.clone()),
                                });
                                self.push_system(format!(
                                    "Seat `{}` bound{}",
                                    seat.name,
                                    if seat.role.is_empty() {
                                        " — set role with /seat role …".to_string()
                                    } else {
                                        format!(" ({})", seat.role)
                                    }
                                ));
                            }
                            Err(e) => self.push_system(e),
                        }
                    }
                    Ok(crate::agent::SeatCommand::SetPronouns(p)) => {
                        if let Some(name) = crate::tools::handoff::active_seat() {
                            match crate::agent::seat::load(&name) {
                                Ok(mut seat) => {
                                    seat.pronouns = p;
                                    if let Err(e) = crate::agent::seat::save(&seat) {
                                        self.push_system(e);
                                    } else {
                                        let _ = self.command_tx.send(AppCommand::SetSeat {
                                            name: Some(seat.name),
                                        });
                                        self.push_system("Pronouns updated");
                                    }
                                }
                                Err(e) => self.push_system(e),
                            }
                        } else {
                            self.push_system("Bind a seat first: /seat <name>");
                        }
                    }
                    Ok(crate::agent::SeatCommand::SetRole(role)) => {
                        if let Some(name) = crate::tools::handoff::active_seat() {
                            match crate::agent::seat::load(&name) {
                                Ok(mut seat) => {
                                    seat.role = role;
                                    if let Err(e) = crate::agent::seat::save(&seat) {
                                        self.push_system(e);
                                    } else {
                                        let _ = self.command_tx.send(AppCommand::SetSeat {
                                            name: Some(seat.name),
                                        });
                                        self.push_system("Role updated");
                                    }
                                }
                                Err(e) => self.push_system(e),
                            }
                        } else {
                            self.push_system("Bind a seat first: /seat <name>");
                        }
                    }
                    Ok(crate::agent::SeatCommand::SetCaste(caste)) => {
                        if let Some(name) = crate::tools::handoff::active_seat() {
                            match crate::agent::seat::load(&name) {
                                Ok(mut seat) => {
                                    seat.caste = caste;
                                    if let Err(e) = crate::agent::seat::save(&seat) {
                                        self.push_system(e);
                                    } else {
                                        let _ = self.command_tx.send(AppCommand::SetSeat {
                                            name: Some(seat.name.clone()),
                                        });
                                        self.push_system(format!(
                                            "Caste set to `{}` (effective: {})",
                                            caste.as_str(),
                                            seat.effective_caste().as_str()
                                        ));
                                    }
                                }
                                Err(e) => self.push_system(e),
                            }
                        } else {
                            self.push_system("Bind a seat first: /seat <name>");
                        }
                    }
                    Ok(crate::agent::SeatCommand::SetOrders(orders)) => {
                        if let Some(name) = crate::tools::handoff::active_seat() {
                            match crate::agent::seat::load(&name) {
                                Ok(mut seat) => {
                                    seat.standing_orders = orders;
                                    if let Err(e) = crate::agent::seat::save(&seat) {
                                        self.push_system(e);
                                    } else {
                                        let _ = self.command_tx.send(AppCommand::SetSeat {
                                            name: Some(seat.name),
                                        });
                                        self.push_system("Standing orders updated");
                                    }
                                }
                                Err(e) => self.push_system(e),
                            }
                        } else {
                            self.push_system("Bind a seat first: /seat <name>");
                        }
                    }
                    Ok(crate::agent::SeatCommand::SetModel(model)) => {
                        if let Some(name) = crate::tools::handoff::active_seat() {
                            match crate::agent::seat::load(&name) {
                                Ok(mut seat) => {
                                    seat.model = model.clone();
                                    if let Err(e) = crate::agent::seat::save(&seat) {
                                        self.push_system(e);
                                    } else {
                                        let _ = self.command_tx.send(AppCommand::SetSeat {
                                            name: Some(seat.name),
                                        });
                                        self.push_system(match model {
                                            Some(m) => format!("Seat model set to {m}"),
                                            None => "Seat model cleared".into(),
                                        });
                                    }
                                }
                                Err(e) => self.push_system(e),
                            }
                        } else {
                            self.push_system("Bind a seat first: /seat <name>");
                        }
                    }
                    Ok(crate::agent::SeatCommand::Rename(new_name)) => {
                        if let Some(old) = crate::tools::handoff::active_seat() {
                            match crate::agent::seat::rename(&old, &new_name) {
                                Ok(seat) => {
                                    let _ = self.command_tx.send(AppCommand::SetSeat {
                                        name: Some(seat.name.clone()),
                                    });
                                    self.push_system(format!(
                                        "Renamed seat → {} (history preserved)",
                                        seat.name
                                    ));
                                }
                                Err(e) => self.push_system(e),
                            }
                        } else {
                            self.push_system("Bind a seat first: /seat <name>");
                        }
                    }
                    Err(e) => self.push_system(e),
                }
                } // end else identity
            }
            "/beads" => {
                let sub = arg.trim().to_lowercase();
                if sub == "ready" {
                    match crate::beads::list_ready(None) {
                        Ok(items) => {
                            let mut out = String::new();
                            if let Some(c) = crate::beads::format_counts_line(None) {
                                out.push_str(&c);
                                out.push('\n');
                            }
                            if items.is_empty() {
                                out.push_str("No ready beads.");
                            } else {
                                out.push_str("Ready:\n");
                                out.push_str(&crate::beads::format_summary(&items));
                            }
                            self.push_system(out);
                        }
                        Err(e) => self.push_system(e),
                    }
                } else {
                    match crate::beads::list(None) {
                        Ok(items) => {
                            let mut out = String::new();
                            if let Some(c) = crate::beads::format_counts_line(None) {
                                out.push_str(&c);
                                out.push('\n');
                            }
                            out.push_str(&crate::beads::format_summary(&items));
                            self.push_system(out);
                        }
                        Err(e) => self.push_system(e),
                    }
                }
            }
            "/worker" => {
                let seat = arg.trim();
                if seat.is_empty() {
                    self.push_system(crate::fleet::format_worker_help(None));
                } else {
                    self.push_system(crate::fleet::format_worker_help(Some(seat)));
                }
            }
            "/marshal" => {
                let rest = arg.trim();
                if rest.is_empty() {
                    let mut out = crate::marshal::run_once();
                    if let Some(r) = crate::marshal::read_last_report() {
                        out.push_str(&format!("\n(report saved at {})", r.at));
                    }
                    self.push_system(out);
                } else if let Some(rest) = rest.strip_prefix("assign ") {
                    let mut parts = rest.split_whitespace();
                    let bead = parts.next().unwrap_or("");
                    let seat = parts.next().unwrap_or("");
                    if bead.is_empty() || seat.is_empty() {
                        self.push_system("Usage: /marshal assign <bead> <seat>");
                    } else {
                        match crate::marshal::assign_bead(bead, seat) {
                            Ok(b) => self.push_system(format!(
                                "Assigned {} → {} — {}",
                                b.id, seat, b.title
                            )),
                            Err(e) => self.push_system(e),
                        }
                    }
                } else {
                    self.push_system("Usage: /marshal | /marshal assign <bead> <seat>");
                }
            }
            "/mail" => {
                let rest = arg.trim();
                if rest.is_empty() || rest == "inbox" || rest == "read" {
                    let seat = crate::tools::handoff::active_seat();
                    self.push_system(crate::mail::format_inbox(seat.as_deref()));
                } else if let Some(rest) = rest.strip_prefix("ack ") {
                    match crate::mail::ack(rest.trim()) {
                        Ok(m) => self.push_system(format!("Acked {}", m.id)),
                        Err(e) => self.push_system(e),
                    }
                } else if let Some(rest) = rest.strip_prefix("send ") {
                    // /mail send <to> <body…>
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    let to = parts.next().unwrap_or("").trim();
                    let body = parts.next().unwrap_or("").trim();
                    if to.is_empty() || body.is_empty() {
                        self.push_system("Usage: /mail send <to> <body>");
                    } else {
                        let from = crate::tools::handoff::active_seat()
                            .unwrap_or_else(|| "human".into());
                        match crate::mail::send(&from, to, body, vec![]) {
                            Ok(m) => self.push_system(format!("Sent {} → {}", m.id, m.to)),
                            Err(e) => self.push_system(e),
                        }
                    }
                } else {
                    self.push_system(
                        "Usage: /mail | /mail send <to> <body> | /mail ack <id>",
                    );
                }
            }
            "/wish" => {
                let text = arg.trim();
                if text.is_empty() {
                    self.push_system("Usage: /wish <text>  (creates a design bead labeled wish)");
                } else {
                    match crate::wish::create_wish(text, false, true) {
                        Ok(b) => self.push_system(crate::wish::format_created(&b)),
                        Err(e) => self.push_system(e),
                    }
                }
            }
            "/moot" => {
                let rest = arg.trim();
                if rest.is_empty() || rest == "list" {
                    self.push_system(crate::moot::list());
                } else if let Some(topic) = rest.strip_prefix("open ") {
                    match crate::moot::open(topic.trim()) {
                        Ok(m) => self.push_system(format!("Opened {} — {}", m.id, m.topic)),
                        Err(e) => self.push_system(e),
                    }
                } else if let Some(rest) = rest.strip_prefix("append ") {
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    let id = parts.next().unwrap_or("");
                    let text = parts.next().unwrap_or("").trim();
                    let from = crate::tools::handoff::active_seat()
                        .unwrap_or_else(|| "human".into());
                    match crate::moot::append(id, &from, text) {
                        Ok(m) => self.push_system(format!(
                            "Appended to {} ({} entries)",
                            m.id,
                            m.entries.len()
                        )),
                        Err(e) => self.push_system(e),
                    }
                } else if let Some(rest) = rest.strip_prefix("close ") {
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    let id = parts.next().unwrap_or("");
                    let summary = parts.next().map(|s| s.trim());
                    match crate::moot::close(id, summary) {
                        Ok(m) => self.push_system(format!("Closed {}", m.id)),
                        Err(e) => self.push_system(e),
                    }
                } else if let Some(id) = rest.strip_prefix("show ") {
                    match crate::moot::show(id.trim()) {
                        Ok(s) => self.push_system(s),
                        Err(e) => self.push_system(e),
                    }
                } else {
                    self.push_system(
                        "Usage: /moot | open <topic> | append <id> <text> | close <id> [summary] | show <id>",
                    );
                }
            }
            "/laurels" => {
                let items = crate::agent::laurel::recent(12);
                if items.is_empty() {
                    self.push_system(
                        "No laurels yet. Sit with this quiet — recognition comes without chasing.\n\
                         Add with /laurel <praise text>.",
                    );
                } else {
                    let mut out = String::from(
                        "## Laurels (sit with these — no work attached)\n",
                    );
                    for l in &items {
                        let seat = l.seat.as_deref().unwrap_or("—");
                        out.push_str(&format!(
                            "- [{}] ({}) {}\n",
                            l.written_at, seat, l.text.trim()
                        ));
                    }
                    self.push_system(out);
                }
            }
            "/city" => {
                let raw = arg.trim().to_lowercase();
                if raw == "board" || raw == "status" || raw == "text" {
                    self.show_city_board();
                } else {
                    self.toggle_city_panel();
                }
            }
            "/fleet" => {
                let raw = arg.trim();
                let lower = raw.to_lowercase();
                if lower.is_empty() || lower == "panel" {
                    self.toggle_city_panel();
                } else if lower == "status" {
                    self.show_city_board();
                } else if lower == "down" {
                    self.push_system(crate::fleet::fleet_down(None));
                } else if let Some(rest) = lower.strip_prefix("down ") {
                    let seats = crate::fleet::parse_seat_list(rest);
                    self.push_system(crate::fleet::fleet_down(Some(seats)));
                } else if let Some(rest) = lower
                    .strip_prefix("up ")
                    .or_else(|| (lower == "up").then_some(""))
                {
                    let seats = if rest.trim().is_empty() {
                        vec!["Fleet-1".into(), "Fleet-2".into()]
                    } else {
                        let after = raw
                            .split_once(char::is_whitespace)
                            .map(|(_, r)| r.trim())
                            .unwrap_or("");
                        crate::fleet::parse_seat_list(if after.is_empty() {
                            "Fleet-1,Fleet-2"
                        } else {
                            after
                        })
                    };
                    let opts = crate::fleet::FleetUpOpts {
                        seats,
                        budget_minutes: 480,
                        sleep_secs: 5,
                        quiet: false,
                        provider: Some(self.provider_name.clone()),
                        model: Some(self.model_name.clone()),
                        approve: true,
                        fail_fast: false,
                    };
                    match crate::fleet::fleet_up(opts) {
                        Ok(msg) => self.push_system(msg),
                        Err(e) => self.push_system(e),
                    }
                } else if self.handle_seat_ops(raw) {
                    // follow/attach/detach/steer/abort/open/logs
                } else {
                    self.push_system(
                        "Usage: /fleet | /fleet up|down | /seat follow|attach|detach|steer|abort|open",
                    );
                }
            }
            "/detach" => {
                self.fleet_detach(false);
            }
            "/brain" => {
                let rest = arg.trim();
                if let Some(fact) = rest
                    .strip_prefix("remember ")
                    .or_else(|| rest.strip_prefix("remember"))
                {
                    let fact = fact.trim();
                    if fact.is_empty() {
                        self.push_system("Usage: /brain remember <short operational fact>");
                    } else if let Err(e) = crate::brain::remember(fact) {
                        self.push_system(e);
                    } else {
                        self.push_system(format!("Remembered: {fact}"));
                    }
                } else if let Some(q) = rest.strip_prefix("falsify ") {
                    match crate::brain::falsify(q.trim()) {
                        Ok(n) => self.push_system(format!("Falsified {n} fact(s)")),
                        Err(e) => self.push_system(e),
                    }
                } else if rest == "ledger" {
                    let items = crate::brain::recent_ledger(20);
                    if items.is_empty() {
                        self.push_system("Ledger empty.");
                    } else {
                        let mut out = String::from("Ledger:\n");
                        for e in items {
                            out.push_str(&format!(
                                "  [{}] {} {} — {}\n",
                                e.at,
                                e.bead,
                                e.kind.as_deref().unwrap_or("?"),
                                e.summary
                            ));
                        }
                        self.push_system(out);
                    }
                } else {
                    let facts = crate::brain::recent_facts(12);
                    if facts.is_empty() {
                        self.push_system(
                            "No brain facts yet. Usage: /brain remember <fact> | falsify <q> | ledger\n\
                             Doctrine: put markdown in ./brain/*.md",
                        );
                    } else {
                        let mut out = String::from("Brain facts:\n");
                        for f in facts {
                            out.push_str(&format!(
                                "- [{}] [{}] {}\n",
                                f.id.as_deref().unwrap_or("-"),
                                f.written_at,
                                f.text
                            ));
                        }
                        self.push_system(out);
                    }
                }
            }
            "/laurel" => {
                let text = arg.trim();
                if text.is_empty() {
                    self.push_system("Usage: /laurel <praise text>");
                } else {
                    let seat = crate::tools::handoff::active_seat();
                    let laurel =
                        crate::agent::laurel::Laurel::new(text.to_string(), seat.clone());
                    if let Err(e) = crate::agent::laurel::append(&laurel) {
                        self.push_system(e);
                    } else {
                        if let Some(ref name) = seat {
                            if let Ok(mut s) = crate::agent::seat::load(name) {
                                s.append_laurel(laurel);
                                let _ = crate::agent::seat::save(&s);
                            }
                        }
                        self.push_system(format!(
                            "Laurel recorded (recognition only): {text}"
                        ));
                    }
                }
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
                            let tools_note = if skill.tools.is_empty() {
                                String::new()
                            } else {
                                format!(" (tools: {})", skill.tools.join(", "))
                            };
                            let _ = self.command_tx.send(AppCommand::SetSkillTools {
                                tools: skill.tools.clone(),
                            });
                            self.push_system(format!(
                                "Loaded skill `{}`{}",
                                skill.name, tools_note
                            ));
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
                            let fork = match (&s.parent_id, &s.branch_label) {
                                (Some(p), Some(l)) => {
                                    format!(" ↩{} [{}]", SessionStore::short_id(p), l)
                                }
                                (Some(p), None) => {
                                    format!(" ↩{}", SessionStore::short_id(p))
                                }
                                _ => String::new(),
                            };
                            out.push_str(&format!(
                                "  {}  {} msgs  {}  — {}{}\n",
                                SessionStore::short_id(&s.id),
                                s.message_count,
                                s.model,
                                title,
                                fork
                            ));
                        }
                        out.push_str(
                            "Resume: rs-agent -r <id> · rs-agent -r latest · Fork: /fork [label]",
                        );
                        self.push_system(out);
                    }
                }
                Err(e) => self.push_system(format!("Failed to list sessions: {}", e)),
            },
            "/export" => {
                let fmt = if arg.is_empty() {
                    "md"
                } else {
                    match arg.trim().to_lowercase().as_str() {
                        "md" | "markdown" => "md",
                        "json" => "json",
                        "html" | "htm" => "html",
                        other => {
                            self.push_system(format!(
                                "Unknown export format `{other}`. Use /export [md|json|html]"
                            ));
                            return true;
                        }
                    }
                };
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                let dir = std::path::Path::new(&home).join(".rs-agent").join("exports");
                let _ = std::fs::create_dir_all(&dir);
                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                let path = dir.join(format!("export_{}.{}", ts, fmt));
                let content = match SessionStore::new().load(&self.session_id) {
                    Ok(data) => match fmt {
                        "json" => match session::export_json(&data) {
                            Ok(s) => s,
                            Err(e) => {
                                self.push_system(format!("JSON export failed: {e}"));
                                return true;
                            }
                        },
                        "html" => session::export_html(&data),
                        _ => session::export_markdown(&data),
                    },
                    Err(_) => {
                        // Session not on disk yet — fall back to in-memory chat.
                        let mut md = format!("# rs-agent export {}\n\n", self.session_id);
                        for m in &self.messages {
                            md.push_str(&format!("## {}\n\n{}\n\n", m.role, m.text));
                            if let Some(ref th) = m.thinking {
                                if !th.is_empty() {
                                    md.push_str(&format!(
                                        "<details><summary>thinking</summary>\n\n{}\n\n</details>\n\n",
                                        th
                                    ));
                                }
                            }
                        }
                        if fmt == "json" {
                            serde_json::json!({"id": self.session_id, "fallback": true, "markdown": md})
                                .to_string()
                        } else if fmt == "html" {
                            format!(
                                "<!DOCTYPE html><html><body><pre>{}</pre></body></html>",
                                md.replace('<', "&lt;")
                            )
                        } else {
                            md
                        }
                    }
                };
                match std::fs::write(&path, content) {
                    Ok(()) => self.push_system(format!("Exported ({fmt}) to {}", path.display())),
                    Err(e) => self.push_system(format!("Export failed: {}", e)),
                }
            }
            "/trust" => {
                let mut parts = arg.splitn(2, char::is_whitespace);
                let sub = parts.next().unwrap_or("").trim();
                match sub {
                    "list" | "" => {
                        let paths = self.trust_store.list();
                        let cwd = std::env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let path_rules = self.path_allows.list_for_project(&cwd);
                        let mut out = String::new();
                        if paths.is_empty() {
                            out.push_str("No trusted projects.\n");
                        } else {
                            out.push_str("Trusted projects:\n");
                            for (p, trusted) in paths {
                                out.push_str(&format!(
                                    "  {} {}\n",
                                    if trusted { "✓" } else { "·" },
                                    p
                                ));
                            }
                        }
                        if path_rules.is_empty() {
                            out.push_str("No path-scoped allows for this project.");
                        } else {
                            out.push_str("Path allows (this project):\n");
                            for r in path_rules {
                                out.push_str(&format!("  {} → {}\n", r.tool, r.path_prefix));
                            }
                        }
                        self.push_system(out);
                    }
                    "reset" | "clear" => {
                        let cwd = std::env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        self.trust_store.clear();
                        self.path_allows.clear_all();
                        let _ = cwd;
                        self.push_system("Trust store and path allows cleared.");
                    }
                    _ => self.push_system("Usage: /trust list|reset"),
                }
            }
            "/compact" => {
                let _ = self.command_tx.send(AppCommand::Compact);
                self.status = "compacting...".to_string();
            }
            "/revert" => {
                match crate::tools::turn_snapshot::restore_last_turn() {
                    Ok(n) => {
                        self.status = format!("restored {n} file(s) from turn");
                        self.push_system(format!(
                            "Reverted last turn snapshot ({n} file(s) restored)."
                        ));
                    }
                    Err(e) => {
                        self.status = "revert failed".into();
                        self.push_system(format!("Revert failed: {e}"));
                    }
                }
            }
            "/new" => {
                self.goal_indicator.clear();
                let _ = self.command_tx.send(AppCommand::NewSession);
                self.push_system("Starting new session…");
            }
            "/fork" => {
                let (at, label) = parse_fork_args(arg);
                let _ = self.command_tx.send(AppCommand::ForkSession { label, at });
                self.push_system(match at {
                    Some(n) => format!("Forking session at API message @{n}…"),
                    None => "Forking session…".into(),
                });
            }
            "/timeline" => {
                self.show_timeline_panel = !self.show_timeline_panel;
                if self.show_timeline_panel {
                    let _ = self.command_tx.send(AppCommand::RequestTimeline);
                    self.push_system(
                        "Timeline panel: j/k or ↑/↓ select · Enter fork at @N · Esc closes · /fork @N [label]",
                    );
                } else {
                    self.push_system("Timeline panel hidden.");
                }
            }
            "/image" => {
                if arg.is_empty() {
                    self.push_system(
                        "Usage: /image <path.png|jpg|…>\n\
                         Queues a Kitty graphics render after the next draw (Kitty/WezTerm/Ghostty, or RS_AGENT_KITTY=1).",
                    );
                } else if super::kitty::is_image_path(arg) && std::path::Path::new(arg).is_file() {
                    self.pending_kitty_images.push(arg.to_string());
                    self.push_system(format!("Queued image: {arg}"));
                } else {
                    self.push_system(format!("Not a readable image file: {arg}"));
                }
            }
            "/lsp" => {
                let sub = arg.split_whitespace().next().unwrap_or("status");
                match sub {
                    "start" => {
                        if let Some(ref tx) = self.lsp_cmd_tx {
                            let _ = tx.send(LspCmd::Start);
                            self.push_system(
                                "Starting LSP (rust-analyzer, or RS_AGENT_LSP). Status appears in the footer.",
                            );
                        }
                    }
                    "stop" => {
                        if let Some(ref tx) = self.lsp_cmd_tx {
                            let _ = tx.send(LspCmd::Stop);
                        }
                        self.lsp_summary.clear();
                        self.push_system("LSP stopped.");
                    }
                    "status" | "" => {
                        if self.lsp_summary.is_empty() {
                            self.push_system("LSP idle. Use `/lsp start` (requires rust-analyzer on PATH).");
                        } else {
                            self.push_system(format!("LSP:{}", self.lsp_summary.trim()));
                        }
                    }
                    _ => self.push_system("Usage: /lsp start|stop|status"),
                }
            }
            "/skill-pack" => {
                let mut parts = arg.split_whitespace();
                let sub = parts.next().unwrap_or("");
                match sub {
                    "export" => {
                        let names: Vec<String> = parts.map(|s| s.to_string()).collect();
                        let out = std::path::PathBuf::from(if names.is_empty() {
                            "skills-pack.zip".into()
                        } else {
                            format!("{}-skills.zip", names[0])
                        });
                        match crate::skills::export_pack(&names, &out) {
                            Ok(msg) => self.push_system(msg),
                            Err(e) => self.push_system(format!("skill-pack export failed: {e}")),
                        }
                    }
                    "import" => {
                        let path = parts.next().unwrap_or("");
                        if path.is_empty() {
                            self.push_system("Usage: /skill-pack import <pack.zip>");
                        } else {
                            match crate::skills::import_pack(std::path::Path::new(path)) {
                                Ok(msg) => {
                                    self.push_system(format!("{msg}\nRun /reload to pick up new skills."));
                                }
                                Err(e) => self.push_system(format!("skill-pack import failed: {e}")),
                            }
                        }
                    }
                    _ => self.push_system(
                        "Usage:\n  /skill-pack export [skill names…]\n  /skill-pack import <pack.zip>",
                    ),
                }
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
                self.show_fleet_panel = false;
                self.show_timeline_panel = false;
                if self.show_tree_panel && self.side_mode == SidePanelMode::Tree {
                    self.show_tree_panel = false;
                } else {
                    self.show_tree_panel = true;
                    self.side_mode = SidePanelMode::Tree;
                }
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
            "/output" => {
                self.show_repl_panel = !self.show_repl_panel;
                let n = self.tool_output_tabs.len();
                self.push_system(format!(
                    "Bottom console {} — {n} run(s). Tab / ↑↓ switch while waiting.",
                    if self.show_repl_panel { "shown" } else { "hidden" }
                ));
            }
            _ => {
                self.push_system(format!("Unknown command: {} (try /help)", cmd));
            }
        }
        true
    }

    fn handle_help_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(overlay) = self.help_overlay.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.help_overlay = None,
            KeyCode::Backspace => {
                overlay.query.pop();
                overlay.selection = 0;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                overlay.query.clear();
                overlay.selection = 0;
            }
            KeyCode::Up => {
                overlay.selection = overlay.selection.saturating_sub(1);
            }
            KeyCode::Down => {
                overlay.selection = overlay.selection.saturating_add(1);
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                overlay.query.push(c);
                overlay.selection = 0;
            }
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(st) = self.settings.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.settings = None;
            }
            KeyCode::Enter => {
                self.theme_name = st.theme;
                self.palette = Palette::for_theme(self.theme_name);
                self.toast_enabled = st.toast;
                self.toast_sound = st.toast_sound;
                self.notify_mode = NotifyMode::parse(&st.notify);
                self.mouse_enabled = st.mouse_enabled;
                self.settings = None;
                self.push_system("Settings applied");
            }
            KeyCode::Left | KeyCode::BackTab => st.tab = st.tab.prev(),
            KeyCode::Right | KeyCode::Tab => st.tab = st.tab.next(),
            KeyCode::Char('1') if st.tab == settings::SettingsTab::Theme => {
                st.theme = ThemeName::Dark;
            }
            KeyCode::Char('2') if st.tab == settings::SettingsTab::Theme => {
                st.theme = ThemeName::Light;
            }
            KeyCode::Char('3') if st.tab == settings::SettingsTab::Theme => {
                st.theme = ThemeName::Forest;
            }
            KeyCode::Char('m') if st.tab == settings::SettingsTab::Input => {
                st.mouse_enabled = !st.mouse_enabled;
            }
            KeyCode::Char('t') if st.tab == settings::SettingsTab::Alerts => {
                st.toast = !st.toast;
            }
            KeyCode::Char('s') if st.tab == settings::SettingsTab::Alerts => {
                st.toast_sound = !st.toast_sound;
            }
            KeyCode::Char('n') if st.tab == settings::SettingsTab::Alerts => {
                st.notify = match NotifyMode::parse(&st.notify) {
                    NotifyMode::Off => "terminal".into(),
                    NotifyMode::Terminal => "system".into(),
                    NotifyMode::System => "off".into(),
                };
            }
            _ => {}
        }
    }

    fn handle_palette_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => self.dismiss_overlays(),
            KeyCode::Up => {
                self.palette_selection = self.palette_selection.saturating_sub(1);
            }
            KeyCode::Down => {
                self.palette_selection = self.palette_selection.saturating_add(1);
                if !self.palette_items.is_empty() {
                    self.palette_selection = self
                        .palette_selection
                        .min(self.palette_items.len() - 1);
                }
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.refresh_palette_items();
            }
            KeyCode::Enter => {
                if let Some(cmd) = self.palette_items.get(self.palette_selection).cloned() {
                    self.dismiss_overlays();
                    let _ = self.handle_slash_command(&cmd);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.palette_query.push(c);
                self.refresh_palette_items();
            }
            _ => {}
        }
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Tab if !self.tool_output_tabs.is_empty() => {
                self.cycle_tool_output_tab(1);
                return;
            }
            KeyCode::BackTab if !self.tool_output_tabs.is_empty() => {
                self.cycle_tool_output_tab(-1);
                return;
            }
            _ => {}
        }
        if self.show_fleet_panel {
            match key.code {
                KeyCode::Up => {
                    self.fleet_panel.move_sel(-1);
                    return;
                }
                KeyCode::Down => {
                    self.fleet_panel.move_sel(1);
                    return;
                }
                KeyCode::Enter => {
                    self.activate_city_selection();
                    return;
                }
                KeyCode::Char('x') => {
                    self.fleet_panel.expanded = !self.fleet_panel.expanded;
                    return;
                }
                _ => {}
            }
        }
        if self.key_matches("insert", key) {
            self.input_mode = InputMode::Insert;
            self.unseen_done = false;
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
            if self.show_tree_panel && !self.show_fleet_panel {
                self.side_mode = self.side_mode.toggle();
                self.status = format!("side · {}", self.side_mode.label());
            } else {
                self.show_fleet_panel = false;
                self.show_timeline_panel = false;
                self.show_tree_panel = true;
                self.side_mode = SidePanelMode::Tree;
            }
            return;
        }
        if self.show_timeline_panel {
            match key.code {
                KeyCode::Esc => {
                    self.show_timeline_panel = false;
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.timeline_selection = self.timeline_selection.saturating_sub(1);
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let max = self.timeline_entries.len().saturating_sub(1);
                    self.timeline_selection = self.timeline_selection.saturating_add(1).min(max);
                    return;
                }
                KeyCode::Enter => {
                    if let Some((idx, _)) = self.timeline_entries.get(self.timeline_selection) {
                        let at = *idx;
                        let _ = self.command_tx.send(AppCommand::ForkSession {
                            label: None,
                            at: Some(at),
                        });
                        self.push_system(format!("Forking at @{at}…"));
                    }
                    return;
                }
                _ => {}
            }
        }
        if self.key_matches("jump_bottom", key) {
            self.follow_bottom = true;
            self.unseen_done = false;
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
        if self.fleet_attach.as_ref().map(|a| a.phase) == Some(FleetAttachPhase::Inspect) {
            self.push_system(
                "Inspect is read-only. Use `/seat attach <seat>` to take over, or `/seat follow` + `/seat steer …`.",
            );
            self.input.clear();
            return;
        }
        if self.input_history.last().map(|s| s.as_str()) != Some(text.as_str()) {
            self.input_history.push(text.clone());
        }
        self.history_index = None;

        let expanded = Self::expand_dir_tokens(&text);

        self.input_mode = InputMode::Waiting;
        self.queued_steers = 0;
        if self.tool_in_progress.is_none() {
            self.tool_output_tabs.clear();
            self.tool_output_tab = 0;
            self.repl_panel.clear();
        }

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
        self.unseen_done = false;
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
        // AUTO: file tools + read-only builtins. Still prompt for bash/repl and
        // mutating MCP tools (those keep requires_permission=true).
        matches!(
            tool_name,
            "read"
                | "grep"
                | "ls"
                | "find"
                | "webfetch"
                | "websearch"
                | "write"
                | "edit"
        )
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
        // Prefer OpenCode Zen keys from the local OpenCode install when unset.
        if mref.provider.eq_ignore_ascii_case("opencode")
            || mref.provider.eq_ignore_ascii_case("opencode-go")
        {
            crate::ai::registry::export_opencode_auth_from_file();
        }
        if !crate::ai::registry::has_configured_auth(&mref.provider) {
            return Err(format!(
                "Provider `{}` has no credentials (export {} or sign in via OpenCode).",
                mref.provider,
                crate::ai::registry::api_key_env_for(&mref.provider)
            ));
        }

        let same_provider = mref.provider.eq_ignore_ascii_case(&self.provider_name);
        let needs_recreate = crate::ai::registry::provider_client_needs_recreate(
            &self.provider_name,
            &self.model_name,
            &mref.provider,
            &mref.model,
        );
        if same_provider && !needs_recreate {
            self.model_name = mref.model.clone();
            let _ = self.command_tx.send(AppCommand::SetModel {
                model: mref.model.clone(),
            });
            self.token_limit = crate::ai::token_count::get_context_limit(&mref.model);
            self.note_cycle_entry(&mref.display());
            Self::remember_selection(&mref.provider, &mref.model);
            let mut msg = if resolved_arg != raw {
                format!(
                    "Model set to {}/{} (alias `{}`)",
                    mref.provider, mref.model, raw
                )
            } else {
                format!("Model set to {}/{}", mref.provider, mref.model)
            };
            if let Some(w) = crate::agent::weak_model_user_warning(&mref.model) {
                msg.push('\n');
                msg.push_str(&w);
            }
            return Ok(msg);
        }

        if mref.provider.eq_ignore_ascii_case("opencode-cli")
            && std::env::var("OPENCODE_API_KEY").is_err()
        {
            std::env::set_var("OPENCODE_API_KEY", "cli-mode-no-key-needed");
        }
        if (mref.provider.eq_ignore_ascii_case("bedrock")
            || mref.provider.eq_ignore_ascii_case("amazon-bedrock"))
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
        let kind = if same_provider {
            "model + API endpoint"
        } else {
            "provider + model"
        };
        Ok(format!(
            "Switched to {}/{} ({})",
            mref.provider, mref.model, kind
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

    /// If the input is `/skill`, `/prompt`, `/model`, `/theme`, `/mode`, or
    /// `/provider` with a partial arg, either completes the single match or
    /// opens a picker. No-op otherwise.
    fn try_start_completion_picker(&mut self) {
        let text = self.input.clone();

        // Fixed-choice slash commands
        let fixed: &[(&str, &[&str], PickerMode)] = &[
            ("/theme ", &["dark", "light", "forest"], PickerMode::Prompt),
            ("/mode ", &["plan", "ask", "agent"], PickerMode::Prompt),
        ];
        for (cmd, options, mode) in fixed {
            let Some(rest) = text.strip_prefix(cmd) else {
                continue;
            };
            if rest.contains(char::is_whitespace) {
                continue;
            }
            let query_lower = rest.to_lowercase();
            let names: Vec<String> = options.iter().map(|s| (*s).to_string()).collect();
            let filtered: Vec<String> = if query_lower.is_empty() {
                names
            } else {
                Self::rank_and_filter(&names, &query_lower, 20)
            };
            match filtered.len() {
                0 => {}
                1 => self.input = format!("{}{} ", cmd, filtered[0]),
                _ => {
                    self.picker_prefix = cmd.to_string();
                    self.picker_query = rest.to_string();
                    self.picker_mode = *mode;
                    self.picker_results = filtered;
                    self.picker_selection = 0;
                    self.picker_active = true;
                }
            }
            return;
        }

        // /model — use available model displays
        if let Some(rest) = text.strip_prefix("/model ") {
            if !rest.contains(char::is_whitespace) {
                let mut names = crate::ai::registry::available_model_displays();
                let cfg = crate::config::Config::load();
                for alias in cfg.model_aliases.keys() {
                    if !names.iter().any(|n| n == alias) {
                        names.push(alias.clone());
                    }
                }
                let query_lower = rest.to_lowercase();
                let filtered: Vec<String> = if query_lower.is_empty() {
                    names
                } else {
                    Self::rank_and_filter(&names, &query_lower, 50)
                };
                match filtered.len() {
                    0 => {}
                    1 => self.input = format!("/model {} ", filtered[0]),
                    _ => {
                        self.picker_prefix = "/model ".into();
                        self.picker_query = rest.to_string();
                        self.picker_models = filtered.clone();
                        self.picker_mode = PickerMode::Model;
                        self.picker_results = filtered;
                        self.picker_selection = 0;
                        self.picker_active = true;
                    }
                }
                return;
            }
        }

        // /provider|/login
        for cmd in ["/provider ", "/login "] {
            let Some(rest) = text.strip_prefix(cmd) else {
                continue;
            };
            if rest.contains(char::is_whitespace) {
                continue;
            }
            let names: Vec<String> = crate::ai::registry::provider_picker_rows()
                .into_iter()
                .filter_map(|row| row.split_whitespace().next().map(|s| s.to_string()))
                .collect();
            let query_lower = rest.to_lowercase();
            let filtered: Vec<String> = if query_lower.is_empty() {
                names
            } else {
                Self::rank_and_filter(&names, &query_lower, 40)
            };
            match filtered.len() {
                0 => {}
                1 => self.input = format!("{cmd}{} ", filtered[0]),
                _ => {
                    self.picker_prefix = cmd.to_string();
                    self.picker_query = rest.to_string();
                    self.picker_mode = PickerMode::Provider;
                    self.picker_results = filtered;
                    self.picker_selection = 0;
                    self.picker_active = true;
                }
            }
            return;
        }

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
        self.hit_map.clear();
        lifecycle::set_focused(true);

        let show_side =
            self.show_fleet_panel || self.show_timeline_panel || self.show_tree_panel;
        let show_repl = self.show_repl_panel || !self.tool_output_tabs.is_empty();
        let view = compute_view(
            area,
            LayoutOpts {
                show_repl,
                show_side,
                side_pct: if self.show_fleet_panel {
                    34
                } else if self.show_timeline_panel {
                    32
                } else {
                    28
                },
                repl_height: if self.tool_output_tabs.len() > 1 { 9 } else { 7 },
            },
        );

        self.render_header(frame, view.header);
        if let Some(ref banner) = self.diagnostic_banner.clone() {
            // Non-modal top strip under header
            let y = view.header.y.saturating_add(view.header.height);
            if y < area.bottom() {
                let rect = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        format!(" ! {banner}"),
                        Style::default().fg(self.palette.warn).bg(self.palette.surface0),
                    )),
                    rect,
                );
            }
        }
        // Turn bar when Deep Context active
        if self.tree_breadcrumb != "idle" && !self.show_fleet_panel {
            let bar = tree_view::turn_bar_line(&self.tree_breadcrumb, &self.palette);
            // Drawn into chat top via messages; keep side tree rich.
            let _ = bar;
        }
        self.render_messages(frame, view.chat);
        if let Some(side) = view.side {
            if self.show_fleet_panel {
                fleet_panel::render_city_panel(frame, side, &self.fleet_panel, &self.palette);
                // Map selectable rows to click targets (rough 1-line-per-row after chrome).
                let mut y = side.y.saturating_add(3);
                for (i, row) in self.fleet_panel.rows.iter().enumerate() {
                    match row {
                        CityRow::Header { .. } => {
                            y = y.saturating_add(2);
                        }
                        CityRow::Hint { .. } => {
                            y = y.saturating_add(1);
                        }
                        CityRow::Worker { .. }
                        | CityRow::Wish { .. }
                        | CityRow::Ready { .. } => {
                            self.hit_map.push_line(
                                side.x,
                                y,
                                side.width,
                                HitTarget::FleetRow { index: i },
                            );
                            y = y.saturating_add(1);
                            if self.fleet_panel.expanded && i == self.fleet_panel.selection {
                                y = y.saturating_add(1);
                            }
                        }
                    }
                }
            } else if self.show_timeline_panel || self.side_mode == SidePanelMode::Timeline {
                self.render_timeline_panel(frame, side);
            } else {
                self.render_tree_panel(frame, side);
            }
        }
        if let Some(repl) = view.repl {
            self.render_repl_panel(frame, repl);
        }
        self.render_input(frame, view.input);
        self.render_footer(frame, view.footer);

        if let Some(ref t) = self.toast {
            let ta = toast::toast_area(area);
            toast::render_toast(frame, ta, t, &self.palette);
            self.hit_map.push(ta, HitTarget::Toast);
        }

        let overlay = self.picker_active
            || self.pending_permission.is_some()
            || self.pending_question.is_some()
            || self.help_overlay.is_some()
            || self.settings.is_some()
            || self.palette_open;
        if overlay {
            widgets::dim_background(frame, area);
            self.hit_map.push(area, HitTarget::ModalDismiss);
        }
        if self.picker_active {
            self.render_picker(frame, area);
        }
        if self.pending_permission.is_some() {
            self.render_permission_prompt(frame, area);
        }
        if self.pending_question.is_some() {
            self.render_question_prompt(frame, area);
        }
        if let Some(ref ho) = self.help_overlay {
            let entries = help::build_entries(&self.keys);
            help::render_help(frame, area, ho, &entries, &self.palette);
        }
        if let Some(ref st) = self.settings {
            settings::render_settings(frame, area, st, &self.palette);
        }
        if self.palette_open {
            let height = (self.palette_items.len() as u16).clamp(3, 12).saturating_add(2);
            let prect = widgets::centered_rect(area, 56, height);
            help::render_palette_list(
                frame,
                prect,
                "commands · Ctrl+K",
                &self.palette_items,
                self.palette_selection,
                &self.palette,
            );
        }
    }

    fn session_ui_state(&self) -> SessionUiState {
        SessionUiState::from_app(
            self.pending_permission.is_some(),
            self.pending_question.is_some(),
            self.input_mode == InputMode::Waiting,
            self.tool_in_progress.is_some(),
            &self.status,
            self.unseen_done,
        )
    }

    fn render_header(&mut self, frame: &mut Frame, area: Rect) {
        let state = self.session_ui_state();
        let yolo = if self.approved {
            "YOLO"
        } else if self.auto_mode {
            "AUTO"
        } else if self.pending_permission.is_some() {
            "ASK"
        } else {
            ""
        };
        let deep = self.tree_breadcrumb != "idle";
        let session_short = SessionStore::short_id(&self.session_id).to_string();
        let attach = self
            .fleet_attach
            .as_ref()
            .map(|a| match a.phase {
                FleetAttachPhase::Follow => format!("FOLLOW {}", a.seat),
                FleetAttachPhase::Attaching => format!("ATTACHING {}", a.seat),
                FleetAttachPhase::Attached => format!("ATTACHED {}", a.seat),
                FleetAttachPhase::Inspect => format!("INSPECT {}", a.seat),
            })
            .unwrap_or_default();
        let token_str = if self.token_limit > 0 {
            let pct = self.token_used as f64 / self.token_limit as f64 * 100.0;
            if self.near_limit {
                format!(" ⚠{:.0}%", pct)
            } else {
                format!(
                    " {:.1}K/{}K",
                    self.token_used as f64 / 1000.0,
                    self.token_limit / 1000
                )
            }
        } else {
            String::new()
        };
        let cost_str = crate::ai::token_count::format_cost_usd(
            &self.model_name,
            self.session_input_tokens,
            self.session_output_tokens,
        );
        let line = status::render_header_line(
            state,
            &self.provider_name,
            &self.model_name,
            self.agent_mode.as_str(),
            self.rlm_depth,
            deep,
            yolo,
            &session_short,
            self.session_title.as_deref(),
            &self.goal_indicator,
            &attach,
            &token_str,
            &cost_str,
            area.width as usize,
            &self.palette,
        );
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(self.palette.surface_dim)),
            area,
        );
    }

    fn render_footer(&mut self, frame: &mut Frame, area: Rect) {
        let mut hints = format!(
            " ^C quit · {} insert · /help · /keys",
            self.keys.binding("insert")
        );
        if self.input_mode == InputMode::Waiting {
            hints = " Esc abort · Tab console · Enter steer".to_string();
            if self.queued_steers > 0 {
                hints.push_str(&format!(" · queued:{}", self.queued_steers));
            }
            if self.tool_output_tabs.len() > 1 {
                hints.push_str(&format!(
                    " · out:{}/{}",
                    self.tool_output_tab + 1,
                    self.tool_output_tabs.len()
                ));
            }
        } else if self.input_mode == InputMode::Question {
            hints = " Enter answer · Esc cancel".to_string();
        } else if self.pending_permission.is_some() {
            hints = format!(
                " [{}] once · [{}] path · [{}] always · [{}] deny",
                self.keys.binding("perm_once"),
                self.keys.binding("perm_path"),
                self.keys.binding("perm_always"),
                self.keys.binding("perm_deny"),
            );
        } else if self.input_mode == InputMode::Normal {
            hints = self.keys.hint_line();
        }

        // Refresh beads at most every 2s (was every frame).
        if self.beads_ready_cache.0.elapsed() >= Duration::from_secs(2) {
            let n = crate::beads::list_ready(None)
                .ok()
                .map(|r| r.len())
                .unwrap_or(0);
            self.beads_ready_cache = (Instant::now(), n);
        }
        if self.beads_ready_cache.1 > 0 {
            hints.push_str(&format!(" · beads:{} ready", self.beads_ready_cache.1));
        }
        if !self.lsp_summary.is_empty() {
            hints.push(' ');
            hints.push_str(&self.lsp_summary);
        }

        let status_color = if self.near_limit {
            self.palette.danger
        } else if self.status == "ready" {
            self.palette.ok
        } else {
            self.palette.warn
        };
        let spinner = self
            .tool_in_progress
            .as_ref()
            .map(|(name, started)| {
                const FRAMES: [char; 4] = ['◐', '◓', '◑', '◒'];
                let elapsed = started.elapsed();
                let frame = FRAMES[(elapsed.as_millis() / 120) as usize % FRAMES.len()];
                format!(" {frame} {name} ({:.1}s)", elapsed.as_secs_f64())
            })
            .unwrap_or_default();

        let line = status::render_footer_line(
            &hints,
            &self.status,
            &spinner,
            status_color,
            &self.palette,
        );
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_tree_panel(&mut self, frame: &mut Frame, area: Rect) {
        let max_w = (area.width as usize).saturating_sub(4).max(8);
        let mut lines: Vec<Line> = Vec::new();
        lines.push(tree_view::turn_bar_line(&self.tree_breadcrumb, &self.palette));
        lines.push(Line::from(Span::styled(
            format!(
                " [{}|{}] ",
                SidePanelMode::Tree.label(),
                SidePanelMode::Timeline.label()
            ),
            Style::default().fg(self.palette.overlay0),
        )));
        lines.push(Line::from(Span::styled(
            "─".repeat(max_w.min(40)),
            Style::default().fg(self.palette.border),
        )));
        if self.tree_nodes.is_empty() {
            self.tree_nodes = tree_view::parse_text_tree(&self.tree_panel_text);
        }
        lines.extend(tree_view::render_nodes(
            &self.tree_nodes,
            &self.palette,
            max_w,
            None,
        ));
        let panel = Paragraph::new(lines).block(widgets::panel_block(
            "Call Tree",
            &self.palette,
            true,
        ));
        frame.render_widget(panel, area);
    }

    fn render_timeline_panel(&mut self, frame: &mut Frame, area: Rect) {
        let style = Style::default().fg(self.palette.muted);
        let sel = Style::default()
            .fg(self.palette.highlight_fg)
            .bg(self.palette.highlight_bg)
            .add_modifier(Modifier::BOLD);
        let items: Vec<ListItem> = if self.timeline_entries.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "(empty — send a message)",
                style,
            )))]
        } else {
            self.timeline_entries
                .iter()
                .enumerate()
                .map(|(i, (idx, summary))| {
                    let marker = if i == self.timeline_selection { "›" } else { " " };
                    let line = format!("{marker} @{idx} {summary}");
                    let st = if i == self.timeline_selection { sel } else { style };
                    ListItem::new(Line::from(Span::styled(line, st)))
                })
                .collect()
        };
        let list = List::new(items).block(widgets::panel_block(
            "Timeline · Enter fork",
            &self.palette,
            true,
        ));
        frame.render_widget(list, area);
    }

    /// Cap a panel buffer to roughly `cap` bytes without slicing mid-UTF-8
    /// character (e.g. emoji like ❌). Prefer cutting at a newline.
    fn trim_panel_utf8(panel: &mut String, cap: usize) {
        if panel.len() <= cap {
            return;
        }
        let mut start = panel.len() - cap;
        while start < panel.len() && !panel.is_char_boundary(start) {
            start += 1;
        }
        let cut = panel[start..]
            .find('\n')
            .map(|i| start + i + 1)
            .unwrap_or(start);
        panel.drain(..cut);
    }

    fn render_repl_panel(&mut self, frame: &mut Frame, area: Rect) {
        use ratatui::widgets::Clear;

        // Erase previous frame so long lines / side-panel thrash cannot bleed.
        frame.render_widget(Clear, area);

        let inner_w = (area.width as usize).saturating_sub(2).max(8);
        let visible_rows = (area.height as usize).saturating_sub(2).max(1);

        let title = if self.tool_output_tabs.is_empty() {
            "Console".to_string()
        } else {
            let mut parts = Vec::new();
            for (i, tab) in self.tool_output_tabs.iter().enumerate().take(6) {
                let on = i == self.tool_output_tab;
                let state = if tab.done { "✓" } else { "●" };
                if on {
                    parts.push(format!("[{state}{}]", tab.label));
                } else {
                    parts.push(format!(" {state}{} ", tab.label));
                }
            }
            if self.tool_output_tabs.len() > 6 {
                parts.push("…".into());
            }
            format!("Console · Tab {}", parts.join(""))
        };
        // Cap title so the border never overflows the terminal width.
        let title = Self::clip_console_line(&title, inner_w.saturating_sub(2));

        let body = self
            .tool_output_tabs
            .get(self.tool_output_tab)
            .map(|t| t.buffer.as_str())
            .filter(|b| !b.is_empty())
            .unwrap_or(self.repl_panel.as_str());

        let all_lines: Vec<&str> = body.lines().collect();
        let start = all_lines.len().saturating_sub(visible_rows);
        let style = Style::default()
            .fg(self.palette.overlay1)
            .bg(self.palette.panel_bg);
        let lines: Vec<Line> = all_lines[start..]
            .iter()
            .map(|l| {
                Line::from(Span::styled(
                    Self::clip_console_line(l, inner_w),
                    style,
                ))
            })
            .collect();

        // Tab hit targets on the title row (approx).
        for i in 0..self.tool_output_tabs.len().min(6) {
            self.hit_map.push_line(
                area.x.saturating_add(2 + (i as u16).saturating_mul(10)),
                area.y,
                10,
                HitTarget::OutputTab { index: i },
            );
        }

        let panel = Paragraph::new(lines)
            .style(Style::default().bg(self.palette.panel_bg))
            .block(
                widgets::panel_block(&title, &self.palette, true)
                    .style(Style::default().bg(self.palette.panel_bg).fg(self.palette.text)),
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
        let max_width = (area.width as usize).saturating_sub(3).max(20);
        let md = MarkdownStyle::from_palette(&self.palette);
        let syn = self.theme_name.syntect_theme();
        let mut lines: Vec<Line> = Vec::new();
        self.thinking_targets.clear();
        self.tool_targets.clear();
        for (msg_idx, msg) in self.messages.iter().enumerate() {
            // Conductor EventRow-style role rail
            let (rail, label, color) = match msg.role.as_str() {
                "system" => ("│", "system", self.palette.system),
                "user" => ("┃", "you", self.palette.user),
                "assistant" => ("┃", "agent", self.palette.assistant),
                "tool" => ("│", "tool", self.palette.tool),
                _ => ("│", "", self.palette.text),
            };
            let rail_style = Style::default().fg(color).add_modifier(Modifier::BOLD);

            if msg.role == "user" || msg.role == "system" {
                lines.push(Line::from(vec![
                    Span::styled(format!("{rail} "), rail_style),
                    Span::styled(
                        format!("{label} "),
                        Style::default()
                            .fg(color)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }

            // Render order: thinking → tools → response text
            if let Some(ref thinking) = msg.thinking {
                if !thinking.is_empty() {
                    if msg.show_thinking {
                        let think_style = Style::default()
                            .fg(self.palette.overlay0)
                            .add_modifier(Modifier::ITALIC | Modifier::UNDERLINED);
                        let line_idx = lines.len();
                        self.thinking_targets.push((line_idx, msg_idx));
                        let header = Line::from(vec![
                            Span::styled("│ ".to_string(), Style::default().fg(self.palette.overlay0)),
                            Span::styled("thinking · click to hide".to_string(), think_style),
                        ]);
                        lines.extend(Self::wrap_line(&header, max_width));
                        let body_style = Style::default()
                            .fg(self.palette.overlay1)
                            .add_modifier(Modifier::ITALIC);
                        for raw_line in render_markdown(thinking, syn, md) {
                            let mut styled_spans = vec![Span::styled(
                                "│ ".to_string(),
                                Style::default().fg(self.palette.overlay0),
                            )];
                            for span in raw_line.spans {
                                styled_spans.push(Span::styled(span.content, body_style));
                            }
                            lines.extend(Self::wrap_line(&Line::from(styled_spans), max_width));
                        }
                    } else {
                        let clickable = Style::default()
                            .fg(self.palette.overlay0)
                            .add_modifier(Modifier::UNDERLINED);
                        let line_idx = lines.len();
                        self.thinking_targets.push((line_idx, msg_idx));
                        let preview_budget = max_width.saturating_sub(14).max(8);
                        let preview: String = thinking.chars().take(preview_budget).collect();
                        let header = Line::from(vec![
                            Span::styled("│ ".to_string(), Style::default().fg(self.palette.overlay0)),
                            Span::styled(format!("💭 {preview}"), clickable),
                        ]);
                        lines.extend(Self::wrap_line(&header, max_width));
                    }
                }
            }

            for (tool_idx, block) in msg.tool_blocks.iter().enumerate() {
                let (icon, color) = if block.is_error {
                    ("×", self.palette.state_blocked)
                } else {
                    ("✓", self.palette.tool)
                };
                if block.expanded {
                    let header_style = Style::default()
                        .fg(color)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                    let line_idx = lines.len();
                    self.tool_targets.push((line_idx, msg_idx, tool_idx));
                    let header = Line::from(vec![
                        Span::styled(format!("{icon} "), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                        Span::styled(
                            format!("{} · collapse", block.name),
                            header_style,
                        ),
                    ]);
                    lines.extend(Self::wrap_line(&header, max_width));
                    for raw_line in render_markdown(&block.full, syn, md) {
                        // Keep syntax highlight (don't wipe to muted).
                        let mut styled = vec![Span::styled(
                            "  ",
                            Style::default(),
                        )];
                        if block.is_error {
                            for span in raw_line.spans {
                                styled.push(Span::styled(
                                    span.content,
                                    Style::default().fg(self.palette.danger),
                                ));
                            }
                        } else {
                            styled.extend(raw_line.spans);
                        }
                        lines.extend(Self::wrap_line(&Line::from(styled), max_width));
                    }
                } else {
                    let clickable = Style::default().fg(color);
                    let line_idx = lines.len();
                    self.tool_targets.push((line_idx, msg_idx, tool_idx));
                    let suffix = " …";
                    let fixed = icon.chars().count()
                        + 1
                        + block.name.chars().count()
                        + 3
                        + suffix.chars().count();
                    let preview_budget = max_width.saturating_sub(fixed).max(8);
                    let preview: String = block.preview.chars().take(preview_budget).collect();
                    let header = Line::from(vec![
                        Span::styled(
                            format!("{icon} "),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{}  ", block.name),
                            Style::default()
                                .fg(color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{preview}{suffix}"),
                            clickable.add_modifier(Modifier::DIM),
                        ),
                    ]);
                    lines.extend(Self::wrap_line(&header, max_width));
                }
            }

            if !msg.text.is_empty() {
                let rendered = render_markdown(&msg.text, syn, md);
                for (i, raw_line) in rendered.into_iter().enumerate() {
                    let mut line = raw_line;
                    if i == 0 && msg.role == "assistant" {
                        line.spans.insert(
                            0,
                            Span::styled(format!("{rail} "), rail_style),
                        );
                    } else if msg.role == "user" || msg.role == "system" {
                        line.spans.insert(
                            0,
                            Span::styled(format!("{rail} "), rail_style),
                        );
                    }
                    lines.extend(Self::wrap_line(&line, max_width));
                }
            } else if msg.role == "assistant"
                && msg.thinking.as_ref().map(|t| t.is_empty()).unwrap_or(true)
                && msg.tool_blocks.is_empty()
                && self.input_mode == InputMode::Waiting
            {
                lines.push(Line::from(vec![
                    Span::styled(format!("{rail} "), rail_style),
                    Span::styled(
                        "…".to_string(),
                        Style::default().fg(self.palette.overlay0),
                    ),
                ]));
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

        let chat = Paragraph::new(visible_lines).block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default().fg(self.palette.text)),
        );

        frame.render_widget(chat, area);
    }

    fn render_input(&mut self, frame: &mut Frame, area: Rect) {
        let (mode_label, border_color) = match self.input_mode {
            InputMode::Normal => ("normal", self.palette.border),
            InputMode::Insert => ("▸", self.palette.accent),
            InputMode::Waiting => ("working", self.palette.state_working),
            InputMode::ApiKey => ("api key", self.palette.warn),
            InputMode::Question => ("answer", self.palette.state_blocked),
        };

        let display_text = if self.input_mode == InputMode::ApiKey {
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

        let placeholder = if display_text.is_empty() && self.input_mode == InputMode::Insert {
            Span::styled(
                " message, /command, @file, #dir…",
                Style::default().fg(self.palette.overlay0),
            )
        } else {
            Span::styled(display_text.clone(), Style::default().fg(self.palette.text))
        };

        let input = Paragraph::new(Line::from(placeholder)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {mode_label} "))
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(self.palette.surface_dim)),
        );

        frame.render_widget(input, area);

        if matches!(
            self.input_mode,
            InputMode::Insert | InputMode::Waiting | InputMode::ApiKey | InputMode::Question
        ) {
            let cursor_len = if self.picker_active {
                match self.picker_mode {
                    PickerMode::File | PickerMode::Dir => {
                        self.picker_prefix.chars().count() + 1 + self.picker_query.chars().count()
                    }
                    PickerMode::Skill
                    | PickerMode::Prompt
                    | PickerMode::Model
                    | PickerMode::Provider => {
                        self.picker_prefix.chars().count() + self.picker_query.chars().count()
                    }
                }
            } else {
                self.input.chars().count()
            };
            let x = (cursor_len as u16 + 1).min(area.width.max(1).saturating_sub(2));
            frame.set_cursor_position(ratatui::layout::Position::new(area.x + x, area.y + 1));
        }
    }

    fn handle_permission_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(pending) = self.pending_permission.take() {
            if key.code == KeyCode::Enter || self.key_matches("perm_once", key) {
                let tool = pending.request.tool_name.clone();
                let _ = pending.reply_tx.send(PermissionReply::AllowOnce);
                self.status = format!("allowed {} (once)", tool);
            } else if self.key_matches("perm_path", key) {
                let tool = pending.request.tool_name.clone();
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if let Some(file) = extract_tool_path(&pending.request.tool_input) {
                    let prefix = path_allow_prefix(&file);
                    self.path_allows.add_rule(&cwd, &tool, &prefix);
                    let _ = pending.reply_tx.send(PermissionReply::AllowOnce);
                    self.status = format!("allowed {} under {} (path)", tool, prefix);
                } else {
                    self.status =
                        "no file path in tool args — use once/always, or pick a write/edit call"
                            .into();
                    self.pending_permission = Some(pending);
                }
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
        let width = if self.picker_mode == PickerMode::Provider {
            96
        } else {
            72
        }
        .min(area.width.saturating_sub(2));
        let height = picker_height + 2;
        // Anchor near input (bottom) like a command palette
        let picker_area = Rect {
            x: area.x + 1,
            y: area
                .y
                .saturating_add(area.height.saturating_sub(height + 4)),
            width,
            height,
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
                        Style::default().fg(self.palette.subtext)
                    };
                    ListItem::new(path.as_str()).style(style)
                })
                .collect()
        };

        let title = match self.picker_mode {
            PickerMode::File => "Files · Enter · Esc",
            PickerMode::Dir => "Directories · Enter · Esc",
            PickerMode::Skill => "Skills",
            PickerMode::Prompt => "Templates",
            PickerMode::Model => "Models",
            PickerMode::Provider => "Providers · Enter connect · Esc",
        };
        use ratatui::widgets::Clear;
        frame.render_widget(Clear, picker_area);
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} "))
                .border_style(
                    Style::default()
                        .fg(self.palette.accent)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(self.palette.panel_bg)),
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
            (self.palette.state_blocked, "blocked · dangerous")
        } else {
            (self.palette.state_working, "blocked · permission")
        };

        let mut text = vec![Line::from(Span::styled(
            format!(" {} needs approval", tool_name),
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        ))];

        if let Some(reason) = danger_reason {
            text.push(Line::from(Span::styled(
                format!(" {}", reason),
                Style::default()
                    .fg(self.palette.state_blocked)
                    .add_modifier(Modifier::BOLD),
            )));
        }

        text.push(Line::from(""));
        for line in &input_lines {
            text.push(Line::from(Span::styled(
                format!(" {}", line),
                Style::default().fg(self.palette.subtext),
            )));
        }

        if let Some(diff) = pending.request.diff_preview.as_deref() {
            text.push(Line::from(""));
            text.push(Line::from(Span::styled(
                " Diff",
                Style::default()
                    .fg(self.palette.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            for line in diff.lines().take(24) {
                let color = if line.starts_with('+') && !line.starts_with("+++") {
                    self.palette.ok
                } else if line.starts_with('-') && !line.starts_with("---") {
                    self.palette.danger
                } else {
                    self.palette.muted
                };
                text.push(Line::from(Span::styled(
                    format!(" {}", line),
                    Style::default().fg(color),
                )));
            }
        }

        text.push(Line::from(""));
        text.push(widgets::action_hints(
            &[
                (self.keys.binding("perm_once"), "once"),
                (self.keys.binding("perm_path"), "path"),
                (self.keys.binding("perm_always"), "always"),
                (self.keys.binding("perm_deny"), "deny"),
            ],
            &self.palette,
        ));
        if let Some(path) = extract_tool_path(&pending.request.tool_input) {
            let prefix = path_allow_prefix(&path);
            text.push(Line::from(Span::styled(
                format!(" path → {} under {}", pending.request.tool_name, prefix),
                Style::default().fg(self.palette.overlay0),
            )));
        }

        let prompt_height = (text.len() as u16 + 2).min(area.height.saturating_sub(4).max(3));
        let prompt_area = widgets::centered_rect(area, 90, prompt_height);
        widgets::render_modal_shell(
            frame,
            prompt_area,
            title,
            border_color,
            &self.palette,
            text,
        );
    }

    fn handle_question_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if let Some(pending) = self.pending_question.take() {
                    let _ = pending.reply_tx.send(QuestionReply::Cancelled);
                }
                self.input.clear();
                self.input_mode = InputMode::Waiting;
                self.status = "question cancelled".to_string();
            }
            KeyCode::Enter => {
                let answer = self.input.trim().to_string();
                if let Some(pending) = self.pending_question.take() {
                    let resolved = if answer.is_empty() {
                        QuestionReply::Cancelled
                    } else if let Ok(n) = answer.parse::<usize>() {
                        if n >= 1 && n <= pending.request.options.len() {
                            QuestionReply::Answer(pending.request.options[n - 1].clone())
                        } else {
                            QuestionReply::Answer(answer)
                        }
                    } else {
                        QuestionReply::Answer(answer)
                    };
                    let _ = pending.reply_tx.send(resolved);
                }
                self.input.clear();
                self.input_mode = InputMode::Waiting;
                self.status = "answered".to_string();
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            _ => {}
        }
    }

    fn render_question_prompt(&mut self, frame: &mut Frame, area: Rect) {
        let pending = match self.pending_question.as_ref() {
            Some(p) => p,
            None => return,
        };

        let mut text = vec![
            Line::from(Span::styled(
                format!(" {}", pending.request.question),
                Style::default()
                    .fg(self.palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
        ];
        if !pending.request.options.is_empty() {
            text.push(Line::from(""));
            for (i, opt) in pending.request.options.iter().enumerate() {
                text.push(Line::from(vec![
                    Span::styled(
                        format!(" {} ", i + 1),
                        Style::default()
                            .fg(self.palette.contrast_on_accent())
                            .bg(self.palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {opt}"),
                        Style::default().fg(self.palette.subtext),
                    ),
                ]));
            }
        }
        text.push(Line::from(""));
        text.push(widgets::action_hints(
            &[("↵", "submit"), ("esc", "cancel")],
            &self.palette,
        ));

        let prompt_height = (text.len() as u16 + 2).min(area.height.saturating_sub(4).max(3));
        let prompt_area = widgets::centered_rect(area, 80, prompt_height);
        widgets::render_modal_shell(
            frame,
            prompt_area,
            "question",
            self.palette.state_blocked,
            &self.palette,
            text,
        );
    }

    fn flush_pending_kitty_images(&mut self) {
        if self.pending_kitty_images.is_empty() {
            return;
        }
        if !super::kitty::kitty_graphics_likely() {
            // Drop quietly unless user forced via env; avoid flooding non-Kitty terms.
            if std::env::var("RS_AGENT_KITTY").is_err() {
                self.pending_kitty_images.clear();
                return;
            }
        }
        let paths: Vec<String> = self.pending_kitty_images.drain(..).collect();
        let mut out = io::stdout();
        for path in paths {
            let _ = super::kitty::write_kitty_image(&mut out, &path, 80);
        }
    }
}
