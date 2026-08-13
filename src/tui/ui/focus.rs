//! Zone-local focus model — keys mean different things per zone.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusZone {
    #[default]
    Chat,
    CityWish,
    CityBoard,
    CityInspector,
    CitySteer,
    CitySpawn,
    Sessions,
    Tree,
}

impl FocusZone {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::CityWish => "wish",
            Self::CityBoard => "city",
            Self::CityInspector => "inspect",
            Self::CitySteer => "steer",
            Self::CitySpawn => "spawn",
            Self::Sessions => "sessions",
            Self::Tree => "tree",
        }
    }

    pub fn is_city(self) -> bool {
        matches!(
            self,
            Self::CityWish
                | Self::CityBoard
                | Self::CityInspector
                | Self::CitySteer
                | Self::CitySpawn
        )
    }
}

/// City zones in Tab cycle order when City is open.
pub fn visible_city_zones(has_selection: bool) -> Vec<FocusZone> {
    let mut z = vec![FocusZone::CityWish, FocusZone::CityBoard];
    if has_selection {
        z.push(FocusZone::CityInspector);
        z.push(FocusZone::CitySteer);
    }
    z.push(FocusZone::CitySpawn);
    z
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
