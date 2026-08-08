//! City cockpit side panel — operator board + seat/bead detail.
//!
//! Board: ACTIONS / WORKERS / WISHES / READY
//! Detail: seat status + action strip + dedicated log viewport

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::agent::SeatCaste;
use crate::beads::{self, Bead, BeadKind, BeadStatus};
use crate::fleet::{self, ParsedLogLine, SeatStatus};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeadDetailKind {
    Wish,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CityView {
    Board,
    SeatDetail { seat: String },
    BeadDetail { id: String, kind: BeadDetailKind },
}

impl Default for CityView {
    fn default() -> Self {
        Self::Board
    }
}

#[derive(Debug, Clone)]
pub enum CityRow {
    Header { title: String },
    Action { id: &'static str, label: String },
    Worker { seat: SeatStatus },
    Wish { bead: Bead },
    Ready { bead: Bead },
    Hint { text: String },
}

impl CityRow {
    pub fn selectable(&self) -> bool {
        matches!(
            self,
            Self::Worker { .. } | Self::Wish { .. } | Self::Ready { .. } | Self::Action { .. }
        )
    }
}

/// Back-compat name used by the TUI app.
pub type FleetPanelState = CityPanelState;

#[derive(Debug, Clone)]
pub struct CityPanelState {
    pub rows: Vec<CityRow>,
    pub selection: usize,
    pub expanded: bool,
    pub seats: Vec<SeatStatus>,
    pub view: CityView,
    /// Dedicated seat log (Follow / detail) — not dumped into operator chat.
    pub log_lines: Vec<String>,
    /// Lines scrolled up from bottom (0 = follow tail).
    pub log_scroll: usize,
    pub detail_seat: Option<SeatStatus>,
    pub detail_bead: Option<Bead>,
}

impl Default for CityPanelState {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selection: 0,
            expanded: false,
            seats: Vec::new(),
            view: CityView::Board,
            log_lines: Vec::new(),
            log_scroll: 0,
            detail_seat: None,
            detail_bead: None,
        }
    }
}

fn is_wish_bead(b: &Bead) -> bool {
    b.notes.contains("label:wish")
        || b.title.to_lowercase().starts_with("wish:")
        || b.title.to_lowercase().starts_with("wish ")
}

pub fn caste_badge(seat: &str) -> &'static str {
    match crate::agent::seat::resolve_caste(seat) {
        SeatCaste::Crew => "C",
        SeatCaste::Fleet => "F",
        SeatCaste::Review => "R",
        SeatCaste::Marshal => "M",
        SeatCaste::Seneschal => "S",
        SeatCaste::Role => "O",
        SeatCaste::Any => "·",
    }
}

fn format_parsed_log(line: &ParsedLogLine) -> String {
    let kind = match line.kind {
        fleet::LogKind::Tool => "tool",
        fleet::LogKind::ToolResult => "result",
        fleet::LogKind::Say => "say",
        fleet::LogKind::Heartbeat => "hb",
        fleet::LogKind::Claimed => "claim",
        fleet::LogKind::Closed => "close",
        fleet::LogKind::Session => "sess",
        fleet::LogKind::Error => "err",
        fleet::LogKind::Status => "stat",
        fleet::LogKind::Raw => "log",
    };
    let ts = line.timestamp.as_deref().unwrap_or("");
    if ts.is_empty() {
        format!("[{kind}] {}", line.body)
    } else {
        format!("{ts} [{kind}] {}", line.body)
    }
}

impl CityPanelState {
    pub fn refresh(&mut self) {
        match self.view.clone() {
            CityView::Board => self.refresh_board(),
            CityView::SeatDetail { seat } => self.refresh_seat_detail(&seat),
            CityView::BeadDetail { id, kind } => self.refresh_bead_detail(&id, kind),
        }
    }

