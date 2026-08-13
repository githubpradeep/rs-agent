//! City cockpit — overview + inspector (never replaces the board).
//!
//! Layout:
//!   wish composer
//!   WORKERS + FLOW lists
//!   inspector (status + log + steer composer + actions)

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::agent::SeatCaste;
use crate::beads::{self, Bead, BeadKind, BeadStatus};
use crate::fleet::{self, ParsedLogLine, SeatStatus};
use crate::lifecycle::Lifecycle;

use super::status;
use super::theme::Palette;
use super::ui::FocusZone;
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
pub enum FlowStage {
    Wish,
    Ready,
    Doing,
    Done,
}

impl FlowStage {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Wish => "wish",
            Self::Ready => "ready",
            Self::Doing => "doing",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlowItem {
    pub id: String,
    pub title: String,
    pub stage: FlowStage,
    pub kind: BeadKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BoardSection {
    Workers,
    Flow,
}

/// Selectable board row for ↑↓ navigation.
#[derive(Debug, Clone)]
pub enum BoardRow {
    Worker { seat: SeatStatus },
    Flow { item: FlowItem },
}

impl BoardRow {
    pub fn key(&self) -> String {
        match self {
            Self::Worker { seat } => format!("w:{}", seat.seat),
            Self::Flow { item } => format!("f:{}", item.id),
        }
    }
}

/// Back-compat name used by the TUI app.
pub type FleetPanelState = CityPanelState;

#[derive(Debug, Clone)]
pub struct CityPanelState {
    pub seats: Vec<SeatStatus>,
    pub board_rows: Vec<BoardRow>,
    pub selection: usize,
    pub selected_seat: Option<String>,
    pub selected_flow_id: Option<String>,
    pub flow: Vec<FlowItem>,
    pub wish_text: String,
    pub steer_text: String,
    pub spawn_fleet_n: String,
    pub spawn_crew_n: String,
    pub spawn_focus_fleet: bool,
    pub log_lines: Vec<String>,
    pub log_scroll: usize,
    pub detail_seat: Option<SeatStatus>,
    pub status_line: String,
    pub expanded: bool,
}

impl Default for CityPanelState {
    fn default() -> Self {
        Self {
            seats: Vec::new(),
            board_rows: Vec::new(),
            selection: 0,
            selected_seat: None,
            selected_flow_id: None,
            flow: Vec::new(),
            wish_text: String::new(),
            steer_text: String::new(),
            spawn_fleet_n: "2".into(),
            spawn_crew_n: "0".into(),
            spawn_focus_fleet: true,
            log_lines: Vec::new(),
            log_scroll: 0,
            detail_seat: None,
            status_line: String::new(),
            expanded: false,
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
        let prev_key = self.board_rows.get(self.selection).map(|r| r.key());

        let mut seats = fleet::list_seat_statuses();
        seats.sort_by_key(|s| SeatAttention::from_status(s).priority());
        self.seats = seats;

        // Flow: wishes (open) → ready → doing (claimed by workers) → recent closed omitted for space
        let open = beads::list_open(None).unwrap_or_default();
        let ready = beads::list_ready(None).unwrap_or_default();
        let mut flow = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for b in open.iter().filter(|b| is_wish_bead(b)) {
            if seen.insert(b.id.clone()) {
                flow.push(FlowItem {
                    id: b.id.clone(),
                    title: b.title.clone(),
                    stage: FlowStage::Wish,
                    kind: b.kind,
                });
            }
        }
        for b in &ready {
            if seen.insert(b.id.clone()) {
                flow.push(FlowItem {
                    id: b.id.clone(),
                    title: b.title.clone(),
                    stage: FlowStage::Ready,
                    kind: b.kind,
                });
            }
        }
        for s in &self.seats {
            if let Some(bid) = s.last_bead.as_ref() {
                if seen.insert(bid.clone()) {
                    flow.push(FlowItem {
                        id: bid.clone(),
                        title: s.last_title.clone().unwrap_or_else(|| bid.clone()),
                        stage: FlowStage::Doing,
                        kind: BeadKind::Implement,
                    });
                } else if let Some(item) = flow.iter_mut().find(|f| &f.id == bid) {
                    item.stage = FlowStage::Doing;
                }
            }
        }
        flow.truncate(14);
        self.flow = flow;

        let mut rows = Vec::new();
        for seat in &self.seats {
            rows.push(BoardRow::Worker { seat: seat.clone() });
        }
        for item in &self.flow {
            rows.push(BoardRow::Flow { item: item.clone() });
        }
        self.board_rows = rows;

        if let Some(key) = prev_key {
            if let Some(i) = self.board_rows.iter().position(|r| r.key() == key) {
                self.selection = i;
            }
        }
        if self.selection >= self.board_rows.len() {
            self.selection = self.board_rows.len().saturating_sub(1);
        }

        // Refresh inspector seat status
        if let Some(seat) = self.selected_seat.clone() {
            self.detail_seat = fleet::read_seat_status(&seat)
                .or_else(|| self.seats.iter().find(|s| s.seat == seat).cloned());
        }
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.board_rows.is_empty() {
            return;
        }
        let n = self.board_rows.len() as isize;
        let cur = self.selection as isize;
        self.selection = ((cur + delta).rem_euclid(n)) as usize;
        self.apply_selection();
    }

    pub fn apply_selection(&mut self) {
        match self.board_rows.get(self.selection).cloned() {
            Some(BoardRow::Worker { seat }) => {
                let name = seat.seat.clone();
                if self.selected_seat.as_deref() != Some(name.as_str()) {
                    self.log_lines.clear();
                    self.log_scroll = 0;
                }
                self.selected_seat = Some(name);
                self.selected_flow_id = None;
                self.detail_seat = Some(seat);
            }
            Some(BoardRow::Flow { item }) => {
                self.selected_flow_id = Some(item.id.clone());
                // Prefer a worker currently on this bead
                if let Some(w) = self
                    .seats
                    .iter()
                    .find(|s| s.last_bead.as_deref() == Some(item.id.as_str()))
                {
                    if self.selected_seat.as_deref() != Some(w.seat.as_str()) {
                        self.log_lines.clear();
                        self.log_scroll = 0;
                    }
                    self.selected_seat = Some(w.seat.clone());
                    self.detail_seat = Some(w.clone());
                }
                self.status_line = format!("{} · {}", item.stage.tag(), item.title);
            }
            None => {}
        }
    }

    pub fn select_seat(&mut self, seat: &str) {
        if let Some(i) = self
            .board_rows
            .iter()
            .position(|r| matches!(r, BoardRow::Worker { seat: s } if s.seat == seat))
        {
            self.selection = i;
            self.apply_selection();
        } else {
            self.selected_seat = Some(seat.to_string());
            self.detail_seat = fleet::read_seat_status(seat);
            self.log_lines.clear();
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selected_seat.is_some()
    }

    pub fn blocked_count(&self) -> usize {
        self.seats
            .iter()
            .filter(|s| SeatAttention::from_status(s) == SeatAttention::Blocked)
            .count()
    }

    pub fn first_blocked_seat(&self) -> Option<&str> {
        self.seats
            .iter()
            .find(|s| SeatAttention::from_status(s) == SeatAttention::Blocked)
            .map(|s| s.seat.as_str())
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
    }

    pub fn log_scroll_by(&mut self, delta: isize) {
        let max = self.log_lines.len().saturating_sub(1);
        if delta < 0 {
            self.log_scroll = (self.log_scroll as isize - delta).min(max as isize).max(0) as usize;
        } else {
            self.log_scroll = self.log_scroll.saturating_sub(delta as usize);
        }
    }

    /// Snapshot for runtime socket `city.board`.
    pub fn board_snapshot_json() -> serde_json::Value {
        let mut state = Self::default();
        state.refresh();
        let workers: Vec<_> = state
            .seats
            .iter()
            .map(|s| {
                serde_json::json!({
                    "seat": s.seat,
                    "state": s.state,
                    "running": s.running,
                    "last_bead": s.last_bead,
                    "last_line": s.last_line,
                    "caste": caste_badge(&s.seat),
                    "attention": format!("{:?}", SeatAttention::from_status(s)).to_lowercase(),
                })
            })
            .collect();
        let flow: Vec<_> = state
            .flow
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "title": f.title,
                    "stage": f.stage.tag(),
                    "kind": f.kind.as_str(),
                })
            })
            .collect();
        let wishes: Vec<_> = state
            .flow
            .iter()
            .filter(|f| f.stage == FlowStage::Wish)
            .map(|f| serde_json::json!({ "id": f.id, "title": f.title }))
            .collect();
        let ready: Vec<_> = state
            .flow
            .iter()
            .filter(|f| f.stage == FlowStage::Ready)
            .map(|f| serde_json::json!({ "id": f.id, "title": f.title }))
            .collect();
        serde_json::json!({
            "workers": workers,
            "flow": flow,
            "wishes": wishes,
            "ready": ready,
        })
    }
}

