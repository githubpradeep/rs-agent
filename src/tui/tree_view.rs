//! Structured call-tree view + TurnBar (Conductor SubAgentTree / TurnBar).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde::{Deserialize, Serialize};

use super::status;
use super::theme::Palette;
use super::widgets;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    #[default]
    Idle,
    Running,
    Ok,
    Fail,
    Wait,
}

impl NodeStatus {
    pub fn from_text(s: &str) -> Self {
        let l = s.to_lowercase();
        if l.contains("fail") || l.contains("error") {
            Self::Fail
        } else if l.contains("run") || l.contains("active") || l.contains("…") || l.contains("...")
        {
            Self::Running
        } else if l.contains("wait") || l.contains("block") {
            Self::Wait
        } else if l.contains("ok") || l.contains("done") || l.contains("complete") {
            Self::Ok
        } else {
            Self::Idle
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Fail => "×",
            Self::Running => "◐",
            Self::Ok => "✓",
            Self::Wait => "○",
            Self::Idle => "·",
        }
    }

    pub fn color(self, p: &Palette) -> ratatui::style::Color {
        match self {
            Self::Fail => p.state_blocked,
            Self::Running => p.state_working,
            Self::Ok => p.state_done,
            Self::Wait => p.state_idle,
            Self::Idle => p.overlay1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallTreeNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub status: NodeStatus,
    pub duration_ms: Option<u64>,
    pub tokens: Option<u64>,
    pub children: Vec<CallTreeNode>,
    #[serde(default)]
    pub expanded: bool,
}

impl CallTreeNode {
    pub fn child_count(&self) -> usize {
        self.children.len()
    }
}

/// Parse legacy text tree into a flat-ish structured list (indent preserved in label).
pub fn parse_text_tree(text: &str) -> Vec<CallTreeNode> {
    let mut roots: Vec<CallTreeNode> = Vec::new();
    let mut stack: Vec<(usize, Vec<usize>)> = Vec::new(); // indent → path of child indexes

    for (i, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let indent = raw
            .chars()
            .take_while(|c| c.is_whitespace() || "│├└─".contains(*c))
            .count();
        let label = raw
            .trim()
            .trim_start_matches(|c: char| "│├└─ ".contains(c))
            .to_string();
        if label.is_empty() {
            continue;
        }
        let kind = if label.to_lowercase().contains("llm") {
            "llm"
        } else if label.to_lowercase().contains("agent") {
            "agent"
        } else if label.to_lowercase().contains("repl") {
            "repl"
        } else {
            "node"
        }
        .to_string();
        let node = CallTreeNode {
            id: format!("n{i}"),
            kind,
            label: label.clone(),
            status: NodeStatus::from_text(&label),
            duration_ms: None,
            tokens: None,
            children: Vec::new(),
            expanded: indent < 6,
        };

        while stack.last().map(|(ind, _)| *ind >= indent).unwrap_or(false) {
            stack.pop();
        }

        if stack.is_empty() {
            let idx = roots.len();
            roots.push(node);
            stack.push((indent, vec![idx]));
        } else {
            let path = stack.last().map(|(_, p)| p.clone()).unwrap_or_default();
            // Navigate via indexes only (avoid stacked &mut).
            fn push_at(roots: &mut [CallTreeNode], path: &[usize], node: CallTreeNode) -> usize {
                if path.is_empty() {
                    return 0;
                }
                let mut cur = &mut roots[path[0]];
                for &idx in &path[1..] {
                    cur = &mut cur.children[idx];
                }
                cur.children.push(node);
                cur.children.len() - 1
            }
            let child_idx = push_at(&mut roots, &path, node);
            let mut new_path = path;
            new_path.push(child_idx);
            stack.push((indent, new_path));
        }
    }

    if roots.is_empty() {
        roots.push(CallTreeNode {
            id: "root".into(),
            kind: "root".into(),
            label: text.lines().next().unwrap_or("(empty)").to_string(),
            status: NodeStatus::Idle,
            duration_ms: None,
            tokens: None,
            children: Vec::new(),
            expanded: true,
        });
    }
    roots
}

/// Turn bar segments from breadcrumb like `root > llm_1 > agent_2`.
pub fn turn_bar_line(breadcrumb: &str, palette: &Palette) -> Line<'static> {
    let parts: Vec<&str> = breadcrumb
        .split(['>', '/', '|'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "idle")
        .collect();
    if parts.is_empty() {
        return Line::from(Span::styled(
            " turns · idle ",
            Style::default().fg(palette.overlay0),
        ));
    }
    let mut spans = vec![Span::styled(
        " turns ",
        Style::default()
            .fg(palette.contrast_on_accent())
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD),
    )];
    for (i, p) in parts.iter().enumerate() {
        let active = i + 1 == parts.len();
        let label = status::ellipsize(p, 16);
        if active {
            spans.push(Span::styled(
                format!("[{label}*]"),
                Style::default()
                    .fg(palette.state_working)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!("[{label}]"),
                Style::default().fg(palette.overlay1),
            ));
        }
    }
    Line::from(spans)
}

pub fn render_nodes(
    nodes: &[CallTreeNode],
    palette: &Palette,
    max_width: usize,
    filter: Option<NodeStatus>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    fn walk(
        nodes: &[CallTreeNode],
        depth: usize,
        palette: &Palette,
        max_width: usize,
        filter: Option<NodeStatus>,
        lines: &mut Vec<Line<'static>>,
    ) {
        for n in nodes {
            if let Some(f) = filter {
                if n.status != f && n.children.is_empty() {
                    continue;
                }
            }
            let indent = "  ".repeat(depth);
            let count = if n.children.is_empty() {
                String::new()
            } else {
                format!(" ×{}", n.child_count())
            };
            let meta = match (n.duration_ms, n.tokens) {
                (Some(ms), Some(tok)) => format!("  {ms}ms · {tok}tok"),
                (Some(ms), None) => format!("  {ms}ms"),
                (None, Some(tok)) => format!("  {tok}tok"),
                _ => String::new(),
            };
            let expand = if n.children.is_empty() {
                " "
            } else if n.expanded {
                "▾"
            } else {
                "▸"
            };
            let body = status::ellipsize(
                &format!(
                    "{indent}{expand} {} {}{count}{meta}",
                    n.status.icon(),
                    n.label
                ),
                max_width,
            );
            let color = n.status.color(palette);
            lines.push(Line::from(Span::styled(body, Style::default().fg(color))));
            if n.expanded {
                walk(&n.children, depth + 1, palette, max_width, filter, lines);
            } else if !n.children.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("{indent}  ▸ {} more", n.child_count()),
                    Style::default().fg(palette.overlay0),
                )));
            }
        }
    }
    walk(nodes, 0, palette, max_width, filter, &mut lines);
    if lines.is_empty() {
        lines.push(widgets::style_tree_line("(no call tree yet)", palette));
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanelMode {
    Tree,
    Timeline,
}

impl SidePanelMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Tree => Self::Timeline,
            Self::Timeline => Self::Tree,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tree => "Tree",
            Self::Timeline => "Timeline",
        }
    }
}
