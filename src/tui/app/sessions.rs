//! Multi-session lifecycle — new/fork/switch, focus routing between
//! session runtimes, and applying saved session data back onto the UI.
//!
use super::helpers::*;
use super::*;

impl App {
    pub(super) fn toggle_sessions_panel(&mut self) {
        self.show_sessions_panel = !self.show_sessions_panel;
        if self.show_sessions_panel {
            self.show_timeline_panel = false;
            self.show_tree_panel = false;
            self.sessions_panel
                .refresh(&self.session_id, self.bg_running_session.as_deref());
            self.focus_zone = FocusZone::Sessions;
        } else if self.focus_zone == FocusZone::Sessions {
            self.focus_zone = FocusZone::Chat;
        }
    }

    /// Rebuild chat + timeline from API messages, keeping the banner system opener.
    pub(super) fn apply_transcript_messages(&mut self, messages: &[Message]) {
        let opener = self
            .messages
            .first()
            .filter(|m| m.role == "system")
            .cloned();
        let mut rebuilt = Vec::new();
        if let Some(sys) = opener {
            rebuilt.push(sys);
        }
        rebuilt.extend(api_messages_to_chat(messages));
        self.messages = rebuilt;
        self.timeline_entries = summarize_api_messages(messages);
        if self.timeline_selection >= self.timeline_entries.len() {
            self.timeline_selection = self.timeline_entries.len().saturating_sub(1);
        }
        self.follow_bottom = true;
        self.scroll_offset = 0;
    }

    /// Instantly update chat/header/panel from a loaded session (don't wait on agent thread).
    fn apply_session_data_to_ui(&mut self, data: &SessionData) {
        self.session_id = data.id.clone();
        self.session_title = data.title.clone().filter(|t| !t.is_empty());
        self.session_input_tokens = data.total_input_tokens;
        self.session_output_tokens = data.total_output_tokens;
        self.token_used = data
            .total_input_tokens
            .saturating_add(data.total_output_tokens);
        self.apply_transcript_messages(&data.messages);
        self.goal_indicator = match &data.goal {
            Some(g)
                if matches!(
                    g.status,
                    crate::agent::goal::GoalStatus::Active | crate::agent::goal::GoalStatus::Paused
                ) =>
            {
                format!("◎ /goal {}", g.status.as_str())
            }
            _ => String::new(),
        };
        self.tool_output_tabs.clear();
        self.tool_output_tab = 0;
        self.show_repl_panel = false;
        self.tool_in_progress = None;
        self.input_mode = InputMode::Insert;
        self.input.clear();
        self.status = "ready".into();
        self.near_limit = false;
        self.queued_steers = 0;
        self.provider_auto_continues = 0;
        crate::tools::turn_snapshot::set_session(&self.session_id);
        crate::agent::trajectory::set_session(&self.session_id);
        if self.show_sessions_panel {
            self.sessions_panel
                .refresh(&self.session_id, self.bg_running_session.as_deref());
        }
    }

    fn clear_chat_ui_for_new_session(&mut self) {
        let opener = self
            .messages
            .first()
            .filter(|m| m.role == "system")
            .cloned();
        self.messages = opener.into_iter().collect();
        self.timeline_entries.clear();
        self.timeline_selection = 0;
        self.goal_indicator.clear();
        self.session_title = None;
        self.session_input_tokens = 0;
        self.session_output_tokens = 0;
        self.token_used = 0;
        self.tool_output_tabs.clear();
        self.tool_output_tab = 0;
        self.show_repl_panel = false;
        self.tool_in_progress = None;
        self.input_mode = InputMode::Insert;
        self.input.clear();
        self.status = "new session…".into();
        self.follow_bottom = true;
        self.scroll_offset = 0;
        self.near_limit = false;
        self.queued_steers = 0;
        self.provider_auto_continues = 0;
    }

