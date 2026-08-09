//! City cockpit ops helpers (UI rethink — overview + inspector + composers).

use super::super::fleet_panel::CityPanelState;
use super::super::ui::FocusZone;
use crate::fleet::{self, FleetUpOpts};

/// Build seat list for spawn counts.
pub fn seats_for_spawn(fleet_n: usize, crew_n: usize) -> Vec<String> {
    let mut seats = Vec::new();
    for i in 1..=fleet_n.min(16) {
        seats.push(format!("Fleet-{i}"));
    }
    for i in 1..=crew_n.min(8) {
        seats.push(format!("Crew-{i}"));
    }
    seats
}

pub fn parse_count(s: &str, default: usize) -> usize {
    s.parse::<usize>().unwrap_or(default)
}

pub fn fleet_up_opts(
    seats: Vec<String>,
    provider: Option<String>,
    model: Option<String>,
) -> FleetUpOpts {
    FleetUpOpts {
        seats,
        budget_minutes: 480,
        sleep_secs: 5,
        quiet: false,
        provider,
        model,
        approve: true,
        fail_fast: false,
    }
}

pub fn cycle_city_focus(panel: &CityPanelState, current: FocusZone, dir: i32) -> FocusZone {
    let visible = super::super::ui::visible_city_zones(panel.has_selection());
    super::super::ui::cycle_focus(current, &visible, dir)
}

pub fn ensure_city_focus(current: FocusZone) -> FocusZone {
    if current.is_city() {
        current
    } else {
        FocusZone::CityWish
    }
}

/// Divert follow logs into panel when City is showing that seat.
pub fn should_divert_logs(
    show_city: bool,
    selected: Option<&str>,
    attach_seat: Option<&str>,
) -> bool {
    show_city
        && selected.is_some()
        && attach_seat.is_some()
        && selected == attach_seat
}
