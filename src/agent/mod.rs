pub mod compact_pins;
pub mod control;
pub mod goal;
pub mod handoff;
pub mod laurel;
pub mod r#loop;
pub mod mode;
pub mod registry;
pub mod repair;
pub mod rlm_escalate;
pub mod seat;
pub mod state;
pub mod tool;
pub mod wake;

pub use control::*;
pub use goal::{parse_goal_arg, GoalCommand, GoalState, GoalStatus};
pub use handoff::{
    handoff_request_message, peek_routing, route_to_seat, take_routing, HandoffNotes,
    RoutingHandoffRecord,
};
pub use mode::*;
pub use r#loop::*;
pub use registry::*;
pub use repair::{is_weak_model, weak_model_system_note, weak_model_user_warning};
pub use rlm_escalate::{escalate_chars, set_escalate_chars};
pub use seat::{parse_seat_arg, SeatCaste, SeatCommand, SeatProfile};
pub use state::*;
pub use tool::*;

pub fn default_system_prompt() -> String {
    crate::prompts::load_system_prompt()
}
