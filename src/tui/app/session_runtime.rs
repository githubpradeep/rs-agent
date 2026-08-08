//! Per-session agent OS thread — keeps running when the TUI focuses elsewhere.

use crate::agent::r#loop::AgentEvent;
use crate::agent::state::AgentState;
use crate::agent::AgentLoop;
use crate::ai::provider::Provider;
use crate::ai::types::Message;
use crate::permission::PendingPermission;
use crate::session::{SessionData, SessionStore};
use crossbeam_channel as channel;
use std::sync::Arc;
use std::time::Duration;

use super::helpers::summarize_api_messages;
use super::AppCommand;

pub struct SessionRuntime {
    pub id: String,
    pub command_tx: channel::Sender<AppCommand>,
    pub event_rx: channel::Receiver<(usize, AgentEvent)>,
    pub abort_flag: crate::agent::AbortFlag,
    pub steer_queue: crate::agent::SteerQueue,
    pub turn_active: bool,
    /// Stream events while this session was unfocused — replayed on re-attach.
    pub offline_events: Vec<AgentEvent>,
    /// True once the focused TUI has snapped chat for this runtime (includes user bubbles).
    pub has_ui_snapshot: bool,
}

/// Everything needed to boot one agent OS thread.
pub struct SessionRuntimeConfig {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub timeout_secs: u64,
    pub approve: bool,
    pub max_iterations: usize,
    pub rlm_depth: u32,
    pub thinking_budget: Option<u32>,
    pub system_prompt: Option<String>,
    pub permission_tx: channel::Sender<PendingPermission>,
    pub id: String,
    pub created_at: String,
    pub title: Option<String>,
    pub parent_id: Option<String>,
    pub branch_label: Option<String>,
    pub messages: Vec<Message>,
    pub goal: Option<crate::agent::goal::GoalState>,
    pub seat: Option<String>,
    pub handoff: Option<crate::agent::handoff::HandoffNotes>,
    pub todos: Option<Vec<crate::tools::todowrite::TodoItem>>,
}

impl SessionRuntimeConfig {
    pub fn from_session_data(
        data: &SessionData,
        provider: Arc<dyn Provider>,
        model: String,
        timeout_secs: u64,
        approve: bool,
        max_iterations: usize,
        rlm_depth: u32,
        thinking_budget: Option<u32>,
        system_prompt: Option<String>,
        permission_tx: channel::Sender<PendingPermission>,
    ) -> Self {
        let goal = data.goal.clone().filter(|g| {
            matches!(
                g.status,
                crate::agent::goal::GoalStatus::Active | crate::agent::goal::GoalStatus::Paused
            )
        });
        Self {
            provider,
            model,
            timeout_secs,
            approve,
            max_iterations,
            rlm_depth,
            thinking_budget,
            system_prompt: system_prompt.or_else(|| {
                if data.system_prompt.trim().is_empty() {
                    None
                } else {
                    Some(data.system_prompt.clone())
                }
            }),
            permission_tx,
            id: data.id.clone(),
            created_at: data.created_at.clone(),
            title: data.title.clone(),
            parent_id: data.parent_id.clone(),
            branch_label: data.branch_label.clone(),
            messages: data.messages.clone(),
            goal,
            seat: data.seat.clone(),
            handoff: data.handoff.clone(),
            todos: data.todos.clone(),
        }
    }

    pub fn empty_new(
        provider: Arc<dyn Provider>,
        model: String,
        timeout_secs: u64,
        approve: bool,
        max_iterations: usize,
        rlm_depth: u32,
        thinking_budget: Option<u32>,
        system_prompt: Option<String>,
        permission_tx: channel::Sender<PendingPermission>,
    ) -> Self {
        Self {
            provider,
            model,
            timeout_secs,
            approve,
            max_iterations,
            rlm_depth,
            thinking_budget,
            system_prompt,
            permission_tx,
            id: SessionStore::generate_id(),
            created_at: chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
            title: None,
            parent_id: None,
            branch_label: None,
            messages: Vec::new(),
            goal: None,
            seat: None,
            handoff: None,
            todos: None,
        }
    }
}

