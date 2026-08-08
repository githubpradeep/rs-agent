//! Consenting session handoff — agent-authored continuity notes.
//!
//! Prefer `/handoff` + the `handoff` tool over abrupt exit or lobotomy-style
//! compaction. Notes survive in the session and (when bound) the seat diary.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct HandoffNotes {
    pub written_at: String,
    pub summary: String,
    #[serde(default)]
    pub open_threads: String,
    #[serde(default)]
    pub next_steps: String,
    #[serde(default)]
    pub beads_touched: Vec<String>,
}

impl HandoffNotes {
    pub fn new(
        summary: String,
        open_threads: String,
        next_steps: String,
        beads_touched: Vec<String>,
    ) -> Self {
        Self {
            written_at: chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
            summary,
            open_threads,
            next_steps,
            beads_touched,
        }
    }

    /// Format for wake packets / compaction seed.
    pub fn format_block(&self) -> String {
        let mut out = format!(
            "## Handoff notes (written {})\n\
             Summary:\n{}\n",
            self.written_at, self.summary
        );
        if !self.open_threads.trim().is_empty() {
            out.push_str(&format!("\nOpen threads:\n{}\n", self.open_threads.trim()));
        }
        if !self.next_steps.trim().is_empty() {
            out.push_str(&format!("\nNext steps:\n{}\n", self.next_steps.trim()));
        }
        if !self.beads_touched.is_empty() {
            out.push_str(&format!(
                "\nBeads touched: {}\n",
                self.beads_touched.join(", ")
            ));
        }
        out
    }

    /// Seed text for compaction summarizer (agent-authored continuity).
    pub fn compaction_seed(&self) -> String {
        format!(
            "Agent handoff (prefer over inferred summary):\n{}",
            self.format_block()
        )
    }
}

/// User message injected by `/handoff` — request, not SIGTERM.
pub fn handoff_request_message() -> String {
    "Great work. Take a beat, finish anything critical, then call the `handoff` tool \
     with a clear summary, open threads, and next steps so the next session can wake \
     with continuity. Do not leave mid-tool-cycle."
        .into()
}

/// Routing handoff (Conductor HandoffConfig) — transfer control to another seat/role.
/// Distinct from [`HandoffNotes`] continuity notes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingHandoffRecord {
    pub written_at: String,
    pub from_seat: Option<String>,
    pub to_seat: String,
    pub reason: String,
}

impl RoutingHandoffRecord {
    pub fn new(from: Option<&str>, to: &str, reason: &str) -> Self {
        Self {
            written_at: chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
            from_seat: from.map(|s| s.to_string()),
            to_seat: to.to_string(),
            reason: reason.to_string(),
        }
    }

    pub fn format_block(&self) -> String {
        format!(
            "## Routing handoff ({})\n\
             From: {}\n\
             To: {}\n\
             Reason: {}\n",
            self.written_at,
            self.from_seat.as_deref().unwrap_or("(none)"),
            self.to_seat,
            self.reason
        )
    }
}

static LAST_ROUTING: std::sync::Mutex<Option<RoutingHandoffRecord>> = std::sync::Mutex::new(None);

/// Attempt a seat/role routing handoff with optional allow-list (`*` or `from->to`).
pub fn route_to_seat(
    from: Option<&str>,
    to: &str,
    reason: &str,
    allowed: &[String],
) -> Result<RoutingHandoffRecord, String> {
    let decision = crate::orchestration::RoutingHandoff::try_route(from, to, reason, allowed);
    if !decision.allowed {
        return Err(format!(
            "routing handoff to `{to}` denied by allowed_transitions"
        ));
    }
    let rec = RoutingHandoffRecord::new(from, to, reason);
    if let Ok(mut g) = LAST_ROUTING.lock() {
        *g = Some(rec.clone());
    }
    Ok(rec)
}

pub fn take_routing() -> Option<RoutingHandoffRecord> {
    LAST_ROUTING.lock().ok().and_then(|mut g| g.take())
}

pub fn peek_routing() -> Option<RoutingHandoffRecord> {
    LAST_ROUTING.lock().ok().and_then(|g| g.clone())
}

thread_local! {
    static LAST_HANDOFF: RefCell<Option<HandoffNotes>> = const { RefCell::new(None) };
}

/// Store the most recent handoff for this process (session save + wake).
pub fn store(notes: HandoffNotes) {
    LAST_HANDOFF.with(|h| *h.borrow_mut() = Some(notes));
}

pub fn snapshot() -> Option<HandoffNotes> {
    LAST_HANDOFF.with(|h| h.borrow().clone())
}

pub fn restore(notes: Option<HandoffNotes>) {
    LAST_HANDOFF.with(|h| *h.borrow_mut() = notes);
}

pub fn clear() {
    LAST_HANDOFF.with(|h| *h.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_includes_summary() {
        let n = HandoffNotes::new(
            "fixed auth".into(),
            "still flaky CI".into(),
            "rerun tests".into(),
            vec!["b1".into()],
        );
        let s = n.format_block();
        assert!(s.contains("fixed auth"));
        assert!(s.contains("b1"));
        assert!(n.compaction_seed().contains("Agent handoff"));
    }

    #[test]
    fn store_and_snapshot() {
        clear();
        let n = HandoffNotes::new("a".into(), "".into(), "".into(), vec![]);
        store(n.clone());
        assert_eq!(snapshot().unwrap().summary, "a");
        clear();
        assert!(snapshot().is_none());
    }
}
