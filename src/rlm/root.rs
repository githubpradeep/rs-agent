//! Root RLM turn: the model sees a stub, the input lives in `context`.

use crate::ai::types::{Content, ContentType, Message, Role};
use serde_json::Value;

pub const RLM_SYSTEM: &str = include_str!("../../prompts/rlm.md");
/// Chars of stdout/stderr fed back into the next cell. The root must not
/// receive the whole REPL namespace.
pub const MODEL_OBS_CAP: usize = 2_000;
pub const HISTORY_CAP: usize = 12_000;
/// Requests shorter than this are shown to the model. Longer ones stay only in `context`.
pub const SHOW_REQUEST_MAX: usize = 8_000;
pub const ANSWER_CAP: usize = 50_000;
pub const DEFAULT_MAX_CELLS: usize = 64;

pub struct RootOutcome {
    pub answer: String,
    /// `false` when `answer` was already streamed (a prose fallback).
    pub append_to_stream: bool,
}

/// What the root model sees for one turn. Prior replies are included.
/// A long new request stays only in the Python variable `context`.
pub fn root_user_turn(context: &str, history: &str) -> String {
    let mut out = String::new();
    if !history.trim().is_empty() {
        out.push_str(
            "Prior conversation. Continue from this. A follow-up refers to these turns, not to a new task:\n",
        );
        out.push_str(history.trim());
        out.push_str("\n\nNames bound in earlier cells of this process are still in the REPL.\n\n");
    }
    let chars = context.chars().count();
    if chars <= SHOW_REQUEST_MAX {
        out.push_str("New request (also in the Python variable `context`):\n");
        out.push_str(context);
        out.push('\n');
    } else {
        out.push_str(&format!(
            "The new request is in the Python variable `context` ({chars} characters). It is not included above. Read it from Python.\n"
        ));
    }
    out.push_str(
        "\nWrite one ```python cell that does the new request. Use the prior conversation when the request refers to it. \
         FINAL is the reply in your own words. Do not FINAL(context) or a raw directory listing. \
         There is no tool-call API.",
    );
    out
}

pub fn nudge_message() -> &'static str {
    "Nothing ran. `context` is the user's request, not text in this chat. \
     There is no tool-call API. Reply with one ```python cell that does the request \
     and FINAL's your reply. Do not FINAL(context)."
}

pub fn echo_nudge() -> &'static str {
    "FINAL repeated the user's request. That is not a reply. \
     Read `context`, do what it asks, and FINAL the text the user should read."
}

pub fn dump_nudge() -> &'static str {
    "FINAL was a raw directory listing or file dump. That is not a reply. \
     Use what you read, then FINAL the answer in your own words."
}

/// True when the model handed the request back unchanged.
pub fn final_echoes_request(context: &str, answer: &str) -> bool {
    let request = context.trim();
    let reply = answer.trim();
    !request.is_empty() && reply == request
}

/// True when FINAL is an `ls` dump and the user did not ask for a listing.
pub fn final_is_unanswered_dump(context: &str, answer: &str) -> bool {
    !request_wants_listing(context) && listing_lines(answer) >= 8
}

fn request_wants_listing(context: &str) -> bool {
    let c = context.to_lowercase();
    [
        "list the files",
        "list files",
        "directory listing",
        "show the files",
        "ls the",
        "run ls",
    ]
    .iter()
    .any(|p| c.contains(p))
}

fn listing_lines(answer: &str) -> usize {
    answer
        .lines()
        .map(str::trim)
        .filter(|line| {
            let b = line.as_bytes();
            b.len() > 12
                && b[0].is_ascii_digit()
                && b[1].is_ascii_digit()
                && b[2].is_ascii_digit()
                && b[3].is_ascii_digit()
                && b.get(4) == Some(&b'-')
        })
        .count()
}

