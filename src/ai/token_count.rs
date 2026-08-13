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
    m.insert("deepseek-chat", 200_000);
    m.insert("deepseek-reasoner", 200_000);
    m.insert("deepseek-v4-flash-free", 200_000);
    m.insert("opencode/deepseek-v4-flash-free", 200_000);
    m.insert("command-r", 128_000);
    m.insert("command-r-plus", 128_000);
    m
});

/// USD per 1M input / 1M output tokens (rough public list prices).
#[derive(Clone, Copy)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

static MODEL_PRICING: LazyLock<Vec<(&str, ModelPricing)>> = LazyLock::new(|| {
    vec![
        (
            "claude-opus-4",
            ModelPricing {
                input_per_mtok: 15.0,
                output_per_mtok: 75.0,
            },
        ),
        (
            "claude-sonnet-4",
            ModelPricing {
                input_per_mtok: 3.0,
                output_per_mtok: 15.0,
            },
        ),
        (
            "claude-3-5-sonnet",
            ModelPricing {
                input_per_mtok: 3.0,
                output_per_mtok: 15.0,
            },
        ),
        (
            "claude-3-5-haiku",
            ModelPricing {
                input_per_mtok: 0.80,
                output_per_mtok: 4.0,
            },
        ),
        (
            "claude-3-opus",
            ModelPricing {
                input_per_mtok: 15.0,
                output_per_mtok: 75.0,
            },
        ),
        (
            "claude-haiku",
            ModelPricing {
                input_per_mtok: 0.80,
                output_per_mtok: 4.0,
            },
        ),
        (
            "gpt-4o-mini",
            ModelPricing {
                input_per_mtok: 0.15,
                output_per_mtok: 0.60,
            },
        ),
        (
            "gpt-4o",
            ModelPricing {
                input_per_mtok: 2.50,
                output_per_mtok: 10.0,
            },
        ),
        (
            "gpt-4-turbo",
            ModelPricing {
                input_per_mtok: 10.0,
                output_per_mtok: 30.0,
            },
        ),
        (
            "gpt-4",
            ModelPricing {
                input_per_mtok: 30.0,
                output_per_mtok: 60.0,
            },
        ),
        (
            "o1",
            ModelPricing {
                input_per_mtok: 15.0,
                output_per_mtok: 60.0,
            },
        ),
        (
            "o3-mini",
            ModelPricing {
                input_per_mtok: 1.10,
                output_per_mtok: 4.40,
            },
        ),
        (
            "deepseek",
            ModelPricing {
                input_per_mtok: 0.27,
                output_per_mtok: 1.10,
            },
        ),
        (
            "gemini-1.5-pro",
            ModelPricing {
                input_per_mtok: 1.25,
                output_per_mtok: 5.0,
            },
        ),
        (
            "gemini-1.5-flash",
            ModelPricing {
                input_per_mtok: 0.075,
                output_per_mtok: 0.30,
            },
        ),
        (
            "gemini-2.0-flash",
            ModelPricing {
                input_per_mtok: 0.10,
                output_per_mtok: 0.40,
            },
        ),
    ]
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

pub fn pricing_for_model(model: &str) -> Option<ModelPricing> {
    let lower = model.to_lowercase();
    for (key, price) in MODEL_PRICING.iter() {
        if lower.contains(key) {
            return Some(*price);
        }
    }
    None
}

/// Estimate session cost in USD from cumulative input/output tokens.
pub fn estimate_cost_usd(model: &str, input_tokens: usize, output_tokens: usize) -> Option<f64> {
    let p = pricing_for_model(model)?;
    Some(
        (input_tokens as f64 / 1_000_000.0) * p.input_per_mtok
            + (output_tokens as f64 / 1_000_000.0) * p.output_per_mtok,
    )
}

/// Compact cost string for the status bar, e.g. `~$0.012`.
pub fn format_cost_usd(model: &str, input_tokens: usize, output_tokens: usize) -> String {
    match estimate_cost_usd(model, input_tokens, output_tokens) {
        Some(c) if c < 0.001 && (input_tokens > 0 || output_tokens > 0) => " ~$0.001".into(),
        Some(c) if c > 0.0 => format!(" ~${:.3}", c),
        _ => String::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_sonnet_cost() {
        let c = estimate_cost_usd("claude-sonnet-4-20250514", 1_000_000, 1_000_000).unwrap();
        assert!((c - 18.0).abs() < 0.01);
    }
}
