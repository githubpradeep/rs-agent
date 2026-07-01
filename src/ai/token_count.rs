use crate::ai::types::*;
use std::collections::HashMap;
use std::sync::LazyLock;

pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    (text.len() / 3).max(text.split_whitespace().count())
}

pub fn estimate_message_tokens(msgs: &[Message]) -> usize {
    msgs.iter().map(|m| estimate_message(m)).sum()
}

pub fn estimate_message(msg: &Message) -> usize {
    let mut total = 4;
    for content in &msg.content {
        if let Some(ref text) = content.text {
            total += estimate_tokens(text);
        }
        if let Some(ref thinking) = content.thinking {
            total += estimate_tokens(thinking);
        }
        if let Some(ref name) = content.name {
            total += name.len() / 3;
        }
        if let Some(ref id) = content.id {
            total += id.len() / 3;
        }
    }
    total += 4;
    total
}

pub fn estimate_tool_def_tokens(defs: &[ToolDef]) -> usize {
    let json = serde_json::to_string(defs).unwrap_or_default();
    estimate_tokens(&json)
}

static MODEL_LIMITS: LazyLock<HashMap<&str, usize>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("gpt-4o", 128_000);
    m.insert("gpt-4o-mini", 128_000);
    m.insert("gpt-4-turbo", 128_000);
    m.insert("gpt-4", 8192);
    m.insert("gpt-3.5-turbo", 16385);
    m.insert("claude-sonnet-4-20250514", 200_000);
    m.insert("claude-sonnet-4", 200_000);
    m.insert("claude-opus-4-8", 200_000);
    m.insert("claude-opus-4", 200_000);
    m.insert("claude-3-5-sonnet", 200_000);
    m.insert("claude-3-5-haiku", 200_000);
    m.insert("claude-3-opus", 200_000);
    m.insert("claude-3-sonnet", 200_000);
    m.insert("us.anthropic.claude-opus-4-8", 200_000);
    m.insert("us.anthropic.claude-sonnet-4-20250514", 200_000);
    m.insert("gemini-1.5-pro", 1_048_576);
    m.insert("gemini-1.5-flash", 1_048_576);
    m.insert("gemini-2.0-flash", 1_048_576);
    m.insert("deepseek-chat", 128_000);
    m.insert("deepseek-reasoner", 128_000);
    m.insert("command-r", 128_000);
    m.insert("command-r-plus", 128_000);
    m
});

pub fn get_context_limit(model: &str) -> usize {
    let lower = model.to_lowercase();
    if let Some(&limit) = MODEL_LIMITS.get(lower.as_str()) {
        return limit;
    }
    for (key, &limit) in MODEL_LIMITS.iter() {
        if lower.contains(key) {
            return limit;
        }
    }
    128_000
}

pub const SAFETY_MARGIN: usize = 4000;

pub fn would_exceed_limit(model: &str, estimated_total: usize) -> bool {
    let limit = get_context_limit(model);
    estimated_total + SAFETY_MARGIN > limit
}

pub fn usage_fraction(model: &str, estimated_total: usize) -> f64 {
    let limit = get_context_limit(model);
    if limit == 0 {
        return 0.0;
    }
    (estimated_total as f64) / (limit as f64)
}
