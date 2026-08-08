//! Optional sink so tools can stream output into the TUI mid-execution.
//!
//! Thread-local so each session runtime (true parallel) has its own sink.

use crate::agent::AgentEvent;
use crossbeam_channel as channel;
use std::cell::RefCell;

thread_local! {
    static SINK: RefCell<Option<channel::Sender<AgentEvent>>> = const { RefCell::new(None) };
}

/// Wire this OS thread's tool-output sink (call once at session-runtime start).
pub fn set_tool_output_sink(tx: channel::Sender<AgentEvent>) {
    SINK.with(|s| {
        *s.borrow_mut() = Some(tx);
    });
}

pub fn emit_tool_output(name: &str, stream: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    SINK.with(|s| {
        if let Some(tx) = s.borrow().as_ref() {
            let _ = tx.send(AgentEvent::ToolOutput {
                name: name.to_string(),
                stream: stream.to_string(),
                text: text.to_string(),
            });
        }
    });
}
