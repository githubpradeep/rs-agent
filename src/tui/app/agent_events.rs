//! AgentEvent pump handling — streaming deltas into chat bubbles,
//! console tool tabs, and offline replay snapshots.
//!
use super::helpers::*;
use super::*;

impl App {
    pub(super) fn handle_agent_event(&mut self, event: AgentEvent) {
        // Drop stale stream chunks when we are not in an active turn (e.g. idle after switch).
        // Never drop Done/Aborted/Error — those clear agent_turn_active.
        let streaming = matches!(
            event,
            AgentEvent::TextDelta { .. }
                | AgentEvent::ThinkingDelta { .. }
                | AgentEvent::ToolUseStart { .. }
                | AgentEvent::ToolUseDelta { .. }
                | AgentEvent::ToolResult { .. }
                | AgentEvent::ToolOutput { .. }
                | AgentEvent::ReplOutput { .. }
                | AgentEvent::TurnEnd { .. }
        );
        if streaming && self.input_mode != InputMode::Waiting && self.tool_in_progress.is_none() {
            return;
        }
        match event {
            AgentEvent::TextDelta { text } => {
                self.agent_turn_active = true;
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
                // Respect the user's scroll position: if they scrolled up mid-turn,
                // do not yank them back to the tail on every streamed chunk.
                // follow_bottom stays true when they are already at the tail.
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
            }
            AgentEvent::ToolUseStart { id, name } => {
                self.status = format!("using {}...", name);
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
                    self.console_scroll = 0;
                    // Bottom console only — never steal chat width mid-turn.
                    self.show_repl_panel = true;
                }
                // Open a live tool block in chat so the command is visible
                // while the tool runs; input streams in via ToolUseDelta.
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
                        id: Some(id),
                        name: name.clone(),
                        preview: "…".to_string(),
                        full: String::new(),
                        expanded: false,
                        is_error: false,
                    });
                    self.follow_bottom = true;
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
                    if let Some(rest) = full.split_once('\n').map(|x| x.1) {
                        full = rest.to_string();
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
                // Finalize the live block opened at ToolUseStart when present;
                // otherwise (offline replay, resumed turn) push a fresh block.
                let live = {
                    let mut hit = None;
                    for (mi, msg) in self.messages.iter().enumerate().rev() {
                        if let Some(bi) = msg
                            .tool_blocks
                            .iter()
                            .rposition(|b| b.id.as_deref() == Some(id.as_str()))
                        {
                            hit = Some((mi, bi));
                            break;
                        }
                    }
                    hit
                };
                if let Some((mi, bi)) = live {
                    let block = &mut self.messages[mi].tool_blocks[bi];
                    block.preview = preview;
                    block.full = full.clone();
                    block.expanded = expand_diag || expand_spill || result.is_error;
                    block.is_error = result.is_error || expand_diag;
                } else if let Some(last) = self.messages.last_mut() {
                    last.tool_blocks.push(ToolBlock {
                        id: Some(id),
                        name,
                        preview,
                        full: full.clone(),
                        expanded: expand_diag || expand_spill || result.is_error,
                        is_error: result.is_error || expand_diag,
                    });
                }
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
                    self.provider_auto_continues = self.provider_auto_continues.saturating_add(1);
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
                    self.agent_turn_active = false;
                    self.refresh_bg_running();
                    self.push_system(format!(
                        "Provider hiccup: {message}\n\
                         Auto-retry exhausted — type continue (session stayed open)."
                    ));
                    self.status = "recover".to_string();
                    self.input_mode = InputMode::Insert;
                    self.tool_in_progress = None;
                } else if let Some(last) = self.messages.last_mut() {
                    last.text.push_str(&format!("\n❌ Error: {}", message));
                    self.agent_turn_active = false;
                    self.refresh_bg_running();
                    self.status = "error".to_string();
                    self.input_mode = InputMode::Insert;
                    self.tool_in_progress = None;
                } else {
                    self.agent_turn_active = false;
                    self.refresh_bg_running();
                    self.status = "error".to_string();
                    self.input_mode = InputMode::Insert;
                    self.tool_in_progress = None;
                }
            }
            AgentEvent::TurnEnd { stop_reason: _ } => {
                self.status = "ready".to_string();
                self.unseen_done = true;
                self.notify_done("turn end");
            }
            AgentEvent::Done => {
                self.provider_auto_continues = 0;
                self.agent_turn_active = false;
                self.refresh_bg_running();
                self.status = "ready".to_string();
                self.unseen_done = true;
                self.input_mode = InputMode::Insert;
                self.input.clear();
                self.near_limit = false;
                self.queued_steers = 0;
                self.tool_in_progress = None;
                self.notify_done("ready");
                if self.show_sessions_panel {
                    self.sessions_panel
                        .refresh(&self.session_id, self.bg_running_session.as_deref());
                }
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
            AgentEvent::ToolUseDelta { id, input } => {
                // Stream the tool input (e.g. the bash command) into the live
                // chat block as it arrives from the model.
                let mut hit = None;
                for (mi, msg) in self.messages.iter_mut().enumerate().rev() {
                    if let Some(bi) = msg
                        .tool_blocks
                        .iter()
                        .rposition(|b| b.id.as_deref() == Some(id.as_str()))
                    {
                        hit = Some((mi, bi));
                        break;
                    }
                }
                if let Some((mi, bi)) = hit {
                    let block = &mut self.messages[mi].tool_blocks[bi];
                    if block.preview == "…" {
                        block.preview.clear();
                    }
                    let room = 160usize.saturating_sub(block.preview.chars().count());
                    if room > 0 {
                        let add: String = input.chars().take(room).collect();
                        block.preview.push_str(&add);
                    }
                }
            }
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
            AgentEvent::ContextWarning {
                fraction: _,
                used,
                limit,
            } => {
                self.token_used = used;
                self.token_limit = limit;
                self.near_limit = true;
            }
            AgentEvent::TokenUpdate {
                used,
                limit,
                input_tokens,
                output_tokens,
                cache_read_tokens,
            } => {
                self.token_used = used;
                self.token_limit = limit;
                self.session_input_tokens = input_tokens;
                self.session_output_tokens = output_tokens;
                self.session_cache_read_tokens = cache_read_tokens;
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
                            self.push_system(format!("stream failed — auto-continuing ({n}/3)…"));
                            self.input_mode = InputMode::Waiting;
                            self.tool_in_progress = None;
                            self.status = "thinking...".to_string();
                            let _ = self.command_tx.send(AppCommand::Submit {
                                text: "continue".into(),
                            });
                        } else {
                            self.provider_auto_continues = 0;
                            self.push_system("stream failed — auto-retry exhausted; type continue");
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
                self.agent_turn_active = false;
                self.refresh_bg_running();
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
                // Rekey the runtime map when fork/load changes the session id.
                if id != self.agent_session_id {
                    if let Some(mut rt) = self.runtimes.remove(&self.agent_session_id) {
                        rt.id = id.clone();
                        self.runtimes.insert(id.clone(), rt);
                    }
                }
                self.agent_session_id = id.clone();
                self.session_id = id;
                self.session_title = title.filter(|t| !t.is_empty());
                crate::tools::turn_snapshot::set_session(&self.session_id);
                crate::agent::trajectory::set_session(&self.session_id);
                if self.show_sessions_panel {
                    self.sessions_panel
                        .refresh(&self.session_id, self.bg_running_session.as_deref());
                }
            }
            AgentEvent::ReloadTranscript { messages } => {
                let has_chat = self.messages.iter().any(|m| m.role != "system");
                // Drop stale empty reload that would wipe a painted chat.
                if messages.is_empty() && has_chat {
                    self.input_mode = InputMode::Insert;
                    self.tool_in_progress = None;
                    self.status = "ready".into();
                    return;
                }
                self.apply_transcript_messages(&messages);
                self.input_mode = InputMode::Insert;
                self.tool_in_progress = None;
                self.tool_output_tabs.clear();
                self.show_repl_panel = false;
                self.status = "ready".into();
                if self.show_sessions_panel {
                    self.sessions_panel
                        .refresh(&self.session_id, self.bg_running_session.as_deref());
                }
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

    pub(super) fn cycle_tool_output_tab(&mut self, delta: i32) {
        if self.tool_output_tabs.is_empty() {
            self.show_repl_panel = true;
            self.status = "no tool outputs yet".into();
            return;
        }
        self.show_repl_panel = true;
        let n = self.tool_output_tabs.len() as i32;
        let cur = self.tool_output_tab as i32;
        self.tool_output_tab = ((cur + delta).rem_euclid(n)) as usize;
        self.console_scroll = 0;
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
            Self::trim_panel_utf8(&mut self.tool_output_tabs[idx].buffer, 64_000);
            self.tool_output_tab = idx;
            self.show_repl_panel = true;
        } else {
            // Fallback single buffer if a tab was not opened.
            self.repl_panel.push_str(chunk);
            Self::trim_panel_utf8(&mut self.repl_panel, 64_000);
            self.show_repl_panel = true;
        }
    }

    pub(super) fn snapshot_focused_chat(&mut self) {
        let id = self.session_id.clone();
        if let Some(rt) = self.runtimes.get_mut(&id) {
            rt.has_ui_snapshot = true;
        }
        self.ui_snapshots.insert(id, self.messages.clone());
    }

    pub(super) fn append_text_delta_to_snapshot(messages: &mut Vec<ChatMessage>, text: &str) {
        if messages.last().map(|m| m.role.as_str()) != Some("assistant") {
            messages.push(ChatMessage {
                role: "assistant".into(),
                text: String::new(),
                thinking: None,
                show_thinking: false,
                tool_blocks: Vec::new(),
            });
        }
        if let Some(last) = messages.last_mut() {
            last.text.push_str(text);
        }
    }

    pub(super) fn append_thinking_delta_to_snapshot(
        messages: &mut Vec<ChatMessage>,
        thinking: &str,
    ) {
        if messages.last().map(|m| m.role.as_str()) != Some("assistant") {
            messages.push(ChatMessage {
                role: "assistant".into(),
                text: String::new(),
                thinking: None,
                show_thinking: true,
                tool_blocks: Vec::new(),
            });
        }
        if let Some(last) = messages.last_mut() {
            let was_empty = last.thinking.as_ref().map(|t| t.is_empty()).unwrap_or(true);
            let prev = last.thinking.take().unwrap_or_default();
            last.thinking = Some(prev + thinking);
            if was_empty {
                last.show_thinking = true;
            }
        }
    }
}
