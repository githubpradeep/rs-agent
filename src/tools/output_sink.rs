//! Optional sink so tools can stream output into the TUI mid-execution.

use crate::agent::AgentEvent;
use crossbeam_channel as channel;
use std::sync::OnceLock;

static SINK: OnceLock<channel::Sender<AgentEvent>> = OnceLock::new();

/// Wire the agent event sink for progressive tool output (first call wins).
pub fn set_tool_output_sink(tx: channel::Sender<AgentEvent>) {
    let _ = SINK.set(tx);
}

pub fn emit_tool_output(name: &str, stream: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(tx) = SINK.get() {
        let _ = tx.send(AgentEvent::ToolOutput {
            name: name.to_string(),
            stream: stream.to_string(),
            text: text.to_string(),
        });
    }
}