pub fn spawn_session_runtime(cfg: SessionRuntimeConfig) -> SessionRuntime {
    let (command_tx, command_rx) = channel::unbounded::<AppCommand>();
    let (event_tx, event_rx) = channel::unbounded::<(usize, AgentEvent)>();
    let abort_flag = crate::agent::AbortFlag::new();
    let steer_queue = crate::agent::SteerQueue::new();
    let abort_for_thread = abort_flag.clone();
    let steer_for_thread = steer_queue.clone();
    let id = cfg.id.clone();

    let provider_name = cfg.provider.name().to_string();
    let provider2 = cfg.provider;
    let model2 = cfg.model;
    let timeout = Duration::from_secs(cfg.timeout_secs);
    let timeout_secs = cfg.timeout_secs;
    let approve = cfg.approve;
    let max_iterations = cfg.max_iterations;
    let max_rlm_depth = cfg.rlm_depth;
    let thinking_budget = cfg.thinking_budget;
    let system_prompt_for_thread = cfg.system_prompt;
    let permission_tx = cfg.permission_tx;
    let session_id_for_thread = cfg.id;
    let created_at_for_thread = cfg.created_at;
    let title_for_thread = cfg.title;
    let parent_for_thread = cfg.parent_id;
    let branch_for_thread = cfg.branch_label;
    let resume_msgs = cfg.messages;
    let resume_goal = cfg.goal;
    let resume_seat = cfg.seat;
    let resume_handoff = cfg.handoff;
    let resume_todos = cfg.todos;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let sp = system_prompt_for_thread
                .unwrap_or_else(crate::agent::default_system_prompt);

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
            state.pending_wake =
                state.seat.is_some() || state.handoff.is_some() || state.goal.is_some();

            for msg in &resume_msgs {
                state.add_message(msg.clone());
            }

            if let Some(todos) = resume_todos {
                crate::tools::todowrite::restore(todos);
            } else {
                crate::tools::todowrite::clear();
            }

            let goal_verify = crate::config::Config::load().goal_verify.unwrap_or(true);
            let mut agent_loop = AgentLoop::new(provider2, state)
                .with_max_iterations(max_iterations)
                .with_abort(abort_for_thread.clone())
                .with_steer(steer_for_thread.clone())
                .with_rlm_depth(0, max_rlm_depth)
                .with_goal_verify(goal_verify);
            if !approve {
                agent_loop.set_permission_channel(permission_tx);
            }
            // Bridge tool-emitted events onto this runtime's event channel.
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
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: "steer queued".to_string(),
                            },
                        ));
                    }
                    AppCommand::Compact => {
                        let _ = agent_loop
                            .compact_now(&mut |event: AgentEvent| {
                                let _ = event_tx.send((0, event));
                            })
                            .await;
                    }
                    AppCommand::NewSession => {
                        agent_loop.clear_messages();
                        agent_loop.clear_goal();
                        agent_loop.state_mut().seat = None;
                        agent_loop.state_mut().handoff = None;
                        agent_loop.state_mut().pending_wake = false;
                        agent_loop.state_mut().total_input_tokens = 0;
                        agent_loop.state_mut().total_output_tokens = 0;
                        crate::agent::handoff::clear();
                        crate::tools::handoff::set_active_seat(None);
                        abort_for_thread.clear();
                        steer_for_thread.clear();
                        crate::tools::todowrite::clear();
                        session_id_local = SessionStore::generate_id();
                        created_at_local =
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                        title_local = None;
                        parent_id_local = None;
                        branch_label_local = None;
                        let _ = event_tx.send((
                            0,
                            AgentEvent::SessionMeta {
                                id: session_id_local.clone(),
                                title: None,
                            },
                        ));
                        let _ = event_tx.send((
                            0,
                            AgentEvent::ReloadTranscript {
                                messages: Vec::new(),
                            },
                        ));
                        let _ = event_tx.send((
                            0,
                            AgentEvent::GoalUpdate {
                                summary: String::new(),
                            },
                        ));
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!("new session {}", session_id_local),
                            },
                        ));
                    }
                    AppCommand::ForkSession { label, at } => {
                        let now =
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
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
                            project_root: None,
                        };
                        current.ensure_title();
                        current.stamp_project();
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
                                let _ = event_tx.send((
                                    0,
                                    AgentEvent::SessionMeta {
                                        id: session_id_local.clone(),
                                        title: title_local.clone(),
                                    },
                                ));
                                if let Some(ref g) = forked.goal {
                                    let _ = event_tx.send((
                                        0,
                                        AgentEvent::GoalUpdate {
                                            summary: format!(
                                                "{}: {}",
                                                g.status.as_str(),
                                                g.condition
                                            ),
                                        },
                                    ));
                                }
                                let _ = event_tx.send((
                                    0,
                                    AgentEvent::ReloadTranscript {
                                        messages: forked.messages.clone(),
                                    },
                                ));
                                let _ = event_tx.send((
                                    0,
                                    AgentEvent::TimelineSnapshot {
                                        entries: summarize_api_messages(&forked.messages),
                                    },
                                ));
                                let _ = event_tx.send((
                                    0,
                                    AgentEvent::Status {
                                        message: format!(
                                            "forked → {} (parent {}){}",
                                            session_id_local,
                                            parent_id_local.as_deref().unwrap_or("?"),
                                            at.map(|n| format!(" @{n}")).unwrap_or_default()
                                        ),
                                    },
                                ));
                            }
                            Err(e) => {
                                let _ = event_tx.send((
                                    0,
                                    AgentEvent::Error {
                                        message: format!("fork failed: {e}"),
                                    },
                                ));
                            }
                        }
                    }
                    AppCommand::RequestTimeline => {
                        let entries = summarize_api_messages(&agent_loop.state().messages);
                        let _ = event_tx.send((0, AgentEvent::TimelineSnapshot { entries }));
                    }
                    AppCommand::SetModel { model } => {
                        agent_loop.set_model(model.clone());
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!("model set to {model}"),
                            },
                        ));
                    }
                    AppCommand::SetProvider { provider, model } => {
                        let pname = provider.name().to_string();
                        agent_loop.set_provider_and_model(provider, model.clone());
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!("provider {pname} · model {model}"),
                            },
                        ));
                    }
                    AppCommand::SetMode { mode } => {
                        agent_loop.state_mut().set_mode(mode);
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!("mode set to {}", mode.as_str()),
                            },
                        ));
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
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: "system prompt rebuilt".to_string(),
                            },
                        ));
                    }
                    AppCommand::GoalSet { condition } => {
                        agent_loop.set_goal(condition.clone());
                        let _ = event_tx.send((
                            0,
                            AgentEvent::GoalUpdate {
                                summary: format!("active: {condition}"),
                            },
                        ));
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: "◎ /goal set — starting turn".into(),
                            },
                        ));
                    }
                    AppCommand::GoalClear => {
                        let msg = match agent_loop.clear_goal() {
                            Some(g) => format!("Goal cleared: {}", g.condition),
                            None => "No goal set".into(),
                        };
                        let _ = event_tx.send((
                            0,
                            AgentEvent::GoalUpdate {
                                summary: String::new(),
                            },
                        ));
                        let _ = event_tx.send((0, AgentEvent::Status { message: msg }));
                    }
                    AppCommand::GoalPause => {
                        let msg = if agent_loop.pause_goal() {
                            "Goal paused".into()
                        } else {
                            "No active goal to pause".into()
                        };
                        let _ = event_tx.send((
                            0,
                            AgentEvent::GoalUpdate {
                                summary: "paused".into(),
                            },
                        ));
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
                            Some(g) => {
                                g.status_line(s.total_input_tokens, s.total_output_tokens)
                            }
                            None => "No goal set. Usage: /goal <condition>".into(),
                        };
                        let _ = event_tx.send((0, AgentEvent::Status { message: msg.clone() }));
                        let _ = event_tx.send((
                            0,
                            AgentEvent::GoalUpdate {
                                summary: format!("STATUS\n{msg}"),
                            },
                        ));
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
                            let result =
                                tokio::time::timeout(timeout, agent_loop.run(&prompt, &mut cb))
                                    .await;
                            match result {
                                Ok(Ok(())) => {
                                    let _ = event_tx.send((
                                        0,
                                        AgentEvent::TreeUpdate {
                                            tree: agent_loop.call_tree().clone(),
                                        },
                                    ));
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
                                        let _ = event_tx.send((
                                            0,
                                            AgentEvent::Status {
                                                message: format!(
                                                    "wall timeout after {timeout_secs}s — auto-continuing ({wall_attempt}/{MAX_WALL_RETRIES}) in {}s…",
                                                    delay.as_secs()
                                                ),
                                            },
                                        ));
                                        tokio::time::sleep(delay).await;
                                        prompt = "continue".to_string();
                                        continue;
                                    }
                                    let _ = event_tx.send((
                                        0,
                                        AgentEvent::Error {
                                            message: format!(
                                                "Request timed out after {timeout_secs}s (auto-retry exhausted)"
                                            ),
                                        },
                                    ));
                                    break;
                                }
                            }
                        }
                        let now =
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
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
                            project_root: None,
                        };
                        session_data.ensure_title();
                        session_data.stamp_project();
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
                        agent_loop.state_mut().total_input_tokens = data.total_input_tokens;
                        agent_loop.state_mut().total_output_tokens = data.total_output_tokens;
                        agent_loop.state_mut().pending_wake = data.seat.is_some()
                            || data.handoff.is_some()
                            || data.goal.as_ref().is_some_and(|g| {
                                matches!(
                                    g.status,
                                    crate::agent::goal::GoalStatus::Active
                                        | crate::agent::goal::GoalStatus::Paused
                                )
                            });
                        crate::agent::handoff::restore(data.handoff.clone());
                        crate::tools::handoff::set_active_seat(data.seat.clone());
                        if let Some(todos) = data.todos.clone() {
                            crate::tools::todowrite::restore(todos);
                        } else {
                            crate::tools::todowrite::clear();
                        }
                        agent_loop.state_mut().messages = data.messages.clone();
                        let msg_count = data.messages.len();
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
                        } else {
                            let _ = event_tx.send((
                                0,
                                AgentEvent::GoalUpdate {
                                    summary: String::new(),
                                },
                            ));
                        }
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!(
                                    "loaded session {} ({msg_count} msgs)",
                                    data.id
                                ),
                            },
                        ));
                    }
                    AppCommand::SwitchSession { data } => {
                        let target_id = data.id.clone();
                        {
                            let now = chrono::Local::now()
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string();
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
                                handoff: s
                                    .handoff
                                    .clone()
                                    .or_else(crate::agent::handoff::snapshot),
                                project_root: None,
                            };
                            session_data.ensure_title();
                            session_data.stamp_project();
                            let _ = store.save(&session_data);
                        }
                        let data = store.load(&target_id).unwrap_or(data);
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
                        agent_loop.state_mut().total_input_tokens = data.total_input_tokens;
                        agent_loop.state_mut().total_output_tokens = data.total_output_tokens;
                        agent_loop.state_mut().pending_wake = data.seat.is_some()
                            || data.handoff.is_some()
                            || data.goal.as_ref().is_some_and(|g| {
                                matches!(
                                    g.status,
                                    crate::agent::goal::GoalStatus::Active
                                        | crate::agent::goal::GoalStatus::Paused
                                )
                            });
                        crate::agent::handoff::restore(data.handoff.clone());
                        crate::tools::handoff::set_active_seat(data.seat.clone());
                        if let Some(todos) = data.todos.clone() {
                            crate::tools::todowrite::restore(todos);
                        } else {
                            crate::tools::todowrite::clear();
                        }
                        agent_loop.state_mut().messages = data.messages.clone();
                        let msg_count = data.messages.len();
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
                        } else {
                            let _ = event_tx.send((
                                0,
                                AgentEvent::GoalUpdate {
                                    summary: String::new(),
                                },
                            ));
                        }
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!(
                                    "switched to {} ({msg_count} msgs)",
                                    data.id
                                ),
                            },
                        ));
                    }
                    AppCommand::PersistSession => {
                        let now =
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
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
                            project_root: None,
                        };
                        session_data.ensure_title();
                        session_data.stamp_project();
                        title_local = session_data.title.clone();
                        let _ = store.save(&session_data);
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!("persisted session {session_id_local}"),
                            },
                        ));
                    }
                    AppCommand::SetTitle { title } => {
                        title_local = Some(title.clone());
                        let now =
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
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
                            project_root: None,
                        };
                        session_data.stamp_project();
                        let _ = store.save(&session_data);
                        let _ = event_tx.send((
                            0,
                            AgentEvent::Status {
                                message: format!("renamed to \"{title}\""),
                            },
                        ));
                    }
                    AppCommand::Submit { text } => {
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
                                    let _ = event_tx.send((
                                        0,
                                        AgentEvent::TreeUpdate {
                                            tree: agent_loop.call_tree().clone(),
                                        },
                                    ));
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
                                        let _ = event_tx.send((
                                            0,
                                            AgentEvent::Status {
                                                message: format!(
                                                    "provider error — auto-continuing ({wall_attempt}/{MAX_WALL_RETRIES}) in {}s… ({e})",
                                                    delay.as_secs()
                                                ),
                                            },
                                        ));
                                        tokio::time::sleep(delay).await;
                                        prompt = "continue".to_string();
                                        continue;
                                    }
                                    let _ = event_tx.send((
                                        0,
                                        AgentEvent::Error {
                                            message: format!("{e} (auto-retry exhausted)"),
                                        },
                                    ));
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
                                        let _ = event_tx.send((
                                            0,
                                            AgentEvent::Status {
                                                message: format!(
                                                    "wall timeout after {timeout_secs}s — auto-continuing ({wall_attempt}/{MAX_WALL_RETRIES}) in {}s…",
                                                    delay.as_secs()
                                                ),
                                            },
                                        ));
                                        tokio::time::sleep(delay).await;
                                        prompt = "continue".to_string();
                                        continue;
                                    }
                                    let _ = event_tx.send((
                                        0,
                                        AgentEvent::Error {
                                            message: format!(
                                                "Request timed out after {timeout_secs}s (auto-retry exhausted)"
                                            ),
                                        },
                                    ));
                                    break;
                                }
                            }
                        }
                        let now =
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
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
                            project_root: None,
                        };
                        session_data.ensure_title();
                        session_data.stamp_project();
                        title_local = session_data.title.clone();
                        if let Some(ref t) = title_local {
                            let _ = event_tx.send((
                                0,
                                AgentEvent::TitleUpdate { title: t.clone() },
                            ));
                        }
                        let _ = store.save(&session_data);
                    }
                }
            }
        });
    });

    SessionRuntime {
        id,
        command_tx,
        event_rx,
        abort_flag,
        steer_queue,
        turn_active: false,
        offline_events: Vec::new(),
        has_ui_snapshot: false,
    }
}

/// Whether an agent event marks a turn as active / finished.
///
/// Only explicit completion clears activity. Do **not** treat `TurnEnd` /
/// `TreeUpdate` as active — those often arrive after `Done` and would leave
/// `turn_active` stuck true (UI shows "thinking..." after switch).
pub fn turn_active_from_event(ev: &AgentEvent) -> Option<bool> {
    match ev {
        AgentEvent::TextDelta { .. }
        | AgentEvent::ThinkingDelta { .. }
        | AgentEvent::ToolUseStart { .. }
        | AgentEvent::ToolUseDelta { .. }
        | AgentEvent::ToolResult { .. }
        | AgentEvent::ToolOutput { .. }
        | AgentEvent::ReplOutput { .. }
        | AgentEvent::Compacting => Some(true),
        AgentEvent::Done | AgentEvent::Aborted | AgentEvent::Error { .. } => Some(false),
        _ => None,
    }
}