    fn focus_runtime(&mut self, id: &str) {
        if let Some(rt) = self.runtimes.get(id) {
            self.command_tx = rt.command_tx.clone();
            self.abort_flag = rt.abort_flag.clone();
            self.steer_queue = rt.steer_queue.clone();
            self.agent_turn_active = rt.turn_active;
        }
        self.session_id = id.to_string();
        self.agent_session_id = id.to_string();
        self.refresh_bg_running();
        crate::tools::turn_snapshot::set_session(&self.session_id);
        crate::agent::trajectory::set_session(&self.session_id);
    }

    fn apply_focus_ui_state(&mut self, id: &str, data: Option<&SessionData>) {
        let title = data
            .and_then(|d| d.title.clone())
            .unwrap_or_else(|| "(untitled)".into());
        let short = SessionStore::short_id(id).to_string();
        let snap = self.ui_snapshots.get(id).cloned();
        let has_snap = snap.is_some();

        if let Some(data) = data {
            self.session_title = data.title.clone().filter(|t| !t.is_empty());
            self.session_input_tokens = data.total_input_tokens;
            self.session_output_tokens = data.total_output_tokens;
            self.token_used = data
                .total_input_tokens
                .saturating_add(data.total_output_tokens);
            if !has_snap {
                self.apply_session_data_to_ui(data);
            }
        }
        if let Some(snap) = snap {
            self.messages = snap;
            self.follow_bottom = true;
            self.scroll_offset = 0;
        }

        let still_active = self
            .runtimes
            .get(id)
            .map(|r| r.turn_active)
            .unwrap_or(false);
        if still_active {
            if !has_snap {
                self.replay_offline_events(id);
            } else if let Some(rt) = self.runtimes.get_mut(id) {
                rt.offline_events.clear();
            }
            for ev in self.pump_runtime_events() {
                self.handle_agent_event(ev);
            }
            self.ui_snapshots
                .insert(id.to_string(), self.messages.clone());
            let still_active = self
                .runtimes
                .get(id)
                .map(|r| r.turn_active)
                .unwrap_or(false);
            if still_active {
                self.input_mode = InputMode::Waiting;
                self.status = "thinking...".into();
                self.agent_turn_active = true;
                self.push_system(format!("Focused live turn on {short} — {title}"));
            } else {
                self.input_mode = InputMode::Insert;
                self.status = "ready".into();
                self.agent_turn_active = false;
                self.tool_in_progress = None;
                self.push_system(format!("Switched to {short} — {title}"));
            }
        } else {
            if let Some(rt) = self.runtimes.get_mut(id) {
                rt.offline_events.clear();
            }
            self.ui_snapshots
                .insert(id.to_string(), self.messages.clone());
            self.input_mode = InputMode::Insert;
            self.status = "ready".into();
            self.agent_turn_active = false;
            self.tool_in_progress = None;
            self.push_system(format!("Switched to {short} — {title}"));
        }
    }

    /// Replay stream buffered while this session was unfocused.
    fn replay_offline_events(&mut self, id: &str) {
        let events = self
            .runtimes
            .get_mut(id)
            .map(|r| std::mem::take(&mut r.offline_events))
            .unwrap_or_default();
        for ev in events {
            self.handle_agent_event(ev);
        }
    }

    pub(super) fn refresh_bg_running(&mut self) {
        let focused = self.session_id.clone();
        self.bg_running_session = self
            .runtimes
            .iter()
            .find(|(id, rt)| rt.turn_active && *id != &focused)
            .map(|(id, _)| id.clone());
    }

    fn spawn_config_for(&self, data: &SessionData) -> SessionRuntimeConfig {
        SessionRuntimeConfig::from_session_data(
            data,
            self.provider.clone(),
            self.model_name.clone(),
            self.timeout_secs,
            self.approved,
            self.max_iterations,
            self.rlm_depth,
            self.thinking_budget,
            self.system_prompt_opt.clone(),
            self.permission_tx.clone(),
        )
    }