    fn refresh_board(&mut self) {
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
            title: "ACTIONS".into(),
        });
        rows.push(CityRow::Action {
            id: "wish",
            label: "＋ new wish".into(),
        });
        rows.push(CityRow::Action {
            id: "spawn",
            label: "⬆ spawn fleet / crew".into(),
        });
        rows.push(CityRow::Action {
            id: "marshal",
            label: "◎ marshal once".into(),
        });
        rows.push(CityRow::Action {
            id: "down_all",
            label: "⏹ stop all workers".into(),
        });

        rows.push(CityRow::Header {
            title: format!("WORKERS ({})", self.seats.len()),
        });
        if self.seats.is_empty() {
            rows.push(CityRow::Hint {
                text: "none — Enter spawn or press u".into(),
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
                text: "none — Enter new wish or press w".into(),
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

        rows.push(CityRow::Hint {
            text: "c city · w wish · u spawn · d stop · A assign".into(),
        });

        self.rows = rows;
        self.selection = self
            .rows
            .iter()
            .position(|r| r.selectable() && self.row_key(r).as_deref() == prev_key.as_deref())
            .or_else(|| self.rows.iter().position(|r| r.selectable()))
            .unwrap_or(0);
    }

    fn refresh_seat_detail(&mut self, seat: &str) {
        self.seats = fleet::list_seat_statuses();
        self.detail_seat = fleet::read_seat_status(seat).or_else(|| {
            self.seats
                .iter()
                .find(|s| s.seat == seat)
                .cloned()
                .or_else(|| {
                    Some(SeatStatus {
                        seat: seat.to_string(),
                        state: "unknown".into(),
                        running: false,
                        ..SeatStatus::default()
                    })
                })
        });
        self.detail_bead = None;
        self.rows.clear();
    }

    fn refresh_bead_detail(&mut self, id: &str, kind: BeadDetailKind) {
        self.detail_bead = beads::get(None, id).ok().flatten().or_else(|| {
            // Fallback: search open/ready lists
            let open = beads::list_open(None).unwrap_or_default();
            open.into_iter().find(|b| b.id == id).or_else(|| {
                beads::list_ready(None)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|b| b.id == id)
            })
        });
        let _ = kind;
        self.detail_seat = None;
        self.rows.clear();
    }

    pub fn open_seat_detail(&mut self, seat: &str) {
        self.view = CityView::SeatDetail {
            seat: seat.to_string(),
        };
        self.log_scroll = 0;
        self.refresh_seat_detail(seat);
    }

    pub fn open_bead_detail(&mut self, bead: &Bead, kind: BeadDetailKind) {
        self.view = CityView::BeadDetail {
            id: bead.id.clone(),
            kind,
        };
        self.detail_bead = Some(bead.clone());
        self.detail_seat = None;
        self.log_lines.clear();
        self.log_scroll = 0;
    }

    pub fn back_to_board(&mut self) {
        self.view = CityView::Board;
        self.detail_seat = None;
        self.detail_bead = None;
        self.log_lines.clear();
        self.log_scroll = 0;
        self.refresh_board();
    }

    pub fn is_board(&self) -> bool {
        matches!(self.view, CityView::Board)
    }

    pub fn seat_detail_name(&self) -> Option<&str> {
        match &self.view {
            CityView::SeatDetail { seat } => Some(seat.as_str()),
            _ => None,
        }
    }

    pub fn is_seat_detail(&self, seat: &str) -> bool {
        matches!(&self.view, CityView::SeatDetail { seat: s } if s == seat)
    }

    pub fn set_log_from_parsed(&mut self, lines: &[ParsedLogLine]) {
        self.log_lines = lines.iter().map(format_parsed_log).collect();
        self.log_scroll = 0;
    }

    pub fn push_log_line(&mut self, line: &ParsedLogLine) {
        self.log_lines.push(format_parsed_log(line));
        const MAX: usize = 400;
        if self.log_lines.len() > MAX {
            let drain = self.log_lines.len() - MAX;
            self.log_lines.drain(0..drain);
        }
        if self.log_scroll == 0 {
            // stay pinned to bottom
        } else {
            // keep relative position when scrolled up
        }
    }

    pub fn log_scroll_by(&mut self, delta: isize) {
        let max = self.log_lines.len().saturating_sub(1);
        if delta < 0 {
            self.log_scroll = (self.log_scroll as isize - delta)
                .min(max as isize)
                .max(0) as usize;
        } else {
            self.log_scroll = self.log_scroll.saturating_sub(delta as usize);
        }
    }

    fn row_key(&self, row: &CityRow) -> Option<String> {
        match row {
            CityRow::Worker { seat } => Some(format!("w:{}", seat.seat)),
            CityRow::Wish { bead } => Some(format!("wish:{}", bead.id)),
            CityRow::Ready { bead } => Some(format!("ready:{}", bead.id)),
            CityRow::Action { id, .. } => Some(format!("a:{id}")),
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
        if !matches!(self.view, CityView::Board) {
            return;
        }
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
    match &state.view {
        CityView::Board => render_board(frame, area, state, palette),
        CityView::SeatDetail { seat } => render_seat_detail(frame, area, state, seat, palette),
        CityView::BeadDetail { id, kind } => {
            render_bead_detail(frame, area, state, id, *kind, palette)
        }
    }
}

fn render_board(frame: &mut Frame, area: Rect, state: &CityPanelState, palette: &Palette) {
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
        "↑↓ · Enter · w/u/d · Esc",
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
            CityRow::Action { label, .. } => {
                let marker = if sel { "›" } else { " " };
                let row_s = status::ellipsize(&format!("{marker}{label}"), max_w);
                let style = if sel {
                    Style::default()
                        .fg(palette.highlight_fg)
                        .bg(palette.highlight_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.subtext)
                };
                lines.push(Line::from(Span::styled(row_s, style)));
            }
            CityRow::Worker { seat } => {
                let att = SeatAttention::from_status(seat);
                let marker = if sel { "›" } else { " " };
                let badge = caste_badge(&seat.seat);
                let bead = seat
                    .last_bead
                    .as_deref()
                    .map(|b| format!(" · {b}"))
                    .unwrap_or_default();
                let row_s = status::ellipsize(
                    &format!("{marker}{} [{badge}] {}{bead}", att.icon(), seat.seat),
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
                let st = bead_status_short(bead.status);
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
                let kind = bead_kind_short(bead.kind);
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
        "City · wish / spawn / watch",
        palette,
        true,
    ));
    frame.render_widget(panel, area);
}

fn bead_status_short(st: BeadStatus) -> &'static str {
    match st {
        BeadStatus::Open => "open",
        BeadStatus::Claimed => "claim",
        BeadStatus::Blocked => "block",
        BeadStatus::Gated => "gate",
        BeadStatus::Closed => "done",
    }
}

fn bead_kind_short(kind: BeadKind) -> &'static str {
    match kind {
        BeadKind::Design => "des",
        BeadKind::Implement => "imp",
        BeadKind::Review => "rev",
        BeadKind::Task => "tsk",
    }
}

