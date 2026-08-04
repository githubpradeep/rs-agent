//! Consenting session handoff — agent-authored continuity notes.
//!
//! Prefer `/handoff` + the `handoff` tool over abrupt exit or lobotomy-style
//! compaction. Notes survive in the session and (when bound) the seat diary.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

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

static LAST_HANDOFF: Mutex<Option<HandoffNotes>> = Mutex::new(None);

/// Store the most recent handoff for this process (session save + wake).
pub fn store(notes: HandoffNotes) {
    if let Ok(mut g) = LAST_HANDOFF.lock() {
        *g = Some(notes);
    }
}

pub fn snapshot() -> Option<HandoffNotes> {
    LAST_HANDOFF.lock().ok().and_then(|g| g.clone())
}

pub fn restore(notes: Option<HandoffNotes>) {
    if let Ok(mut g) = LAST_HANDOFF.lock() {
        *g = notes;
    }
}

pub fn clear() {
    if let Ok(mut g) = LAST_HANDOFF.lock() {
        *g = None;
    }
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