    pub(super) fn start_new_project_session(&mut self) {
        // Persist the focused session; do not abort or NewSession the old runtime
        // (it may still be running a turn in parallel).
        let _ = self.command_tx.send(AppCommand::PersistSession);
        for ev in self.pump_runtime_events() {
            self.handle_agent_event(ev);
        }
        self.snapshot_focused_chat();

        let cfg = SessionRuntimeConfig::empty_new(
            self.provider.clone(),
            self.model_name.clone(),
            self.timeout_secs,
            self.approved,
            self.max_iterations,
            self.rlm_depth,
            self.thinking_budget,
            self.system_prompt_opt.clone(),
            self.permission_tx.clone(),
        );
        let new_id = cfg.id.clone();
        let rt = spawn_session_runtime(cfg);
        self.runtimes.insert(new_id.clone(), rt);
        self.focus_runtime(&new_id);
        self.clear_chat_ui_for_new_session();
        self.ui_snapshots
            .insert(new_id.clone(), self.messages.clone());
        if let Some(rt) = self.runtimes.get_mut(&new_id) {
            rt.has_ui_snapshot = true;
        }
        self.status = "ready".into();
        self.push_system(format!(
            "Started new session {} (previous session keeps running if mid-turn).",
            SessionStore::short_id(&new_id)
        ));
        if self.show_sessions_panel {
            self.sessions_panel
                .refresh(&self.session_id, self.bg_running_session.as_deref());
        }
    }

    fn switch_to_session_id(&mut self, id: &str) {
        if id == self.session_id {
            self.push_system(format!("Already on session {}", SessionStore::short_id(id)));
            return;
        }

        // Persist focused session before leaving (non-blocking command).
        let _ = self.command_tx.send(AppCommand::PersistSession);
        // Drain so turn_active flags are up to date before we decide Waiting vs ready.
        for ev in self.pump_runtime_events() {
            self.handle_agent_event(ev);
        }
        // Keep the live transcript (including typed user bubbles) — disk often
        // lags behind PersistSession and offline TextDelta replay drops users.
        self.snapshot_focused_chat();

        if self.runtimes.contains_key(id) {
            self.focus_runtime(id);
            match SessionStore::new().load(id) {
                Ok(data) => self.apply_focus_ui_state(id, Some(&data)),
                Err(e) => {
                    self.push_system(format!("Runtime attached, but failed to load UI: {e}"));
                    self.apply_focus_ui_state(id, None);
                }
            }
            if self.show_sessions_panel {
                self.sessions_panel
                    .refresh(&self.session_id, self.bg_running_session.as_deref());
            }
            return;
        }

        let data = match SessionStore::new().load(id) {
            Ok(d) => d,
            Err(e) => {
                self.push_system(format!("Failed to load session: {e}"));
                return;
            }
        };
        let title = data.title.clone().unwrap_or_else(|| "(untitled)".into());
        let short = SessionStore::short_id(&data.id).to_string();
        let msg_count = data.messages.len();
        let cfg = self.spawn_config_for(&data);
        let rt = spawn_session_runtime(cfg);
        self.runtimes.insert(data.id.clone(), rt);
        self.focus_runtime(&data.id);
        self.apply_session_data_to_ui(&data);
        self.ui_snapshots
            .insert(data.id.clone(), self.messages.clone());
        if let Some(rt) = self.runtimes.get_mut(&data.id) {
            rt.has_ui_snapshot = true;
        }
        self.input_mode = InputMode::Insert;
        self.status = "ready".into();
        self.agent_turn_active = false;
        self.tool_in_progress = None;
        self.push_system(format!("Switched to {short} — {title} ({msg_count} msgs)"));
        if self.show_sessions_panel {
            self.sessions_panel
                .refresh(&self.session_id, self.bg_running_session.as_deref());
        }
    }

    pub(super) fn activate_sessions_selection(&mut self) {
        match self.sessions_panel.selected_row().cloned() {
            Some(SessionRow::Session { summary }) => {
                self.switch_to_session_id(&summary.id);
            }
            Some(SessionRow::Action { id, .. }) => match id {
                "new" => self.start_new_project_session(),
                "toggle_scope" => {
                    self.sessions_panel.show_all = !self.sessions_panel.show_all;
                    self.sessions_panel
                        .refresh(&self.session_id, self.bg_running_session.as_deref());
                }
                _ => {}
            },
            _ => {
                self.push_system("Nothing selected — ↑↓ to a session or action.");
            }
        }
    }
}
