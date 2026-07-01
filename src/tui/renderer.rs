use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::FontStyle;
use syntect::parsing::SyntaxSet;
use std::sync::OnceLock;

fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(|| SyntaxSet::load_defaults_newlines())
}

fn highlight_code(code: &str, lang: &str) -> Vec<Line<'static>> {
    let ss = syntax_set();
    let ts = syntect::highlighting::ThemeSet::load_defaults();
    let syntax = ss
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    let theme = &ts.themes["base16-ocean.dark"];
    let mut highlighter = HighlightLines::new(syntax, theme);

    code.lines()
        .map(|line| {
            let ranges = highlighter.highlight_line(line, ss).unwrap_or_default();
            let spans: Vec<Span<'static>> = ranges
                .into_iter()
                .map(|(style, text)| {
                    let fg = style.foreground;
                    let mut s = Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b));
                    if style.font_style.contains(FontStyle::BOLD) {
                        s = s.add_modifier(Modifier::BOLD);
                    }
                    if style.font_style.contains(FontStyle::ITALIC) {
                        s = s.add_modifier(Modifier::ITALIC);
                    }
                    Span::styled(text.to_string(), s)
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let parser = Parser::new_ext(text, options);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut para_sep = false;

    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_text = String::new();
    let mut in_heading = false;
    let mut heading_lvl = 0u32;
    let mut in_blockquote = false;
    let mut list_depth: usize = 0;
    let mut is_first_item_line = false;
    let mut emph_bold = false;
    let mut emph_italic = false;
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
                    code_lang = match kind {
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
                Tag::Strikethrough => {}
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
                        let highlighted = highlight_code(&code_text, &code_lang);
                        for hl_line in highlighted {
                            let mut s = Vec::with_capacity(hl_line.spans.len() + 1);
                            s.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
                            s.extend(hl_line.spans);
                            lines.push(Line::from(s));
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
                TagEnd::Strikethrough => {}
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    code_text.push_str(&text);
                } else {
                    let mut style = Style::default();
                    if in_heading {
                        let c = match heading_lvl {
                            1 => Color::Yellow,
                            2 => Color::LightYellow,
                            _ => Color::White,
                        };
                        style = style.fg(c).add_modifier(Modifier::BOLD);
                    }
                    if in_blockquote {
                        style = style.fg(Color::DarkGray);
                    }
                    if emph_bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if emph_italic {
                        style = style.add_modifier(Modifier::ITALIC);
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
                    text.to_string(),
                    Style::default().fg(Color::Cyan),
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
                    "─".repeat(50),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Event::Html(text) => {
                spans.push(Span::styled(
                    text.to_string(),
                    Style::default().fg(Color::DarkGray),
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
