//! Attention toasts (herdr): blocked / finished only, suppress when focused.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use std::time::{Duration, Instant};

use super::status::SessionUiState;
use super::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    NeedsAttention,
    Finished,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: String,
    pub body: String,
    pub created: Instant,
    pub ttl: Duration,
}

impl Toast {
    pub fn blocked(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind: ToastKind::NeedsAttention,
            title: title.into(),
            body: body.into(),
            created: Instant::now(),
            ttl: Duration::from_secs(8),
        }
    }

    pub fn finished(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind: ToastKind::Finished,
            title: title.into(),
            body: body.into(),
            created: Instant::now(),
            ttl: Duration::from_secs(5),
        }
    }

    pub fn expired(&self) -> bool {
        self.created.elapsed() >= self.ttl
    }

    pub fn color(&self, p: &Palette) -> ratatui::style::Color {
        match self.kind {
            ToastKind::NeedsAttention => p.state_blocked,
            ToastKind::Finished => p.state_done,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self.kind {
            ToastKind::NeedsAttention => SessionUiState::Blocked.icon(),
            ToastKind::Finished => SessionUiState::Done.icon(),
        }
    }
}

/// Play a short attention sound when enabled (best-effort).
pub fn play_sound(kind: ToastKind) {
    #[cfg(target_os = "macos")]
    {
        let name = match kind {
            ToastKind::NeedsAttention => "/System/Library/Sounds/Purr.aiff",
            ToastKind::Finished => "/System/Library/Sounds/Pop.aiff",
        };
        let _ = std::process::Command::new("afplay")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        return;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        // BEL — terminals that support it will beep.
        let _ = std::io::Write::write_all(&mut std::io::stdout(), b"\x07");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}

/// Top-right toast rect.
pub fn toast_area(full: Rect) -> Rect {
    let width = full.width.min(42).max(18);
    let height = 4u16;
    Rect {
        x: full.x + full.width.saturating_sub(width + 1),
        y: full.y.saturating_add(1),
        width,
        height,
    }
}

pub fn render_toast(frame: &mut Frame, area: Rect, toast: &Toast, palette: &Palette) {
    let color = toast.color(palette);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", toast.icon()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                toast.title.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            toast.body.clone(),
            Style::default().fg(palette.subtext),
        )),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(palette.panel_bg));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(palette.panel_bg)),
        area,
    );
}
