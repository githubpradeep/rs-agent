//! City cockpit side panel — herdr agent switcher + Conductor board.
//!
//! One place for a PM/operator to see:
//! - Workers (fleet seats) — attention-sorted, Enter to follow
//! - Wishes — open wish-labeled beads
//! - Ready work — claimable implement/task beads

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::beads::{self, Bead, BeadKind, BeadStatus};
use crate::fleet::{self, SeatStatus};
use crate::lifecycle::Lifecycle;

use super::status;
use super::theme::Palette;
use super::widgets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatAttention {
    Blocked,
    Done,
    Working,
    Idle,
    Unknown,
}

impl SeatAttention {
    pub fn from_status(s: &SeatStatus) -> Self {
        let st = s.state.to_lowercase();
        let life = s
            .lifecycle
            .as_deref()
            .map(Lifecycle::parse)
            .unwrap_or(Lifecycle::Unknown);
        if st.contains("error")
            || st.contains("stuck")
            || st.contains("paused")
            || life == Lifecycle::Blocked
            || s.awaiting_human.unwrap_or(false)
        {
            return Self::Blocked;
        }
        if st.contains("done") || life == Lifecycle::Done {
            return Self::Done;
        }
        if st.contains("running")
            || st.contains("working")
            || st.contains("tool")
            || life == Lifecycle::Working
            || s.last_bead.is_some()
        {
            return Self::Working;
        }
        if st.contains("idle") || life == Lifecycle::Idle {
            return Self::Idle;
        }
        Self::Unknown
    }

    pub fn priority(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Done => 1,
            Self::Working => 2,
            Self::Idle => 3,
            Self::Unknown => 4,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Blocked => "●",
            Self::Done => "✓",
            Self::Working => "◐",
            Self::Idle => "○",
            Self::Unknown => "·",
        }
    }

    pub fn color(self, p: &Palette) -> ratatui::style::Color {
        match self {
            Self::Blocked => p.state_blocked,
            Self::Done => p.state_done,
            Self::Working => p.state_working,
            Self::Idle => p.state_idle,
            Self::Unknown => p.overlay0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CityRow {
    Header {
        title: String,
    },
    Worker {
        seat: SeatStatus,
    },
    Wish {
        bead: Bead,
    },
    Ready {
        bead: Bead,
    },
    Hint {
        text: String,
    },
}

impl CityRow {
    pub fn selectable(&self) -> bool {
        matches!(self, Self::Worker { .. } | Self::Wish { .. } | Self::Ready { .. })
    }
}

/// Back-compat name used by the TUI app.
pub type FleetPanelState = CityPanelState;

#[derive(Debug, Clone)]
pub struct CityPanelState {
    pub rows: Vec<CityRow>,
    /// Index into `rows` (always a selectable row when possible).
    pub selection: usize,
    pub expanded: bool,
    /// Cached seats for blocked_count / selected seat helpers.
    pub seats: Vec<SeatStatus>,
}

impl Default for CityPanelState {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selection: 0,
            expanded: false,
            seats: Vec::new(),
        }
    }
}

fn is_wish_bead(b: &Bead) -> bool {
    b.notes.contains("label:wish")
        || b.title.to_lowercase().starts_with("wish:")
        || b.title.to_lowercase().starts_with("wish ")
}

impl CityPanelState {
    pub fn refresh(&mut self) {
        let prev_key = self.selected_key();

        let mut seats = fleet::list_seat_statuses();
        seats.sort_by_key(|s| SeatAttention::from_status(s).priority());
        self.seats = seats;

        let open = beads::list_open(None).unwrap_or_default();
        let mut wishes: Vec<Bead> = open.into_iter().filter(is_wish_bead).collect();
        wishes.sort_by_key(|b| b.priority);
        wishes.truncate(8);

        let mut ready = beads::list_ready(None).unwrap_or_default();
        ready.sort_by_key(|b| b.priority);
        ready.truncate(10);

        let mut rows = Vec::new();

        rows.push(CityRow::Header {
            title: format!("WORKERS ({})", self.seats.len()),
        });
        if self.seats.is_empty() {
            rows.push(CityRow::Hint {
                text: "no seats — /fleet up".into(),
            });
        } else {
            for seat in &self.seats {
                rows.push(CityRow::Worker { seat: seat.clone() });
            }
        }

        rows.push(CityRow::Header {
            title: format!("WISHES ({})", wishes.len()),
        });
        if wishes.is_empty() {
            rows.push(CityRow::Hint {
                text: "none — /wish <text>".into(),
            });
        } else {
            for bead in wishes {
                rows.push(CityRow::Wish { bead });
            }
        }

        rows.push(CityRow::Header {
            title: format!("READY ({})", ready.len()),
        });
        if ready.is_empty() {
            rows.push(CityRow::Hint {
                text: "queue empty".into(),
            });
        } else {
            for bead in ready {
                rows.push(CityRow::Ready { bead });
            }
        }

        self.rows = rows;
        self.selection = self
            .rows
            .iter()
            .position(|r| r.selectable() && self.row_key(r).as_deref() == prev_key.as_deref())
            .or_else(|| self.rows.iter().position(|r| r.selectable()))
            .unwrap_or(0);
    }

    fn row_key(&self, row: &CityRow) -> Option<String> {
        match row {
            CityRow::Worker { seat } => Some(format!("w:{}", seat.seat)),
            CityRow::Wish { bead } => Some(format!("wish:{}", bead.id)),
            CityRow::Ready { bead } => Some(format!("ready:{}", bead.id)),
            _ => None,
        }
    }

