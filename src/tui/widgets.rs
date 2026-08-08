//! Shared modal / panel kit (herdr: dim backdrop, accent shell, action hints).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::theme::Palette;

/// Dim every cell under a modal (herdr `dim_background`).
pub fn dim_background(frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(cell.style().add_modifier(Modifier::DIM));
            }
        }
    }
}

/// Centered rect clamped to terminal size.
pub fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2).max(10));
    let height = height.min(area.height.saturating_sub(2).max(3));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Clear + bordered panel with optional panel_bg fill.
pub fn render_modal_shell(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    border: ratatui::style::Color,
    palette: &Palette,
    lines: Vec<Line<'static>>,
) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(Style::default().fg(border).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(palette.panel_bg).fg(palette.text));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(palette.panel_bg)), inner);
}

/// Primary / secondary action hint row: `[a] once` style pills.
pub fn action_hints(items: &[(&str, &str)], palette: &Palette) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, label)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ".to_string(), Style::default()));
        }
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(palette.contrast_on_accent())
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(palette.subtext),
        ));
    }
    Line::from(spans)
}

/// Panel title block with accent border.
pub fn panel_block<'a>(title: &'a str, palette: &Palette, focused: bool) -> Block<'a> {
    let border = if focused { palette.accent } else { palette.border };
    Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(Style::default().fg(border))
        .style(Style::default().fg(palette.text))
}

/// Style a call-tree / timeline line using Conductor status vocabulary.
pub fn style_tree_line(line: &str, palette: &Palette) -> Line<'static> {
    let lower = line.to_lowercase();
    let (icon, color) = if lower.contains("error")
        || lower.contains("fail")
        || lower.contains("blocked")
    {
        ("×", palette.state_blocked)
    } else if lower.contains("run")
        || lower.contains("active")
        || lower.contains("working")
        || lower.contains("…")
        || lower.contains("...")
    {
        ("◐", palette.state_working)
    } else if lower.contains("done")
        || lower.contains("ok")
        || lower.contains("complete")
        || lower.contains("success")
    {
        ("✓", palette.state_done)
    } else if lower.contains("idle") || lower.contains("wait") {
        ("○", palette.state_idle)
    } else if line.trim_start().starts_with("├")
        || line.trim_start().starts_with("└")
        || line.trim_start().starts_with("│")
    {
        ("", palette.tool)
    } else {
        ("·", palette.overlay1)
    };

    let max_preview = 120usize;
    let body: String = line.chars().take(max_preview).collect();
    if icon.is_empty() {
        Line::from(Span::styled(body, Style::default().fg(color)))
    } else {
        Line::from(vec![
            Span::styled(
                format!("{icon} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(body, Style::default().fg(palette.subtext)),
        ])
    }
}
