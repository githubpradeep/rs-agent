pub mod control;
pub mod mode;
pub mod r#loop;
pub mod compact_pins;
pub mod rlm_escalate;
pub mod repair;
pub mod tool;
pub mod state;
pub mod registry;

pub use control::*;
pub use mode::*;
pub use r#loop::*;
pub use repair::{is_weak_model, weak_model_system_note, weak_model_user_warning};
pub use rlm_escalate::{escalate_chars, set_escalate_chars};
pub use tool::*;
pub use state::*;
pub use registry::*;

pub fn default_system_prompt() -> String {
    crate::prompts::load_system_prompt()
}
