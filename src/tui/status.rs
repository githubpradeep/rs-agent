//! Session attention state — herdr blocked/working/done/idle vocabulary.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Palette;

/// High-signal session state for the header chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionUiState {
    /// Needs a human answer (permission / question).
    Blocked,
    /// Agent or tool is running.
    Working,
    /// Finished since last focus (unseen completion).
    Done,
    /// Idle / ready.
    Idle,
}

impl SessionUiState {
    pub fn icon(self) -> &'static str {
        match self {
            Self::Blocked => "●",
            Self::Working => "◐",
            Self::Done => "✓",
            Self::Idle => "○",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Working => "working",
            Self::Done => "done",
            Self::Idle => "idle",
        }
    }

    pub fn color(self, p: &Palette) -> ratatui::style::Color {
        match self {
            Self::Blocked => p.state_blocked,
            Self::Working => p.state_working,
            Self::Done => p.state_done,
            Self::Idle => p.state_idle,
        }
    }

    /// Attention rank: blocked > done > working > idle (herdr).
    pub fn priority(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Done => 1,
            Self::Working => 2,
            Self::Idle => 3,
        }
    }

    pub fn from_app(
        pending_permission: bool,
        pending_question: bool,
        waiting: bool,
        tool_running: bool,
        status: &str,
        unseen_done: bool,
    ) -> Self {
        if pending_permission || pending_question || status == "STUCK" {
            return Self::Blocked;
        }
        if waiting || tool_running || status == "thinking..." || status.starts_with("using ") {
            return Self::Working;
        }
        if status == "error" || status == "recover" {
            return Self::Blocked;
        }
        if unseen_done || status == "aborted" {
            return Self::Done;
        }
        Self::Idle
    }
}

/// Compact `● working` chip for the header.
pub fn state_chip(state: SessionUiState, palette: &Palette) -> Vec<Span<'static>> {
    let color = state.color(palette);
    vec![
        Span::styled(
            format!("{} ", state.icon()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            state.label().to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ]
}

/// Truncate by display chars with an ellipsis.
pub fn ellipsize(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// Build a single header line: state · model · mode · chips | tokens cost
pub fn render_header_line(
    state: SessionUiState,
    provider: &str,
    model: &str,
    mode: &str,
    depth: u32,
    deep: bool,
    yolo: &str,
    session_short: &str,
    title: Option<&str>,
    goal: &str,
    attach: &str,
    tokens: &str,
    cost: &str,
    max_width: usize,
    palette: &Palette,
) -> Line<'static> {
    let mut left: Vec<Span<'static>> = state_chip(state, palette);
    left.push(Span::styled(" · ".to_string(), Style::default().fg(palette.overlay0)));
    left.push(Span::styled(
        format!("{provider}/{model}"),
        Style::default().fg(palette.subtext),
    ));
    left.push(Span::styled(" · ".to_string(), Style::default().fg(palette.overlay0)));
    left.push(Span::styled(mode.to_string(), Style::default().fg(palette.accent)));
    if depth > 0 {
        left.push(Span::styled(
            format!(" d{depth}"),
            Style::default().fg(palette.overlay1),
        ));
    }
    if deep {
        left.push(Span::styled(
            " [D]".to_string(),
            Style::default()
                .fg(palette.state_done)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if !yolo.is_empty() {
        left.push(Span::styled(
            format!(" {yolo}"),
            Style::default().fg(palette.warn),
        ));
    }
    left.push(Span::styled(
        format!(" {session_short}"),
        Style::default().fg(palette.overlay0),
    ));
    if let Some(t) = title {
        if !t.is_empty() {
            left.push(Span::styled(
                format!(" \"{}\"", ellipsize(t, 28)),
                Style::default().fg(palette.subtext),
            ));
        }
    }
    if !goal.is_empty() {
        left.push(Span::styled(
            format!(" {goal}"),
            Style::default().fg(palette.accent),
        ));
    }
    if !attach.is_empty() {
        left.push(Span::styled(
            format!(" {attach}"),
            Style::default().fg(palette.warn),
        ));
    }

    let right = format!("{tokens}{cost}");
    let left_plain: String = left.iter().map(|s| s.content.as_ref()).collect();
    let left_w = left_plain.chars().count();
    let right_w = right.chars().count();
    let pad = max_width.saturating_sub(left_w).saturating_sub(right_w);

    let mut spans = left;
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    } else if left_w + right_w > max_width {
        // Drop left content to keep right visible.
        let budget = max_width.saturating_sub(right_w).saturating_sub(1);
        let truncated = ellipsize(&left_plain, budget);
        spans = vec![Span::styled(truncated, Style::default().fg(palette.subtext))];
        let gap = max_width
            .saturating_sub(spans[0].content.chars().count())
            .saturating_sub(right_w);
        if gap > 0 {
            spans.push(Span::raw(" ".repeat(gap)));
        }
    }
    if !tokens.is_empty() {
        spans.push(Span::styled(
            tokens.to_string(),
            Style::default().fg(palette.overlay1),
        ));
    }
    if !cost.is_empty() {
        spans.push(Span::styled(
            cost.to_string(),
            Style::default().fg(palette.overlay0),
        ));
    }
    Line::from(spans)
}

/// Contextual footer hints only (herdr mode bar — ephemeral).
pub fn render_footer_line(
    hints: &str,
    status: &str,
    spinner: &str,
    status_color: ratatui::style::Color,
    palette: &Palette,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        hints.to_string(),
        Style::default().fg(palette.overlay0),
    )];
    if !status.is_empty() && status != "ready" {
        spans.push(Span::styled("  ".to_string(), Style::default()));
        spans.push(Span::styled(
            status.to_string(),
            Style::default().fg(status_color),
        ));
    }
    if !spinner.is_empty() {
        spans.push(Span::styled(
            spinner.to_string(),
            Style::default()
                .fg(palette.state_working)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}
