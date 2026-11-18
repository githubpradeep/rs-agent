//! Frame rendering — chat column, header/footer, side panels, and the
//! input box.
//!
use super::*;

impl App {
    pub(super) fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.hit_map.clear();
        self.console_area = None;

        let show_side =
            self.show_sessions_panel || self.show_timeline_panel || self.show_tree_panel;
        let show_repl = self.show_repl_panel || !self.tool_output_tabs.is_empty();
        let view = compute_view(
            area,
            LayoutOpts {
                show_repl,
                show_side,
                side_pct: if self.show_sessions_panel {
                    34
                } else if self.show_timeline_panel || self.show_tree_panel {
                    32
                } else {
                    28
                },
                repl_height: if self.tool_output_tabs.len() > 1 {
                    9
                } else {
                    7
                },
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
                        Style::default()
                            .fg(self.palette.warn)
                            .bg(self.palette.surface0),
                    )),
                    rect,
                );
            }
        }
        // Turn bar when Deep Context active
        if self.tree_breadcrumb != "idle" {
            let bar = tree_view::turn_bar_line(&self.tree_breadcrumb, &self.palette);
            // Drawn into chat top via messages; keep side tree rich.
            let _ = bar;
        }
        self.render_messages(frame, view.chat);
        if let Some(side) = view.side {
            if self.show_sessions_panel {
                sessions_panel::render_sessions_panel(
                    frame,
                    side,
                    &self.sessions_panel,
                    &self.palette,
                );
                let mut y = side.y.saturating_add(1);
                for (i, row) in self.sessions_panel.rows.iter().enumerate() {
                    match row {
                        SessionRow::Header { .. } => {
                            y = y.saturating_add(1);
                        }
                        SessionRow::Hint { .. } => {
                            y = y.saturating_add(1);
                        }
                        SessionRow::Session { summary } => {
                            self.hit_map.push_line(
                                side.x,
                                y,
                                side.width,
                                HitTarget::SessionRow { index: i },
                            );
                            y = y.saturating_add(1);
                            if i == self.sessions_panel.selection
                                || summary.id == self.sessions_panel.active_id
                            {
                                y = y.saturating_add(1);
                            }
                        }
                        SessionRow::Action { .. } => {
                            self.hit_map.push_line(
                                side.x,
                                y,
                                side.width,
                                HitTarget::SessionRow { index: i },
                            );
                            y = y.saturating_add(1);
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
            let height = (self.palette_items.len() as u16)
                .clamp(3, 12)
                .saturating_add(2);
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
        let mut token_str = if self.token_limit > 0 {
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
        // Prompt-cache hit chip: cache reads bill at ~10% of input on Anthropic.
        let denom = self.session_input_tokens + self.session_cache_read_tokens;
        if self.session_cache_read_tokens > 0 && denom > 0 {
            let pct = (self.session_cache_read_tokens as f64 / denom as f64 * 100.0) as u64;
            token_str.push_str(&format!(" · cache {pct}%"));
        }
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
            hints = " Esc abort · Tab console · PgUp/PgDn console scroll · Enter steer".to_string();
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

        let line =
            status::render_footer_line(&hints, &self.status, &spinner, status_color, &self.palette);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_tree_panel(&mut self, frame: &mut Frame, area: Rect) {
        let max_w = (area.width as usize).saturating_sub(4).max(8);
        let mut lines: Vec<Line> = Vec::new();
        lines.push(tree_view::turn_bar_line(
            &self.tree_breadcrumb,
            &self.palette,
        ));
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
        let panel =
            Paragraph::new(lines).block(widgets::panel_block("Call Tree", &self.palette, true));
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
                    let marker = if i == self.timeline_selection {
                        "›"
                    } else {
                        " "
                    };
                    let line = format!("{marker} @{idx} {summary}");
                    let st = if i == self.timeline_selection {
                        sel
                    } else {
                        style
                    };
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
    pub(super) fn trim_panel_utf8(panel: &mut String, cap: usize) {
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

        self.console_area = Some(area);
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
            let mut title = format!("Console · Tab {}", parts.join(""));
            if self.console_scroll > 0 {
                title.push_str(&format!(" · ↑{} (PgDn to follow)", self.console_scroll));
            }
            title
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
        let max_off = all_lines.len().saturating_sub(visible_rows);
        let off = self.console_scroll.min(max_off);
        let start = max_off.saturating_sub(off);
        let end = (start + visible_rows).min(all_lines.len());
        let style = Style::default()
            .fg(self.palette.overlay1)
            .bg(self.palette.panel_bg);
        let lines: Vec<Line> = all_lines[start..end]
            .iter()
            .map(|l| Line::from(Span::styled(Self::clip_console_line(l, inner_w), style)))
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
                widgets::panel_block(&title, &self.palette, true).style(
                    Style::default()
                        .bg(self.palette.panel_bg)
                        .fg(self.palette.text),
                ),
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
                    let take = max_width
                        .saturating_sub(current_chars)
                        .min(span_chars.len() - i);
                    let segment: String = span_chars[i..i + take].iter().collect();
                    current_spans.push(Span::styled(segment, span.style));
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
        self.chat_rows = area.height as usize;
        let max_width = (area.width as usize).saturating_sub(3).max(20);
        let md = MarkdownStyle::from_palette(&self.palette);
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
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
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
                            Span::styled(
                                "│ ".to_string(),
                                Style::default().fg(self.palette.overlay0),
                            ),
                            Span::styled("thinking · click to hide".to_string(), think_style),
                        ]);
                        lines.extend(Self::wrap_line(&header, max_width));
                        let body_style = Style::default()
                            .fg(self.palette.overlay1)
                            .add_modifier(Modifier::ITALIC);
                        for raw_line in render_markdown(thinking, md) {
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
                            Span::styled(
                                "│ ".to_string(),
                                Style::default().fg(self.palette.overlay0),
                            ),
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
                        Span::styled(
                            format!("{icon} "),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("{} · collapse", block.name), header_style),
                    ]);
                    lines.extend(Self::wrap_line(&header, max_width));
                    for raw_line in render_markdown(&block.full, md) {
                        // Keep syntax highlight (don't wipe to muted).
                        let mut styled = vec![Span::styled("  ", Style::default())];
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
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
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
                let rendered = render_markdown(&msg.text, md);
                for (i, raw_line) in rendered.into_iter().enumerate() {
                    let mut line = raw_line;
                    let wants_rail = (i == 0 && msg.role == "assistant")
                        || msg.role == "user"
                        || msg.role == "system";
                    if wants_rail {
                        line.spans
                            .insert(0, Span::styled(format!("{rail} "), rail_style));
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
                    Span::styled("…".to_string(), Style::default().fg(self.palette.overlay0)),
                ]));
            }

            // One blank line between messages. Markdown already ends some blocks
            // with an empty line — never stack two (made the chat feel oversized).
            let last_blank = lines
                .last()
                .map(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
                .unwrap_or(false);
            if !last_blank {
                lines.push(Line::from(""));
            }
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
}
