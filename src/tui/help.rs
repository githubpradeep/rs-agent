//! Filterable `?` keybind / slash-command help overlay (herdr keybind_help).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem};
use ratatui::Frame;

use super::keys::KeyMap;
use super::theme::Palette;
use super::widgets;

#[derive(Debug, Clone)]
pub struct HelpOverlay {
    pub query: String,
    pub selection: usize,
}

impl Default for HelpOverlay {
    fn default() -> Self {
        Self {
            query: String::new(),
            selection: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HelpEntry {
    pub keys: String,
    pub desc: String,
    pub group: &'static str,
}

pub fn build_entries(keys: &KeyMap) -> Vec<HelpEntry> {
    let mut e = vec![
        HelpEntry {
            keys: "^C".into(),
            desc: "quit".into(),
            group: "global",
        },
        HelpEntry {
            keys: keys.binding("insert").into(),
            desc: "enter insert mode".into(),
            group: "normal",
        },
        HelpEntry {
            keys: "Esc".into(),
            desc: "normal mode / abort / dismiss overlay".into(),
            group: "global",
        },
        HelpEntry {
            keys: keys.binding("toggle_thinking").into(),
            desc: "toggle thinking block".into(),
            group: "normal",
        },
        HelpEntry {
            keys: keys.binding("expand_tool").into(),
            desc: "expand/collapse last tool".into(),
            group: "normal",
        },
        HelpEntry {
            keys: keys.binding("toggle_tree").into(),
            desc: "toggle call tree panel".into(),
            group: "normal",
        },
        HelpEntry {
            keys: keys.binding("jump_bottom").into(),
            desc: "jump to bottom".into(),
            group: "normal",
        },
        HelpEntry {
            keys: format!(
                "{}/{}/{}/{}",
                keys.binding("perm_once"),
                keys.binding("perm_path"),
                keys.binding("perm_always"),
                keys.binding("perm_deny")
            ),
            desc: "permission once / path / always / deny".into(),
            group: "permission",
        },
        HelpEntry {
            keys: "?".into(),
            desc: "this help (filterable)".into(),
            group: "global",
        },
        HelpEntry {
            keys: "^K".into(),
            desc: "command palette".into(),
            group: "global",
        },
        HelpEntry {
            keys: "^P".into(),
            desc: "cycle provider/model".into(),
            group: "global",
        },
        HelpEntry {
            keys: "@ / #".into(),
            desc: "file / directory picker".into(),
            group: "insert",
        },
        HelpEntry {
            keys: "/help".into(),
            desc: "list slash commands".into(),
            group: "slash",
        },
        HelpEntry {
            keys: "/keys".into(),
            desc: "keybind cheat sheet".into(),
            group: "slash",
        },
        HelpEntry {
            keys: "/theme".into(),
            desc: "switch theme dark|light|forest".into(),
            group: "slash",
        },
        HelpEntry {
            keys: "/settings".into(),
            desc: "settings modal".into(),
            group: "slash",
        },
        HelpEntry {
            keys: "/tree".into(),
            desc: "Deep Context call tree".into(),
            group: "slash",
        },
        HelpEntry {
            keys: "/timeline".into(),
            desc: "session timeline / fork".into(),
            group: "slash",
        },
        HelpEntry {
            keys: keys.binding("toggle_sessions").into(),
            desc: "toggle sessions panel".into(),
            group: "normal",
        },
        HelpEntry {
            keys: keys.binding("toggle_city").into(),
            desc: "toggle city cockpit (wish/spawn/watch)".into(),
            group: "normal",
        },
        HelpEntry {
            keys: "/fleet".into(),
            desc: "city cockpit panel (alias /city)".into(),
            group: "slash",
        },
        HelpEntry {
            keys: "w/u/d".into(),
            desc: "city: wish / spawn / stop (panel open)".into(),
            group: "city",
        },
        HelpEntry {
            keys: "f/a/o/s/b".into(),
            desc: "seat detail: follow/attach/open/steer/abort".into(),
            group: "city",
        },
        HelpEntry {
            keys: "/model /provider".into(),
            desc: "model / provider pickers".into(),
            group: "slash",
        },
        HelpEntry {
            keys: "/goal".into(),
            desc: "set / pause / clear auto-continue goal".into(),
            group: "slash",
        },
    ];
    e.sort_by(|a, b| a.group.cmp(b.group).then(a.keys.cmp(&b.keys)));
    e
}

pub fn filtered<'a>(entries: &'a [HelpEntry], query: &str) -> Vec<&'a HelpEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return entries.iter().collect();
    }
    entries
        .iter()
        .filter(|e| {
            e.keys.to_lowercase().contains(&q)
                || e.desc.to_lowercase().contains(&q)
                || e.group.contains(&q)
        })
        .collect()
}

pub fn render_help(
    frame: &mut Frame,
    full: Rect,
    overlay: &HelpOverlay,
    entries: &[HelpEntry],
    palette: &Palette,
) {
    let matched = filtered(entries, &overlay.query);
    let height = (matched.len() as u16).clamp(4, 16).saturating_add(4);
    let area = widgets::centered_rect(full, 72, height);

    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled(
            " filter ",
            Style::default()
                .fg(palette.contrast_on_accent())
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", if overlay.query.is_empty() { "type to filter…" } else { &overlay.query }),
            Style::default().fg(palette.text),
        ),
    ])];
    lines.push(Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(palette.border),
    )));

    if matched.is_empty() {
        lines.push(Line::from(Span::styled(
            " (no matches)",
            Style::default().fg(palette.muted),
        )));
    } else {
        for (i, e) in matched.iter().enumerate() {
            let selected = i == overlay.selection.min(matched.len().saturating_sub(1));
            let style = if selected {
                Style::default()
                    .fg(palette.highlight_fg)
                    .bg(palette.highlight_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.subtext)
            };
            lines.push(Line::from(Span::styled(
                format!(" {:12}  {} ({})", e.keys, e.desc, e.group),
                style,
            )));
        }
    }
    lines.push(Line::from(Span::styled(
        " Esc close · ↑↓ select · / clears with ctrl+u",
        Style::default().fg(palette.overlay0),
    )));

    widgets::render_modal_shell(frame, area, "help", palette.accent, palette, lines);
}

/// Simple list render used by command palette (shared chrome).
pub fn render_palette_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: &[String],
    selection: usize,
    palette: &Palette,
) {
    let list_items: Vec<ListItem> = if items.is_empty() {
        vec![ListItem::new("(no matches)").style(Style::default().fg(palette.muted))]
    } else {
        items
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let style = if i == selection {
                    Style::default()
                        .fg(palette.highlight_fg)
                        .bg(palette.highlight_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.subtext)
                };
                ListItem::new(s.as_str()).style(style)
            })
            .collect()
    };
    use ratatui::widgets::{Block, Borders, Clear};
    frame.render_widget(Clear, area);
    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
            .border_style(
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(palette.panel_bg)),
    );
    frame.render_widget(list, area);
}