/// Legacy row enum kept so older hit/activate call sites compile during transition.
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

pub fn render_fleet_panel(
    frame: &mut Frame,
    area: Rect,
    state: &CityPanelState,
    palette: &Palette,
    focus: FocusZone,
) {
    render_city_panel(frame, area, state, palette, focus);
}

pub fn render_city_panel(
    frame: &mut Frame,
    area: Rect,
    state: &CityPanelState,
    palette: &Palette,
    focus: FocusZone,
) {
    let title = format!("City · {}w · focus:{}", state.seats.len(), focus.label());
    let block = widgets::panel_block(&title, palette, focus.is_city());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // wish
            Constraint::Min(6),     // board
            Constraint::Length(10), // inspector
        ])
        .split(inner);

    render_wish_composer(
        frame,
        chunks[0],
        state,
        palette,
        focus == FocusZone::CityWish,
    );
    render_board(
        frame,
        chunks[1],
        state,
        palette,
        focus == FocusZone::CityBoard,
    );
    render_inspector(frame, chunks[2], state, palette, focus);
}

fn render_wish_composer(
    frame: &mut Frame,
    area: Rect,
    state: &CityPanelState,
    palette: &Palette,
    focused: bool,
) {
    let cursor = if focused { "▌" } else { "" };
    let style = if focused {
        Style::default()
            .fg(palette.highlight_fg)
            .bg(palette.highlight_bg)
    } else {
        Style::default().fg(palette.subtext)
    };
    let text = if state.wish_text.is_empty() && !focused {
        "wish> (Tab focus · type ambition · ↵ create)".into()
    } else {
        format!("wish> {}{cursor}", state.wish_text)
    };
    frame.render_widget(Paragraph::new(Span::styled(text, style)), area);
}