    fn selected_key(&self) -> Option<String> {
        self.rows.get(self.selection).and_then(|r| self.row_key(r))
    }

    pub fn blocked_count(&self) -> usize {
        self.seats
            .iter()
            .filter(|s| SeatAttention::from_status(s) == SeatAttention::Blocked)
            .count()
    }

    pub fn selected(&self) -> Option<&SeatStatus> {
        match self.rows.get(self.selection) {
            Some(CityRow::Worker { seat }) => Some(seat),
            _ => None,
        }
    }

    pub fn selected_row(&self) -> Option<&CityRow> {
        self.rows.get(self.selection)
    }

    pub fn move_sel(&mut self, delta: isize) {
        let selectable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .map(|(i, _)| i)
            .collect();
        if selectable.is_empty() {
            return;
        }
        let cur_pos = selectable
            .iter()
            .position(|&i| i == self.selection)
            .unwrap_or(0) as isize;
        let n = selectable.len() as isize;
        let next = selectable[((cur_pos + delta).rem_euclid(n)) as usize];
        self.selection = next;
    }
}

pub fn render_fleet_panel(frame: &mut Frame, area: Rect, state: &CityPanelState, palette: &Palette) {
    render_city_panel(frame, area, state, palette);
}

pub fn render_city_panel(frame: &mut Frame, area: Rect, state: &CityPanelState, palette: &Palette) {
    let max_w = (area.width as usize).saturating_sub(4).max(8);
    let mut lines: Vec<Line> = Vec::new();
    let blocked = state.blocked_count();
    let wish_n = state
        .rows
        .iter()
        .filter(|r| matches!(r, CityRow::Wish { .. }))
        .count();
    let ready_n = state
        .rows
        .iter()
        .filter(|r| matches!(r, CityRow::Ready { .. }))
        .count();

    lines.push(Line::from(vec![
        Span::styled(
            " city ",
            Style::default()
                .fg(palette.contrast_on_accent())
                .bg(if blocked > 0 {
                    palette.state_blocked
                } else {
                    palette.accent
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {}w · {}wish · {}ready{} ",
                state.seats.len(),
                wish_n,
                ready_n,
                if blocked > 0 {
                    format!(" · {blocked}⚠")
                } else {
                    String::new()
                }
            ),
            Style::default().fg(palette.subtext),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "↑↓ · Enter act · x detail",
        Style::default().fg(palette.overlay0),
    )));

    for (i, row) in state.rows.iter().enumerate() {
        let sel = i == state.selection && row.selectable();
        match row {
            CityRow::Header { title } => {
                lines.push(Line::from(Span::styled(
                    "─".repeat(max_w.min(36)),
                    Style::default().fg(palette.border),
                )));
                lines.push(Line::from(Span::styled(
                    title.clone(),
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            CityRow::Hint { text } => {
                lines.push(Line::from(Span::styled(
                    format!("  {text}"),
                    Style::default().fg(palette.muted),
                )));
            }
            CityRow::Worker { seat } => {
                let att = SeatAttention::from_status(seat);
                let marker = if sel { "›" } else { " " };
                let bead = seat
                    .last_bead
                    .as_deref()
                    .map(|b| format!(" · {b}"))
                    .unwrap_or_default();
                let row_s = status::ellipsize(
                    &format!("{marker}{} {}{bead}", att.icon(), seat.seat),
                    max_w,
                );
                let style = if sel {
                    Style::default()
                        .fg(palette.highlight_fg)
                        .bg(palette.highlight_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(att.color(palette))
                };
                lines.push(Line::from(Span::styled(row_s, style)));
                if sel && state.expanded {
                    if let Some(line) = seat.last_line.as_deref() {
                        lines.push(Line::from(Span::styled(
                            status::ellipsize(&format!("   {line}"), max_w),
                            Style::default().fg(palette.overlay0),
                        )));
                    }
                }
            }
            CityRow::Wish { bead } => {
                let marker = if sel { "›" } else { " " };
                let st = match bead.status {
                    BeadStatus::Open => "open",
                    BeadStatus::Claimed => "claim",
                    BeadStatus::Blocked => "block",
                    BeadStatus::Gated => "gate",
                    BeadStatus::Closed => "done",
                };
                let row_s = status::ellipsize(
                    &format!("{marker}◇ {} [{st}] {}", bead.id, bead.title),
                    max_w,
                );
                let style = if sel {
                    Style::default()
                        .fg(palette.highlight_fg)
                        .bg(palette.highlight_bg)
                } else {
                    Style::default().fg(palette.warn)
                };
                lines.push(Line::from(Span::styled(row_s, style)));
            }
            CityRow::Ready { bead } => {
                let marker = if sel { "›" } else { " " };
                let kind = match bead.kind {
                    BeadKind::Design => "des",
                    BeadKind::Implement => "imp",
                    BeadKind::Review => "rev",
                    BeadKind::Task => "tsk",
                };
                let row_s = status::ellipsize(
                    &format!("{marker}▸ {} [{kind}] {}", bead.id, bead.title),
                    max_w,
                );
                let style = if sel {
                    Style::default()
                        .fg(palette.highlight_fg)
                        .bg(palette.highlight_bg)
                } else {
                    Style::default().fg(palette.ok)
                };
                lines.push(Line::from(Span::styled(row_s, style)));
            }
        }
    }

    let panel = Paragraph::new(lines).block(widgets::panel_block(
        "City · workers / wishes / ready",
        palette,
        true,
    ));
    frame.render_widget(panel, area);
}