pub fn bind_history_code(history: &str) -> String {
    // A JSON string literal is valid Python. `json.loads("")` is not: the
    // quotes are Python syntax, so the decoder receives an empty string.
    let lit = serde_json::to_string(history).unwrap_or_else(|_| "\"\"".to_string());
    format!("history = {lit}")
}

pub fn clip_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}\n…[truncated {count} chars]")
}

pub fn final_answer(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn cell_feedback(stdout: &str, stderr: &str, ok: bool) -> String {
    if !ok {
        return format!(
            "The cell raised:\n{}\nFix it with another ```python cell. Results already bound stay in variables.",
            clip_chars(stderr, MODEL_OBS_CAP)
        );
    }
    if stdout.trim().is_empty() {
        "Cell finished with no stdout. Values are in variables. \
         Call FINAL(value) or FINAL_VAR(name) when the user-visible answer is ready."
            .to_string()
    } else {
        format!(
            "Cell finished. stdout preview:\n{}\n\
             Call FINAL or FINAL_VAR with the reply the user should read. Do not FINAL(context). \
             Leave long text in variables.",
            clip_chars(stdout, MODEL_OBS_CAP)
        )
    }
}

/// Fenced ```python / ```py cells. A bare fence is accepted when the body looks like Python.
pub fn extract_python(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let Some(nl) = after.find('\n') else {
            break;
        };
        let lang = after[..nl].trim().to_ascii_lowercase();
        let body = &after[nl + 1..];
        let (code, closed) = if let Some(end) = body.find("```") {
            (body[..end].trim(), true)
        } else {
            (body.trim(), false)
        };
        let lang_ok = lang.is_empty() || lang == "py" || lang.starts_with("python");
        if lang_ok
            && !code.is_empty()
            && (lang == "py" || lang.starts_with("python") || looks_like_python(code))
        {
            out.push(code.to_string());
        }
        if !closed {
            break;
        }
        let end = body.find("```").unwrap_or(body.len());
        rest = body.get(end + 3..).unwrap_or("");
    }
    out
}

fn looks_like_python(code: &str) -> bool {
    let t = code.trim_start();
    const STARTS: &[&str] = &[
        "import ",
        "from ",
        "def ",
        "class ",
        "for ",
        "while ",
        "if ",
        "with ",
        "try:",
        "try ",
        "print(",
        "FINAL",
        "context",
        "history",
        "read_file",
        "grep(",
        "bash(",
        "edit(",
        "write(",
        "ls(",
        "find(",
        "bead(",
        "llm_query",
        "agent_query",
        "answer",
    ];
    STARTS.iter().any(|s| t.starts_with(s)) || t.contains("FINAL(") || t.contains("FINAL_VAR(")
}