fn render_seat_detail(
    frame: &mut Frame,
    area: Rect,
    state: &CityPanelState,
    seat: &str,
    palette: &Palette,
) {
    let title = format!("City · {seat}");
    let block = widgets::panel_block(&title, palette, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(4)])
        .split(inner);

    let st = state.detail_seat.as_ref();
    let att = st
        .map(SeatAttention::from_status)
        .unwrap_or(SeatAttention::Unknown);
    let badge = caste_badge(seat);
    let mut top: Vec<Line> = Vec::new();
    top.push(Line::from(vec![
        Span::styled(
            format!(" {} ", att.icon()),
            Style::default().fg(att.color(palette)),
        ),
        Span::styled(
            format!("{seat} [{badge}]"),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    if let Some(s) = st {
        top.push(Line::from(Span::styled(
            format!(
                "state={} pid={} {}",
                s.state,
                s.pid,
                if s.running { "running" } else { "stopped" }
            ),
            Style::default().fg(palette.subtext),
        )));
        if let Some(b) = s.last_bead.as_deref() {
            top.push(Line::from(Span::styled(
                format!(
                    "bead {b}{}",
                    s.last_title
                        .as_deref()
                        .map(|t| format!(" — {t}"))
                        .unwrap_or_default()
                ),
                Style::default().fg(palette.overlay1),
            )));
        }
        if let Some(tool) = s.last_tool.as_deref() {
            top.push(Line::from(Span::styled(
                format!("tool {tool}"),
                Style::default().fg(palette.tool),
            )));
        }
        if s.awaiting_human.unwrap_or(false) {
            top.push(Line::from(Span::styled(
                "awaiting human",
                Style::default().fg(palette.state_blocked),
            )));
        }
        if let Some(line) = s.last_line.as_deref() {
            top.push(Line::from(Span::styled(
                status::ellipsize(line, (chunks[0].width as usize).saturating_sub(2).max(8)),
                Style::default().fg(palette.muted),
            )));
        }
    } else {
        top.push(Line::from(Span::styled(
            "no status file yet",
            Style::default().fg(palette.muted),
        )));
    }
    top.push(Line::from(Span::styled(
        "[f]ollow [a]ttach [o]pen [s]teer [b]abort",
        Style::default().fg(palette.overlay0),
    )));
    top.push(Line::from(Span::styled(
        "[D]etach [d]own  Esc back",
        Style::default().fg(palette.overlay0),
    )));

    frame.render_widget(Paragraph::new(top), chunks[0]);

    let log_h = chunks[1].height as usize;
    let visible = log_h.saturating_sub(1).max(1);
    let end = state
        .log_lines
        .len()
        .saturating_sub(state.log_scroll);
    let start = end.saturating_sub(visible);
    let slice = if start < end {
        &state.log_lines[start..end]
    } else {
        &[][..]
    };
    let mut log_lines: Vec<Line> = vec![Line::from(Span::styled(
        format!(
            "─ log {} ─ PgUp/Dn",
            if state.log_scroll == 0 {
                "live".into()
            } else {
                format!("↑{}", state.log_scroll)
            }
        ),
        Style::default().fg(palette.border),
    ))];
    for l in slice {
        log_lines.push(Line::from(Span::styled(
            status::ellipsize(l, (chunks[1].width as usize).saturating_sub(1).max(8)),
            Style::default().fg(palette.overlay1),
        )));
    }
    if slice.is_empty() {
        log_lines.push(Line::from(Span::styled(
            "(no log yet — press f to follow)",
            Style::default().fg(palette.muted),
        )));
    }
    frame.render_widget(Paragraph::new(log_lines), chunks[1]);
}

fn render_bead_detail(
    frame: &mut Frame,
    area: Rect,
    state: &CityPanelState,
    id: &str,
    kind: BeadDetailKind,
    palette: &Palette,
) {
    let title = match kind {
        BeadDetailKind::Wish => format!("City · wish {id}"),
        BeadDetailKind::Ready => format!("City · ready {id}"),
    };
    let mut lines: Vec<Line> = Vec::new();
    if let Some(b) = &state.detail_bead {
        lines.push(Line::from(Span::styled(
            b.title.clone(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "{} · {} · prio {}",
                b.kind.as_str(),
                b.status.as_str(),
                b.priority
            ),
            Style::default().fg(palette.subtext),
        )));
        if !b.deps.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("deps: {}", b.deps.join(", ")),
                Style::default().fg(palette.overlay0),
            )));
        }
        lines.push(Line::from(""));
        for chunk in b.notes.lines().take(12) {
            lines.push(Line::from(Span::styled(
                chunk.to_string(),
                Style::default().fg(palette.overlay1),
            )));
        }
        lines.push(Line::from(""));
        if kind == BeadDetailKind::Ready {
            lines.push(Line::from(Span::styled(
                "A assign to seat · Esc back",
                Style::default().fg(palette.overlay0),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "Esc back to board",
                Style::default().fg(palette.overlay0),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!("bead {id} not found"),
            Style::default().fg(palette.state_blocked),
        )));
        lines.push(Line::from(Span::styled(
            "Esc back",
            Style::default().fg(palette.overlay0),
        )));
    }

    let panel = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(widgets::panel_block(&title, palette, true));
    frame.render_widget(panel, area);
}
