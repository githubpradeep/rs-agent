//! Auto-escalate to RLM when tool output / files are too large for chat.

use std::sync::OnceLock;

pub const DEFAULT_ESCALATE_CHARS: usize = 10_000;
pub const PREVIEW_CHARS: usize = 4_000;
pub const RLM_ESCALATE_MARKER: &str = "[rlm_escalate]";

static THRESHOLD: OnceLock<usize> = OnceLock::new();

/// Set from config/CLI at process start (idempotent first-write wins).
pub fn set_escalate_chars(n: usize) {
    let _ = THRESHOLD.set(n.max(4_000));
}

pub fn escalate_chars() -> usize {
    *THRESHOLD.get_or_init(|| {
        std::env::var("RS_AGENT_RLM_ESCALATE_CHARS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_ESCALATE_CHARS)
    })
}

pub fn escalate_enabled() -> bool {
    !matches!(
        std::env::var("RS_AGENT_RLM_ESCALATE")
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

/// One-shot system note after an escalate marker appears in a tool result.
pub fn system_note() -> &'static str {
    "DEEP CONTEXT ESCALATE (harness): A tool result marked [rlm_escalate] is too large for chat. \
     Do NOT re-read or paste the full file. Use the repl tool (Deep Context): \
     `context = load_file('…')` (or load_dir), peek/search in Python, then \
     llm_query on slices, and FINAL(value). Prefer sub-calls over stuffing the window."
}

pub fn format_escalate_footer(file_path: &str, total_chars: usize) -> String {
    format!(
        "{RLM_ESCALATE_MARKER}\n\
         file_path: {file_path}\n\
         chars: {total_chars}\n\
         hint: Do not re-read this into chat. Use repl:\n\
           context = load_file('{file_path}')\n\
           # peek/search in Python, then llm_query(slice); FINAL(...)"
    )
}

/// If content is huge, return preview + escalate footer; else return unchanged.
pub fn maybe_wrap_huge_output(file_path: &str, body: String, total_chars: usize) -> (String, bool) {
    if !escalate_enabled() {
        return (body, false);
    }
    let threshold = escalate_chars();
    let body_chars = body.chars().count();
    let should = body_chars > threshold || (total_chars > threshold && body_chars > PREVIEW_CHARS);
    if !should {
        return (body, false);
    }
    let preview: String = body.chars().take(PREVIEW_CHARS).collect();
    let out = format!(
        "{preview}\n… (preview only; {} chars in this slice, {} in file)\n\n{}",
        body_chars,
        total_chars,
        format_escalate_footer(file_path, total_chars)
    );
    (out, true)
}

/// Append escalate hint when loop truncates a large tool result.
pub fn append_truncate_escalate_hint(name: &str, truncated: &str, original_len: usize) -> String {
    if !escalate_enabled() || original_len <= escalate_chars() {
        return truncated.to_string();
    }
    if truncated.contains(RLM_ESCALATE_MARKER) {
        return truncated.to_string();
    }
    format!(
        "{truncated}\n\n{RLM_ESCALATE_MARKER}\n\
         tool: {name}\n\
         chars: {original_len}\n\
         hint: Output was truncated for chat. For large corpora use repl + load_file/load_dir \
         + llm_query; do not re-fetch the full dump into the conversation."
    )
}

pub fn content_has_escalate(s: &str) -> bool {
    s.contains(RLM_ESCALATE_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_when_over_threshold() {
        // Use a high explicit body vs low threshold via wrap logic directly
        let big = "x".repeat(PREVIEW_CHARS + 100);
        let (out, _escalated) =
            maybe_wrap_huge_output("/tmp/big.md", big.clone(), big.len() + 10_000);
        // Default threshold 10k — this body is ~4100, file total huge with body > PREVIEW
        // total_chars > threshold (if default 10k) — 4100+10000 = 14100, may not escalate
        // Force with large body:
        let big2 = "y".repeat(escalate_chars() + 10);
        let (out2, esc2) = maybe_wrap_huge_output("/tmp/big.md", big2, escalate_chars() + 10);
        assert!(esc2);
        assert!(out2.contains(RLM_ESCALATE_MARKER));
        assert!(out2.contains("load_file"));
        let _ = out;
    }

    #[test]
    fn footer_contains_path() {
        let f = format_escalate_footer("/abs/corpus.md", 99999);
        assert!(f.contains("/abs/corpus.md"));
        assert!(f.contains("99999"));
    }
}
