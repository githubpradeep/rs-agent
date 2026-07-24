pub mod control;
pub mod mode;
pub mod r#loop;
pub mod tool;
pub mod state;
pub mod registry;

pub use control::*;
pub use mode::*;
pub use r#loop::*;
pub use tool::*;
pub use state::*;
pub use registry::*;

pub fn default_system_prompt() -> String {
    crate::prompts::load_system_prompt()
}
