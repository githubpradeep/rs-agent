//! Wake packet — purpose on startup, not amnesia.
//!
//! Assembled from seat identity, last handoff, goal, open beads, and laurels.

use crate::agent::goal::GoalState;
use crate::agent::handoff::HandoffNotes;
use crate::agent::laurel::{self, Laurel};
use crate::agent::seat::SeatProfile;
use crate::beads;

#[derive(Debug, Clone, Default)]
pub struct WakeInputs {
    pub seat: Option<SeatProfile>,
    pub handoff: Option<HandoffNotes>,
    pub goal: Option<GoalState>,
    pub open_beads_limit: usize,
    pub laurels_limit: usize,
    /// Extra laurels (e.g. from seat) merged with project log.
    pub extra_laurels: Vec<Laurel>,
}

impl WakeInputs {
    pub fn from_parts(
        seat: Option<SeatProfile>,
        handoff: Option<HandoffNotes>,
        goal: Option<GoalState>,
    ) -> Self {
        let extra_laurels = seat
            .as_ref()
            .map(|s| s.laurels.iter().rev().take(5).cloned().collect())
            .unwrap_or_default();
        // Prefer seat diary handoff if session handoff missing.
        let handoff = handoff.or_else(|| seat.as_ref().and_then(|s| s.last_handoff().cloned()));
        Self {
            seat,
            handoff,
            goal,
            open_beads_limit: 12,
            laurels_limit: 5,
            extra_laurels,
        }
    }
}

/// Build the system-note wake block. Returns `None` if nothing to inject.
pub fn build(inputs: &WakeInputs) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(ref seat) = inputs.seat {
        parts.push(seat.wake_identity_block());
    }

    if let Some(ref h) = inputs.handoff {
        parts.push(h.format_block());
    }

    if let Some(ref g) = inputs.goal {
        if let Some(note) = g.system_note() {
            parts.push(format!(
                "## Goal on wake\nStatus: {}\nCondition: {}\n",
                g.status.as_str(),
                g.condition
            ));
            let _ = note;
        }
    }

    if let Some(block) = crate::brain::format_wake_block(8, 4_000) {
        parts.push(block);
    }

    if let Some(block) = beads::format_wake_block(inputs.open_beads_limit) {
        parts.push(block);
    }

    let mut laurels = laurel::recent(inputs.laurels_limit);
    for l in &inputs.extra_laurels {
        if !laurels
            .iter()
            .any(|x| x.text == l.text && x.written_at == l.written_at)
        {
            laurels.push(l.clone());
        }
    }
    if laurels.len() > inputs.laurels_limit {
        let skip = laurels.len() - inputs.laurels_limit;
        laurels = laurels.split_off(skip);
    }
    if let Some(block) = laurel::format_wake_block(&laurels) {
        parts.push(block);
    }

    if parts.is_empty() {
        return None;
    }

    let mut out = String::from(
        "# Wake packet\n\
         You are waking into this session with purpose. Use the notes below; \
         do not rediscover from scratch.\n\n",
    );
    out.push_str(&parts.join("\n"));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::goal::GoalState;

    #[test]
    fn empty_wake_is_none() {
        // Project cwd may have beads/brain; isolate so Default stays empty.
        crate::with_temp_cwd(|_| {
            assert!(build(&WakeInputs::default()).is_none());
        });
    }

    #[test]
    fn wake_with_handoff() {
        let notes = HandoffNotes::new(
            "shipped patch".into(),
            "CI flake".into(),
            "watch green".into(),
            vec![],
        );
        let packet = build(&WakeInputs {
            handoff: Some(notes),
            ..Default::default()
        })
        .unwrap();
        assert!(packet.contains("Wake packet"));
        assert!(packet.contains("shipped patch"));
    }

    #[test]
    fn wake_with_goal() {
        let goal = GoalState::new("tests pass".into(), 0, 0);
        let packet = build(&WakeInputs {
            goal: Some(goal),
            ..Default::default()
        })
        .unwrap();
        assert!(packet.contains("tests pass"));
    }
}