pub fn last_message_text(messages: &[Message]) -> String {
    messages
        .last()
        .map(|m| {
            m.content
                .iter()
                .filter_map(|c| c.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Short projection of earlier turns. The current user text stays out when
/// `skip_trailing_user` is set — that text is bound to `context` instead.
pub fn history_from_messages(messages: &[Message], skip_trailing_user: bool) -> String {
    let end = if skip_trailing_user
        && messages
            .last()
            .map(|m| m.role == Role::User)
            .unwrap_or(false)
    {
        messages.len().saturating_sub(1)
    } else {
        messages.len()
    };
    let slice = &messages[..end];
    let start = slice.len().saturating_sub(8);
    let mut parts = Vec::new();
    for m in &slice[start..] {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::System => "system",
        };
        let text: String = m
            .content
            .iter()
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            continue;
        }
        parts.push(format!("{role}: {}", clip_chars(&text, 2_000)));
    }
    clip_chars(&parts.join("\n"), HISTORY_CAP)
}

pub fn text_message(role: Role, text: &str) -> Message {
    Message {
        role,
        content: vec![Content {
            content_type: ContentType::Text,
            text: Some(text.to_string()),
            ..Default::default()
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEEDLE: &str = "NEEDLE-context-body-9f3a";

    #[test]
    fn turn_shows_prior_conversation_and_hides_a_long_request() {
        let prior = "assistant: The factory runs beads overnight.";
        let follow = root_user_turn("change the second paragraph", prior);
        assert!(follow.contains("The factory runs beads overnight."));
        assert!(follow.contains("change the second paragraph"));
        assert!(follow.contains("Prior conversation"));
        assert!(follow.contains("Do not FINAL(context)"));

        let huge = format!("{NEEDLE}{}", "x".repeat(SHOW_REQUEST_MAX));
        let hidden = root_user_turn(&huge, prior);
        assert!(!hidden.contains(NEEDLE));
        assert!(hidden.contains("The factory runs beads overnight."));
        assert!(hidden.contains(&huge.chars().count().to_string()));
        assert!(!RLM_SYSTEM.contains(NEEDLE));
        assert!(RLM_SYSTEM.contains("Prior conversation"));
        assert!(RLM_SYSTEM.contains("FINAL_VAR"));
        assert!(RLM_SYSTEM.contains("Do not call FINAL(context)"));
        assert!(!RLM_SYSTEM.contains("tool_use"));
    }

    #[test]
    fn listing_dump_is_not_an_answer() {
        let dump = "\
CWD: /tmp/rs-agent
LS:
2026-08-06 16:49:12      1KB LICENSE
2026-08-06 16:49:12     79KB Cargo.lock
2026-08-06 16:49:12 .github/
2026-08-08 05:29:07 skills/
2026-08-08 05:31:57 .rs-agent/
2026-08-08 11:00:37 example/
2026-08-13 03:48:13 .cursor/
2026-08-13 04:05:46      1KB Cargo.toml
2026-10-07 10:01:12     11KB README.md
2026-10-10 04:43:14 src/";
        assert!(final_is_unanswered_dump(
            "do you notice any gaps in implementatin or issues",
            dump
        ));
        assert!(!final_is_unanswered_dump("list the files", dump));
        assert!(!final_is_unanswered_dump(
            "how does the architecture work",
            "The TUI lives in src/tui and the agent loop lives in src/agent."
        ));
    }

    #[test]
    fn echo_is_the_request_itself() {
        assert!(final_echoes_request(
            "explain this projetc",
            "explain this projetc"
        ));
        assert!(final_echoes_request(
            "explain this projetc",
            "  explain this projetc\n"
        ));
        assert!(!final_echoes_request(
            "explain this projetc",
            "This repo is a Rust coding agent."
        ));
        assert!(!final_echoes_request("", ""));
        assert!(!final_echoes_request("explain this projetc", ""));
    }

    #[test]
    fn history_skips_the_current_user_text() {
        let messages = vec![
            text_message(Role::Assistant, "earlier answer"),
            text_message(Role::User, NEEDLE),
        ];
        let history = history_from_messages(&messages, true);
        assert!(!history.contains(NEEDLE));
        assert!(history.contains("earlier answer"));
        let bound = bind_history_code(&history);
        assert!(!bound.contains(NEEDLE));
        assert!(!bound.contains("json.loads"));
    }

    #[test]
    fn bind_history_empty_is_a_python_string() {
        assert_eq!(bind_history_code(""), "history = \"\"");
        let quoted = bind_history_code("say \"hi\"\nnext");
        assert_eq!(quoted, "history = \"say \\\"hi\\\"\\nnext\"");
        assert_eq!(quoted.lines().count(), 1);
    }

    #[test]
    fn extract_python_takes_fenced_cells_only() {
        let fenced = "I will look.\n```python\nprint(context[:20])\nFINAL('ok')\n```\n";
        let blocks = extract_python(fenced);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("FINAL"));
        assert!(extract_python("The answer is 4 because 2+2.").is_empty());
        let bare = "```\nFINAL(1)\n```";
        assert_eq!(extract_python(bare).len(), 1);
    }
}
