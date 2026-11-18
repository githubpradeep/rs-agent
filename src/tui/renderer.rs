use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Palette;

/// Palette-derived markdown colors (light themes no longer get dark ANSI).
#[derive(Clone, Copy)]
pub struct MarkdownStyle {
    pub heading1: Color,
    pub heading2: Color,
    pub heading3: Color,
    pub blockquote: Color,
    pub inline_code: Color,
    pub rule: Color,
    pub code_gutter: Color,
    pub text: Color,
}

impl MarkdownStyle {
    pub fn from_palette(p: &Palette) -> Self {
        Self {
            heading1: p.assistant,
            heading2: p.accent,
            heading3: p.text,
            blockquote: p.overlay0,
            inline_code: p.tool,
            rule: p.border,
            code_gutter: p.overlay0,
            text: p.text,
        }
    }
}

fn render_code_lines(code: &str, md: MarkdownStyle) -> Vec<Line<'static>> {
    code.lines()
        .map(|line| {
            let mut s = Vec::with_capacity(2);
            s.push(Span::styled(
                " │ ".to_string(),
                Style::default().fg(md.code_gutter),
            ));
            s.push(Span::styled(
                if line.is_empty() {
                    " ".to_string()
                } else {
                    line.to_string()
                },
                Style::default().fg(md.inline_code),
            ));
            Line::from(s)
        })
        .collect()
}

/// Renders markdown to styled ratatui lines; fenced code blocks get a
/// gutter-marked plain style.
pub fn render_markdown(text: &str, md: MarkdownStyle) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let parser = Parser::new_ext(text, options);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut para_sep = false;

    let mut in_code_block = false;
    let mut code_text = String::new();
    let mut in_heading = false;
    let mut heading_lvl = 0u32;
    let mut in_blockquote = false;
    let mut list_depth: usize = 0;
    let mut is_first_item_line = false;
    let mut emph_bold = false;
    let mut emph_italic = false;
    let mut strike = false;
    macro_rules! flush_line {
        () => {{
            let taken = std::mem::take(&mut spans);
            lines.push(if taken.is_empty() {
                Line::from("")
            } else {
                Line::from(taken)
            });
        }};
    }

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if para_sep && list_depth == 0 {
                        lines.push(Line::from(""));
                    }
                    para_sep = false;
                }
                Tag::Heading { level, .. } => {
                    in_heading = true;
                    heading_lvl = level as u32;
                    para_sep = false;
                }
                Tag::BlockQuote => {
                    in_blockquote = true;
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    // Language token is parsed but unused (plain code styling).
                    let _lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    code_text.clear();
                }
                Tag::List { .. } => {
                    list_depth += 1;
                }
                Tag::Item => {
                    is_first_item_line = true;
                }
                Tag::Emphasis => emph_italic = true,
                Tag::Strong => emph_bold = true,
                Tag::Link { .. } | Tag::Image { .. } => {}
                Tag::Strikethrough => strike = true,
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    flush_line!();
                    para_sep = true;
                }
                TagEnd::Heading(_) => {
                    in_heading = false;
                    heading_lvl = 0;
                    flush_line!();
                }
                TagEnd::BlockQuote => {
                    in_blockquote = false;
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    if !code_text.is_empty() {
                        for hl_line in render_code_lines(&code_text, md) {
                            lines.push(hl_line);
                        }
                    }
                    code_text.clear();
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                }
                TagEnd::Item => {
                    if !spans.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut spans)));
                    }
                    is_first_item_line = false;
                }
                TagEnd::Emphasis => emph_italic = false,
                TagEnd::Strong => emph_bold = false,
                TagEnd::Link | TagEnd::Image => {}
                TagEnd::Strikethrough => strike = false,
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    code_text.push_str(&text);
                } else {
                    let mut style = Style::default().fg(md.text);
                    if in_heading {
                        let c = match heading_lvl {
                            1 => md.heading1,
                            2 => md.heading2,
                            _ => md.heading3,
                        };
                        style = style.fg(c).add_modifier(Modifier::BOLD);
                    }
                    if in_blockquote {
                        style = style.fg(md.blockquote);
                    }
                    if emph_bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if emph_italic {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if strike {
                        style = style.add_modifier(Modifier::CROSSED_OUT);
                    }
                    if is_first_item_line && spans.is_empty() {
                        let indent = "  ".repeat(list_depth.saturating_sub(1));
                        spans.push(Span::raw(format!("{}• ", indent)));
                        is_first_item_line = false;
                    }
                    spans.push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(text) => {
                spans.push(Span::styled(
                    format!(" {} ", text),
                    Style::default().fg(md.inline_code).bg(Color::Reset),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code_block {
                    code_text.push('\n');
                } else {
                    flush_line!();
                }
            }
            Event::Rule => {
                lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(md.rule),
                )));
            }
            Event::Html(text) => {
                spans.push(Span::styled(
                    text.to_string(),
                    Style::default().fg(md.blockquote),
                ));
            }
            _ => {}
        }
    }

    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}
