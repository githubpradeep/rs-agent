//! Free helpers used by the TUI app (kept out of the giant App impl).

use super::ChatMessage;
use crate::ai::types::Message;

pub(super) fn summarize_api_messages(messages: &[Message]) -> Vec<(usize, String)> {
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let role = match m.role {
                crate::ai::types::Role::System => "system",
                crate::ai::types::Role::User => "user",
                crate::ai::types::Role::Assistant => "assistant",
                crate::ai::types::Role::Tool => "tool",
            };
            let preview = m
                .content
                .first()
                .and_then(|c| {
                    c.text
                        .as_deref()
                        .or(c.name.as_deref())
                        .or(c.thinking.as_deref())
                })
                .unwrap_or("");
            let preview: String = preview.chars().take(48).collect();
            let preview = preview.replace('\n', " ");
            (i, format!("{role}: {preview}"))
        })
        .collect()
}

pub(super) fn api_messages_to_chat(messages: &[Message]) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    for msg in messages {
        match &msg.role {
            crate::ai::types::Role::User => {
                let text = msg
                    .content
                    .first()
                    .and_then(|c| c.text.as_deref())
                    .unwrap_or("");
                if !text.is_empty() {
                    out.push(ChatMessage {
                        role: "user".into(),
                        text: text.to_string(),
                        thinking: None,
                        show_thinking: false,
                        tool_blocks: Vec::new(),
                    });
                }
            }
            crate::ai::types::Role::Assistant => {
                let mut text = String::new();
                let mut thinking: Option<String> = None;
                for c in &msg.content {
                    match c.content_type {
                        crate::ai::types::ContentType::Text => {
                            if let Some(ref t) = c.text {
                                text.push_str(t);
                            }
                        }
                        crate::ai::types::ContentType::ToolUse => {
                            let name = c.name.as_deref().unwrap_or("tool");
                            let input = c.input.as_ref().map(|v| v.to_string()).unwrap_or_default();
                            let preview: String = input.chars().take(120).collect();
                            text.push_str(&format!("\n🛠 {} {}\n", name, preview));
                        }
                        crate::ai::types::ContentType::Thinking => {
                            if let Some(ref t) = c.thinking {
                                thinking = Some(t.clone());
                            }
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() || thinking.as_ref().is_some_and(|t| !t.is_empty()) {
                    out.push(ChatMessage {
                        role: "assistant".into(),
                        text,
                        thinking: thinking.clone(),
                        show_thinking: thinking.as_ref().map(|t| !t.is_empty()).unwrap_or(false),
                        tool_blocks: Vec::new(),
                    });
                }
            }
            crate::ai::types::Role::Tool => {
                let name = msg
                    .content
                    .first()
                    .and_then(|c| c.name.as_deref())
                    .unwrap_or("tool");
                let result = msg
                    .content
                    .first()
                    .and_then(|c| c.text.as_deref())
                    .unwrap_or("");
                let preview: String = result.chars().take(200).collect();
                if !preview.is_empty() {
                    out.push(ChatMessage {
                        role: "tool".into(),
                        text: format!("✅ [{name}] {preview}"),
                        thinking: None,
                        show_thinking: false,
                        tool_blocks: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Parse `/fork [@N] [label…]` → (at, label).
pub(super) fn parse_fork_args(arg: &str) -> (Option<usize>, Option<String>) {
    let arg = arg.trim();
    if arg.is_empty() {
        return (None, None);
    }
    let mut parts = arg.split_whitespace();
    let first = parts.next().unwrap_or("");
    if let Some(rest) = first.strip_prefix('@') {
        if let Ok(n) = rest.parse::<usize>() {
            let label = {
                let joined: String = parts.collect::<Vec<_>>().join(" ");
                if joined.is_empty() {
                    None
                } else {
                    Some(joined)
                }
            };
            return (Some(n), label);
        }
    }
    (None, Some(arg.to_string()))
}

pub(super) fn extract_saved_file_path(tool_result: &str) -> Option<String> {
    for token in tool_result.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}'
            )
        });
        if cleaned.contains('/') || cleaned.contains('.') {
            let p = std::path::Path::new(cleaned);
            if p.is_file() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod fuzzy_tests {
    use crate::tui::App;

    #[test]
    fn fuzzy_matches_subsequence() {
        let items = vec![
            "opencode-cli/opencode/claude-sonnet-4-6".into(),
            "opencode-cli/opencode/deepseek-v4-flash-free".into(),
            "anthropic/claude-haiku-4-5".into(),
        ];
        let hit = App::rank_and_filter(&items, "sonnet", 10);
        assert!(hit.iter().any(|s| s.contains("sonnet")));
        let hit = App::rank_and_filter(&items, "ocs4", 10);
        assert!(
            hit.iter().any(|s| s.contains("claude-sonnet")),
            "expected subsequence match, got {:?}",
            hit
        );
        let hit = App::rank_and_filter(&items, "deep flash", 10);
        assert!(hit.iter().any(|s| s.contains("deepseek")));
    }

    #[test]
    fn fuzzy_prefers_prefix() {
        let items = vec!["claude-sonnet-4".into(), "x-claude-extra".into()];
        let hit = App::rank_and_filter(&items, "claude", 10);
        assert_eq!(hit[0], "claude-sonnet-4");
    }

    #[test]
    fn parse_fork_at_and_label() {
        assert_eq!(super::parse_fork_args(""), (None, None));
        assert_eq!(
            super::parse_fork_args("hotfix"),
            (None, Some("hotfix".into()))
        );
        assert_eq!(super::parse_fork_args("@3"), (Some(3), None));
        assert_eq!(
            super::parse_fork_args("@3 try again"),
            (Some(3), Some("try again".into()))
        );
    }
}