fn render_board(
    frame: &mut Frame,
    area: Rect,
    state: &CityPanelState,
    palette: &Palette,
    focused: bool,
) {
    let max_w = (area.width as usize).saturating_sub(2).max(8);
    let mut lines: Vec<Line> = Vec::new();
    let blocked = state.blocked_count();

    lines.push(Line::from(vec![
        Span::styled(
            " WORKERS ",
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
            format!(" {}  [u]spawn  [d]stop  [X]del ", state.seats.len()),
            Style::default().fg(palette.overlay0),
        ),
    ]));

    if state.seats.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none — press u to spawn",
            Style::default().fg(palette.muted),
        )));
    }

    for (i, row) in state.board_rows.iter().enumerate() {
        let is_worker = matches!(row, BoardRow::Worker { .. });
        // Section header before first flow item
        if matches!(row, BoardRow::Flow { .. })
            && (i == 0 || matches!(state.board_rows.get(i - 1), Some(BoardRow::Worker { .. })))
        {
            lines.push(Line::from(Span::styled(
                " FLOW  wish→ready→doing ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        if is_worker && i == 0 && !state.seats.is_empty() {
            // already have WORKERS header
        }

        let sel = focused && i == state.selection;
        match row {
            BoardRow::Worker { seat } => {
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
            }
            BoardRow::Flow { item } => {
                let marker = if sel { "›" } else { " " };
                let glyph = match item.stage {
                    FlowStage::Wish => "◆",
                    FlowStage::Ready => "▸",
                    FlowStage::Doing => "◐",
                    FlowStage::Done => "✓",
                };
                let row_s = status::ellipsize(
                    &format!(
                        "{marker}{glyph} {} [{}] {}",
                        item.id,
                        item.stage.tag(),
                        item.title
                    ),
                    max_w,
                );
                let style = if sel {
                    Style::default()
                        .fg(palette.highlight_fg)
                        .bg(palette.highlight_bg)
                } else {
                    Style::default().fg(match item.stage {
                        FlowStage::Wish => palette.warn,
                        FlowStage::Ready => palette.ok,
                        FlowStage::Doing => palette.state_working,
                        FlowStage::Done => palette.state_done,
                    })
                };
                lines.push(Line::from(Span::styled(row_s, style)));
            }
        }
    }

    // Spawn inline hint when spawn zone focused — drawn at end of board area via status
    if !state.status_line.is_empty() {
        lines.push(Line::from(Span::styled(
            status::ellipsize(&state.status_line, max_w),
            Style::default().fg(palette.muted),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_inspector(
    frame: &mut Frame,
    area: Rect,
    state: &CityPanelState,
    palette: &Palette,
    focus: FocusZone,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(2),
            Constraint::Length(2),
        ])
        .split(area);

    let seat_name = state.selected_seat.as_deref().unwrap_or("(none)");
    let insp_focus = matches!(
        focus,
        FocusZone::CityInspector | FocusZone::CitySteer | FocusZone::CitySpawn
    );

    let mut top: Vec<Line> = Vec::new();
    if let Some(s) = &state.detail_seat {
        let att = SeatAttention::from_status(s);
        top.push(Line::from(vec![
            Span::styled(
                format!(" {} ", att.icon()),
                Style::default().fg(att.color(palette)),
            ),
            Span::styled(
                format!("{seat_name} [{}] {}", caste_badge(seat_name), s.state),
                Style::default()
                    .fg(if insp_focus {
                        palette.text
                    } else {
                        palette.subtext
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        top.push(Line::from(Span::styled(
            "[f]ollow [a]ttach [o]pen [b]abort [D]etach [d]stop [X]delete",
            Style::default().fg(palette.overlay0),
        )));
    } else if focus == FocusZone::CitySpawn {
        let fmark = if state.spawn_focus_fleet { "›" } else { " " };
        let cmark = if state.spawn_focus_fleet { " " } else { "›" };
        top.push(Line::from(Span::styled(
            format!(
                "spawn {fmark}Fleet:{}  {cmark}Crew:{}  (Tab field · ↵ go)",
                state.spawn_fleet_n, state.spawn_crew_n
            ),
            Style::default()
                .fg(palette.highlight_fg)
                .bg(palette.highlight_bg),
        )));
        top.push(Line::from(Span::styled(
            "Esc back · ↑↓ workers",
            Style::default().fg(palette.overlay0),
        )));
    } else {
        top.push(Line::from(Span::styled(
            "inspector — select a worker",
            Style::default().fg(palette.muted),
        )));
        top.push(Line::from(Span::styled(
            "↑↓ board · Tab zones · u spawn",
            Style::default().fg(palette.overlay0),
        )));
    }
    frame.render_widget(Paragraph::new(top), chunks[0]);

    // Log
    let log_h = chunks[1].height as usize;
    let visible = log_h.max(1);
    let end = state.log_lines.len().saturating_sub(state.log_scroll);
    let start = end.saturating_sub(visible);
    let slice = if start < end {
        &state.log_lines[start..end]
    } else {
        &[][..]
    };
    let mut log_lines: Vec<Line> = Vec::new();
    if slice.is_empty() {
        log_lines.push(Line::from(Span::styled(
            if state.selected_seat.is_some() {
                "(log empty — f to follow)"
            } else {
                ""
            },
            Style::default().fg(palette.muted),
        )));
    } else {
        for l in slice {
            log_lines.push(Line::from(Span::styled(
                status::ellipsize(l, (chunks[1].width as usize).saturating_sub(1).max(8)),
                Style::default().fg(palette.overlay1),
            )));
        }
    }
    frame.render_widget(Paragraph::new(log_lines), chunks[1]);

    // Steer composer
    let steer_focus = focus == FocusZone::CitySteer;
    let cursor = if steer_focus { "▌" } else { "" };
    let style = if steer_focus {
        Style::default()
            .fg(palette.highlight_fg)
            .bg(palette.highlight_bg)
    } else {
        Style::default().fg(palette.subtext)
    };
    let steer = if state.selected_seat.is_none() {
        String::new()
    } else if state.steer_text.is_empty() && !steer_focus {
        "steer> (Tab · type · ↵)".into()
    } else {
        format!("steer> {}{cursor}", state.steer_text)
    };
    frame.render_widget(Paragraph::new(Span::styled(steer, style)), chunks[2]);
}

// Silence unused import warnings for BeadStatus in this rewrite.
#[allow(dead_code)]
fn _bead_status_short(st: BeadStatus) -> &'static str {
    match st {
        BeadStatus::Open => "open",
        BeadStatus::Claimed => "claim",
        BeadStatus::Blocked => "block",
        BeadStatus::Gated => "gate",
        BeadStatus::Closed => "done",
    }
}
