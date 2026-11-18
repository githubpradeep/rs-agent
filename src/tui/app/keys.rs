//! Keyboard handlers per input mode (normal / insert / waiting) plus the
//! overlay key handlers (help, settings, palette).
//!
use super::*;

impl App {
    pub(super) fn handle_waiting_key(&mut self, key: crossterm::event::KeyEvent) {
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
            KeyCode::PageUp if self.show_repl_panel => {
                // Scroll the console buffer up (toward older output).
                self.console_scroll = self.console_scroll.saturating_add(10);
            }
            KeyCode::PageDown if self.show_repl_panel => {
                self.console_scroll = self.console_scroll.saturating_sub(10);
            }
            KeyCode::Enter => {
                if !self.input.trim().is_empty() {
                    let text = std::mem::take(&mut self.input);
                    self.queued_steers += 1;
                    self.steer_queue.push(text.clone());
                    let _ = self
                        .command_tx
                        .send(AppCommand::Steer { text: text.clone() });
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

    pub(super) fn handle_help_key(&mut self, key: crossterm::event::KeyEvent) {
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

    pub(super) fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
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
            _ => {}
        }
    }

    pub(super) fn handle_palette_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => self.dismiss_overlays(),
            KeyCode::Up => {
                self.palette_selection = self.palette_selection.saturating_sub(1);
            }
            KeyCode::Down => {
                self.palette_selection = self.palette_selection.saturating_add(1);
                if !self.palette_items.is_empty() {
                    self.palette_selection =
                        self.palette_selection.min(self.palette_items.len() - 1);
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

    pub(super) fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) {
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
        if self.show_sessions_panel {
            match key.code {
                KeyCode::Up => {
                    self.sessions_panel.move_sel(-1);
                    return;
                }
                KeyCode::Down => {
                    self.sessions_panel.move_sel(1);
                    return;
                }
                KeyCode::Enter => {
                    self.activate_sessions_selection();
                    return;
                }
                KeyCode::Char('n') => {
                    self.start_new_project_session();
                    return;
                }
                KeyCode::Char('a') => {
                    self.sessions_panel.show_all = !self.sessions_panel.show_all;
                    self.sessions_panel
                        .refresh(&self.session_id, self.bg_running_session.as_deref());
                    return;
                }
                _ => {}
            }
        }
        if self.key_matches("insert", key) {
            self.input_mode = InputMode::Insert;
            self.focus_zone = FocusZone::Chat;
            self.unseen_done = false;
            return;
        }
        if self.key_matches("quit", key) {
            self.should_exit = true;
            return;
        }
        if self.key_matches("toggle_thinking", key) {
            if let Some(msg) = self.messages.iter_mut().rev().find(|m| {
                m.role == "assistant" && m.thinking.as_ref().is_some_and(|t| !t.is_empty())
            }) {
                msg.show_thinking = !msg.show_thinking;
            }
            return;
        }
        if self.key_matches("expand_tool", key) {
            if let Some(msg) = self
                .messages
                .iter_mut()
                .rev()
                .find(|m| !m.tool_blocks.is_empty())
            {
                if let Some(block) = msg.tool_blocks.last_mut() {
                    block.expanded = !block.expanded;
                }
            }
            return;
        }
        if self.key_matches("toggle_sessions", key) {
            self.toggle_sessions_panel();
            return;
        }
        if self.key_matches("toggle_tree", key) {
            if self.show_tree_panel && !self.show_sessions_panel {
                self.side_mode = self.side_mode.toggle();
                self.status = format!("side · {}", self.side_mode.label());
            } else {
                self.show_sessions_panel = false;
                self.show_timeline_panel = false;
                self.show_tree_panel = true;
                self.side_mode = SidePanelMode::Tree;
                self.focus_zone = FocusZone::Tree;
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
                self.scroll_offset = self.scroll_offset.saturating_sub(self.page_rows());
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(self.page_rows());
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Vim-style half page up.
                self.follow_bottom = false;
                self.scroll_offset = self.scroll_offset.saturating_sub(self.page_rows() / 2);
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_offset = self.scroll_offset.saturating_add(self.page_rows() / 2);
            }
            _ => {}
        }
    }

    /// Page size for chat scrolling (viewport minus header/footer rows).
    fn page_rows(&self) -> usize {
        self.chat_rows.saturating_sub(2).max(10)
    }

    pub(super) fn handle_insert_key(&mut self, key: crossterm::event::KeyEvent) {
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
                    if let Some(selected) = self.picker_results.get(self.picker_selection).cloned()
                    {
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
                            PickerMode::Model => match self.apply_model_selection(&selected) {
                                Ok(msg) => {
                                    self.input.clear();
                                    self.push_system(msg);
                                }
                                Err(e) => self.push_system(e),
                            },
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

    /// True if `key` matches the configured binding for `action`, trying
    /// both the exact case and the opposite case of a single-char binding
    /// (so e.g. a binding of "a" also matches Shift-A).
    pub(super) fn key_matches(&self, action: &str, key: KeyEvent) -> bool {
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
}
