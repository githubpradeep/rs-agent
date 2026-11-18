//! Permission cards and `question` prompts — rendering, key handling,
//! and the auto-allow policy.
//!
use super::*;

impl App {
    pub(super) fn auto_allow(&self, tool_name: &str) -> bool {
        if !self.auto_mode {
            return false;
        }
        // AUTO: file tools + read-only builtins. Still prompt for bash/repl and
        // mutating MCP tools (those keep requires_permission=true).
        matches!(
            tool_name,
            "read" | "grep" | "ls" | "find" | "webfetch" | "websearch" | "write" | "edit"
        )
    }

    pub(super) fn handle_permission_key(&mut self, key: crossterm::event::KeyEvent) {
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
            lines.push(format!(
                "… ({} more line{})",
                dropped,
                if dropped == 1 { "" } else { "s" }
            ));
        }
        lines
    }

    pub(super) fn render_permission_prompt(&mut self, frame: &mut Frame, area: Rect) {
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
        widgets::render_modal_shell(frame, prompt_area, title, border_color, &self.palette, text);
    }

    pub(super) fn handle_question_key(&mut self, key: crossterm::event::KeyEvent) {
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

    pub(super) fn render_question_prompt(&mut self, frame: &mut Frame, area: Rect) {
        let pending = match self.pending_question.as_ref() {
            Some(p) => p,
            None => return,
        };

        let mut text = vec![Line::from(Span::styled(
            format!(" {}", pending.request.question),
            Style::default()
                .fg(self.palette.text)
                .add_modifier(Modifier::BOLD),
        ))];
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
                    Span::styled(format!(" {opt}"), Style::default().fg(self.palette.subtext)),
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
}
