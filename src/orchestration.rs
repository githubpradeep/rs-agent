//! Conductor-inspired orchestration primitives (retries, timeouts, outcomes, JOIN).

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Soft-fail (may retry) vs hard-fail (do not auto-retry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailKind {
    Retriable,
    Terminal,
}

impl FailKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retriable => "retriable",
            Self::Terminal => "terminal",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "terminal" | "fatal" | "permanent" => Self::Terminal,
            _ => Self::Retriable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailOutcome {
    pub kind: FailKind,
    pub reason: String,
}

impl FailOutcome {
    pub fn retriable(reason: impl Into<String>) -> Self {
        Self {
            kind: FailKind::Retriable,
            reason: reason.into(),
        }
    }

    pub fn terminal(reason: impl Into<String>) -> Self {
        Self {
            kind: FailKind::Terminal,
            reason: reason.into(),
        }
    }

    /// Heuristic: safety / escalate / policy → terminal.
    pub fn classify(reason: &str) -> Self {
        let lower = reason.to_lowercase();
        if lower.contains("escalate")
            || lower.contains("policy")
            || lower.contains("denied")
            || lower.contains("safety")
            || lower.contains("terminal")
            || lower.contains("awaiting_human")
        {
            Self::terminal(reason)
        } else {
            Self::retriable(reason)
        }
    }
}

/// Dual timeout: worker silent vs wall-clock budget (Conductor TaskDef).
#[derive(Debug, Clone, Copy)]
pub struct DualTimeout {
    /// No heartbeat / response within this → treat as stuck (retriable reclaim).
    pub response: Duration,
    /// Absolute wall budget for the unit of work.
    pub wall: Duration,
}

impl DualTimeout {
    pub fn from_secs(response_secs: u64, wall_secs: u64) -> Self {
        Self {
            response: Duration::from_secs(response_secs.max(1)),
            wall: Duration::from_secs(wall_secs.max(1)),
        }
    }

    pub fn response_exceeded(self, since_heartbeat: Duration) -> bool {
        since_heartbeat >= self.response
    }

    pub fn wall_exceeded(self, since_start: Duration) -> bool {
        since_start >= self.wall
    }
}

/// Exponential backoff with jitter (Conductor RetryLogic).
pub fn backoff_delay(attempt: u32, base_ms: u64, max_ms: u64, jitter_ms: u64) -> Duration {
    let exp = base_ms.saturating_mul(1u64 << attempt.min(16));
    let capped = exp.min(max_ms);
    let jitter = if jitter_ms == 0 {
        0
    } else {
        // Cheap deterministic jitter from attempt (avoid pulling rand).
        (attempt as u64).wrapping_mul(1103515245).wrapping_add(12345) % (jitter_ms + 1)
    };
    Duration::from_millis(capped.saturating_add(jitter))
}

/// Compact JOIN merge for parallel child summaries (parent sees only summaries).
pub fn join_summaries(children: &[(String, String)], max_each: usize, max_total: usize) -> String {
    let mut out = String::new();
    for (i, (name, summary)) in children.iter().enumerate() {
        if out.len() >= max_total {
            out.push_str("\n… (truncated)");
            break;
        }
        let body: String = summary.chars().take(max_each).collect();
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("[{name}] {body}"));
    }
    out
}

/// Tiny safe condition AST for goal termination (no code exec).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GoalCond {
    /// Substring present in transcript / status text (case-insensitive).
    Contains { text: String },
    /// All children must hold.
    And { children: Vec<GoalCond> },
    /// Any child holds.
    Or { children: Vec<GoalCond> },
    /// Negation.
    Not { child: Box<GoalCond> },
    /// Always true (stop).
    True,
    /// Always false.
    False,
}

impl GoalCond {
    pub fn eval(&self, haystack: &str) -> bool {
        let hay = haystack.to_lowercase();
        match self {
            Self::Contains { text } => hay.contains(&text.to_lowercase()),
            Self::And { children } => children.iter().all(|c| c.eval(&hay)),
            Self::Or { children } => children.iter().any(|c| c.eval(&hay)),
            Self::Not { child } => !child.eval(&hay),
            Self::True => true,
            Self::False => false,
        }
    }

    /// Parse a tiny DSL: `contains:foo`, `and:(a|b)`, `or:(a|b)`, `not:a`, `true`, `false`.
    /// Nested and/or use `|` separators inside parens for leaf `contains:` terms only.
    pub fn parse_dsl(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("true") {
            return Some(Self::True);
        }
        if s.eq_ignore_ascii_case("false") {
            return Some(Self::False);
        }
        if let Some(rest) = s.strip_prefix("contains:") {
            return Some(Self::Contains {
                text: rest.trim().to_string(),
            });
        }
        if let Some(rest) = s.strip_prefix("not:") {
            let inner = Self::parse_dsl(rest)?;
            return Some(Self::Not {
                child: Box::new(inner),
            });
        }
        if let Some(rest) = s.strip_prefix("and:(").and_then(|r| r.strip_suffix(')')) {
            let children: Vec<_> = rest
                .split('|')
                .filter_map(|p| Self::parse_dsl(p.trim()))
                .collect();
            if children.is_empty() {
                return None;
            }
            return Some(Self::And { children });
        }
        if let Some(rest) = s.strip_prefix("or:(").and_then(|r| r.strip_suffix(')')) {
            let children: Vec<_> = rest
                .split('|')
                .filter_map(|p| Self::parse_dsl(p.trim()))
                .collect();
            if children.is_empty() {
                return None;
            }
            return Some(Self::Or { children });
        }
        // Bare text → contains
        if !s.is_empty() {
            return Some(Self::Contains {
                text: s.to_string(),
            });
        }
        None
    }
}

/// Routing handoff (Conductor HandoffConfig) — distinct from continuity notes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingHandoff {
    pub from_seat: Option<String>,
    pub to_seat: String,
    pub reason: String,
    pub allowed: bool,
}

impl RoutingHandoff {
    pub fn try_route(
        from: Option<&str>,
        to: &str,
        reason: &str,
        allowed_transitions: &[String],
    ) -> Self {
        let allowed = allowed_transitions.is_empty()
            || allowed_transitions.iter().any(|a| {
                a == "*"
                    || a == to
                    || from
                        .map(|f| a == &format!("{f}->{to}") || a == &format!("*->{to}"))
                        .unwrap_or(false)
            });
        Self {
            from_seat: from.map(|s| s.to_string()),
            to_seat: to.to_string(),
            reason: reason.to_string(),
            allowed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let d0 = backoff_delay(0, 100, 1000, 0);
        let d3 = backoff_delay(3, 100, 1000, 0);
        assert!(d3 > d0);
        let d20 = backoff_delay(20, 100, 1000, 0);
        assert_eq!(d20, Duration::from_millis(1000));
    }

    #[test]
    fn goal_cond_dsl() {
        let c = GoalCond::parse_dsl("and:(contains:pass|contains:green)").unwrap();
        assert!(c.eval("tests pass and green"));
        assert!(!c.eval("fail"));
    }

    #[test]
    fn join_truncates() {
        let kids = vec![
            ("a".into(), "hello world".into()),
            ("b".into(), "second".into()),
        ];
        let s = join_summaries(&kids, 5, 100);
        assert!(s.contains("[a] hello"));
    }
}
