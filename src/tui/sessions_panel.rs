//! Project session switcher — herdr-style multi-session side panel.
//!
//! Lists sessions for the current project (or all), highlights the active
//! one, and supports Enter to switch / `n` to start a new chat in-project.

use std::path::Path;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::session::{SessionStore, SessionSummary};

use super::theme::Palette;
use super::widgets;

#[derive(Debug, Clone)]
pub enum SessionRow {
    Header { title: String },
    Session { summary: SessionSummary },
    Action { id: &'static str, label: String },
    Hint { text: String },
}

impl SessionRow {
    pub fn selectable(&self) -> bool {
        matches!(self, Self::Session { .. } | Self::Action { .. })
    }
}

#[derive(Debug, Clone)]
pub struct SessionsPanelState {
    pub rows: Vec<SessionRow>,
    pub selection: usize,
    /// When false, only sessions tagged with the current project.
    pub show_all: bool,
    pub project_root: Option<String>,
    pub active_id: String,
    /// Session id with a turn still running while UI is elsewhere.
    pub bg_running_id: Option<String>,
}

impl Default for SessionsPanelState {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selection: 0,
            show_all: false,
            project_root: SessionStore::current_project_root(),
            active_id: String::new(),
            bg_running_id: None,
        }
    }
}

impl SessionsPanelState {
    pub fn refresh(&mut self, active_id: &str, bg_running: Option<&str>) {
        self.active_id = active_id.to_string();
        self.bg_running_id = bg_running.map(|s| s.to_string());
        self.project_root = SessionStore::current_project_root();
        let store = SessionStore::new();
        let bg = self.bg_running_id.clone();

        let mut rows: Vec<SessionRow> = Vec::new();
        let project_label = self
            .project_root
            .as_deref()
            .map(path_leaf)
            .unwrap_or(".");

        let scope = if self.show_all {
            "ALL PROJECTS"
        } else {
            "THIS PROJECT"
        };
        rows.push(SessionRow::Header {
            title: format!("sessions · {scope}"),
        });
        rows.push(SessionRow::Hint {
            text: format!("cwd {project_label}"),
        });
        if let Some(ref bg_id) = bg {
            rows.push(SessionRow::Hint {
                text: format!("◐ bg {}", SessionStore::short_id(bg_id)),
            });
        }

        let mut list = if self.show_all {
            store.list_summaries().unwrap_or_default()
        } else if let Some(ref root) = self.project_root {
            store
                .list_summaries_for_project(root, false)
                .unwrap_or_default()
        } else {
            store.list_summaries().unwrap_or_default()
        };

        // Always surface the active session even if it falls outside the filter.
        if !list.iter().any(|s| s.id == active_id) {
            if let Ok(data) = store.load(active_id) {
                list.insert(
                    0,
                    SessionSummary {
                        id: data.id,
                        title: data.title,
                        model: data.model,
                        updated_at: data.updated_at,
                        message_count: data.messages.len(),
                        parent_id: data.parent_id,
                        branch_label: data.branch_label,
                        project_root: data.project_root,
                    },
                );
            }
        }

        if list.is_empty() {
            rows.push(SessionRow::Hint {
                text: "no sessions yet — press n".into(),
            });
        } else {
            rows.push(SessionRow::Header {
                title: format!("chats ({})", list.len()),
            });
            for s in list {
                rows.push(SessionRow::Session { summary: s });
            }
        }

        rows.push(SessionRow::Header {
            title: "actions".into(),
        });
        rows.push(SessionRow::Action {
            id: "new",
            label: "＋ new session in project".into(),
        });
        rows.push(SessionRow::Action {
            id: "toggle_scope",
            label: if self.show_all {
                "⊙ show this project only".into()
            } else {
                "◎ show all projects".into()
            },
        });
        rows.push(SessionRow::Hint {
            text: "↑↓ · Enter switch · n new · a all".into(),
        });

        self.rows = rows;
        self.clamp_selection();
        if let Some(idx) = self
            .rows
            .iter()
            .position(|r| matches!(r, SessionRow::Session { summary } if summary.id == active_id))
        {
            self.selection = idx;
        }
    }

    fn clamp_selection(&mut self) {
        if self.rows.is_empty() {
            self.selection = 0;
            return;
        }
        if self.selection >= self.rows.len() {
            self.selection = self.rows.len() - 1;
        }
        if !self.rows[self.selection].selectable() {
            self.move_sel(1);
            if !self.rows.get(self.selection).is_some_and(|r| r.selectable()) {
                self.move_sel(-1);
            }
        }
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as i32;
        let mut idx = self.selection as i32;
        for _ in 0..n {
            idx = (idx + delta).rem_euclid(n);
            if self.rows[idx as usize].selectable() {
                self.selection = idx as usize;
                return;
            }
        }
    }

    pub fn selected_row(&self) -> Option<&SessionRow> {
        self.rows.get(self.selection)
    }
}

fn path_leaf(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
}

pub fn render_sessions_panel(
    frame: &mut Frame,
    area: Rect,
    state: &SessionsPanelState,
    palette: &Palette,
) {
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in state.rows.iter().enumerate() {
        let selected = i == state.selection;
        match row {
            SessionRow::Header { title } => {
                lines.push(Line::from(Span::styled(
                    format!(" {}", title.to_uppercase()),
                    Style::default()
                        .fg(palette.overlay0)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            SessionRow::Hint { text } => {
                lines.push(Line::from(Span::styled(
                    format!(" {text}"),
                    Style::default().fg(palette.overlay0),
                )));
            }
            SessionRow::Action { label, .. } => {
                let prefix = if selected { "›" } else { " " };
                let style = if selected {
                    Style::default()
                        .fg(palette.text)
                        .bg(palette.surface1)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.accent)
                };
                lines.push(Line::from(Span::styled(
                    format!("{prefix}{label}"),
                    style,
                )));
            }
            SessionRow::Session { summary } => {
                let active = summary.id == state.active_id;
                let bg = state
                    .bg_running_id
                    .as_ref()
                    .is_some_and(|b| b == &summary.id);
                let prefix = if selected {
                    "›"
                } else if active {
                    "●"
                } else if bg {
                    "◐"
                } else {
                    " "
                };
                let title = summary.title.as_deref().unwrap_or("(untitled)");
                let short = SessionStore::short_id(&summary.id);
                let title_trim: String = title.chars().take(28).collect();
                let fork = summary
                    .branch_label
                    .as_ref()
                    .map(|l| format!(" [{l}]"))
                    .unwrap_or_default();
                let main = format!("{prefix}{short} · {title_trim}{fork}");
                let meta = format!("  {} msgs · {}", summary.message_count, summary.model);
                let fg = if active {
                    palette.ok
                } else if bg {
                    palette.state_working
                } else if selected {
                    palette.text
                } else {
                    palette.subtext
                };
                let style = if selected {
                    Style::default().fg(fg).bg(palette.surface1)
                } else {
                    Style::default().fg(fg)
                };
                lines.push(Line::from(Span::styled(main, style)));
                if selected || active {
                    lines.push(Line::from(Span::styled(
                        meta,
                        Style::default().fg(palette.overlay0),
                    )));
                }
            }
        }
    }

    let panel = Paragraph::new(lines).block(widgets::panel_block(
        "Sessions · switch / new",
        palette,
        true,
    ));
    frame.render_widget(panel, area);
}
