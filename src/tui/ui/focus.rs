//! Zone-local focus model — keys mean different things per zone.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusZone {
    #[default]
    Chat,
    Sessions,
    Tree,
}

impl FocusZone {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Sessions => "sessions",
            Self::Tree => "tree",
        }
    }
}

/// Cycle focus among `visible` zones (forward if `dir > 0`).
pub fn cycle_focus(current: FocusZone, visible: &[FocusZone], dir: i32) -> FocusZone {
    if visible.is_empty() {
        return current;
    }
    let idx = visible.iter().position(|&z| z == current).unwrap_or(0) as i32;
    let n = visible.len() as i32;
    let next = (idx + dir).rem_euclid(n) as usize;
    visible[next]
}
