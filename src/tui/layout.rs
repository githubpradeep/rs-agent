//! Pure geometry for the TUI shell (herdr `compute_view` idea).

use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub struct ViewRects {
    pub header: Rect,
    pub chat: Rect,
    pub side: Option<Rect>,
    pub repl: Option<Rect>,
    pub input: Rect,
    pub footer: Rect,
    pub full: Rect,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutOpts {
    pub show_repl: bool,
    pub show_side: bool,
    /// Side panel width percent (25–40 typical).
    pub side_pct: u16,
    /// Bottom console height (live bash/repl). Keep chat width stable.
    pub repl_height: u16,
}

/// Compute chrome geometry. Does not mutate app state.
pub fn compute_view(area: Rect, opts: LayoutOpts) -> ViewRects {
    // header(1) + body(min) + optional repl + input(3) + footer(1)
    let mut constraints = vec![
        Constraint::Length(1), // header
        Constraint::Min(3),    // chat (+ side)
    ];
    if opts.show_repl {
        let h = opts.repl_height.clamp(4, 14);
        constraints.push(Constraint::Length(h));
    }
    constraints.push(Constraint::Length(3)); // input
    constraints.push(Constraint::Length(1)); // footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;
    let header = chunks[idx];
    idx += 1;
    let body = chunks[idx];
    idx += 1;
    let repl = if opts.show_repl {
        let r = chunks[idx];
        idx += 1;
        Some(r)
    } else {
        None
    };
    let input = chunks[idx];
    idx += 1;
    let footer = chunks[idx];

    let (chat, side) = if opts.show_side {
        let pct = opts.side_pct.clamp(22, 48);
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(100 - pct),
                Constraint::Percentage(pct),
            ])
            .split(body);
        (h[0], Some(h[1]))
    } else {
        (body, None)
    };

    ViewRects {
        header,
        chat,
        side,
        repl,
        input,
        footer,
        full: area,
    }
}
