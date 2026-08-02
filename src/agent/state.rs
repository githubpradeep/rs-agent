use crate::ai::token_count;
use crate::ai::types::*;
use crate::agent::mode::AgentMode;


#[derive(Debug, Clone)]
pub struct AgentState {
    pub system_prompt: String,
    pub model: String,
    pub provider: String,
    pub messages: Vec<Message>,
    pub thinking_budget: Option<u32>,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
    pub mode: AgentMode,
    /// Active skill tool allow-list (Skills 2.0). Empty = unrestricted.
    pub skill_tools: Vec<String>,
}

impl AgentState {
    pub fn new(model: String, provider: String) -> Self {
        Self {
            system_prompt: String::new(),
            model,
            provider,
            messages: Vec::new(),
            thinking_budget: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            mode: AgentMode::Agent,
            skill_tools: Vec::new(),
        }
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    pub fn with_thinking_budget(mut self, budget: Option<u32>) -> Self {
        self.thinking_budget = budget;
        self
    }

    pub fn with_mode(mut self, mode: AgentMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn set_mode(&mut self, mode: AgentMode) {
        self.mode = mode;
    }

    pub fn set_skill_tools(&mut self, tools: Vec<String>) {
        self.skill_tools = tools
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
    }

    pub fn clear_skill_tools(&mut self) {
        self.skill_tools.clear();
    }

    /// Mode + optional skill allow-list.
    pub fn allows_tool(&self, name: &str) -> bool {
        if !self.mode.allows_tool(name) {
            return false;
        }
        if self.skill_tools.is_empty() {
            return true;
        }
        let lower = name.to_lowercase();
        self.skill_tools.iter().any(|t| t == &lower || lower.starts_with(&format!("{t}__")))
    }

    pub fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn add_tool_result(&mut self, tool_use_id: String, tool_name: String, content: String, is_error: bool) {
        let msg = Message {
            role: Role::Tool,
            content: vec![Content {
                content_type: ContentType::ToolResult,
                text: Some(content),
                id: None,
                name: Some(tool_name),
                input: None,
                tool_use_id: Some(tool_use_id),
                content: None,
                signature: None,
                thinking: None,
                is_error,
            }],
        };
        self.messages.push(msg);
    }

    fn tool_uses_in(msg: &Message) -> Vec<(String, String)> {
        msg.content
            .iter()
            .filter(|c| c.content_type == ContentType::ToolUse)
            .filter_map(|c| {
                let id = c.id.as_ref()?.trim();
                if id.is_empty() {
                    return None;
                }
                Some((
                    id.to_string(),
                    c.name.clone().unwrap_or_else(|| "unknown".into()),
                ))
            })
            .collect()
    }

    fn tool_result_id(msg: &Message) -> Option<&str> {
        msg.content.iter().find_map(|c| {
            if c.content_type == ContentType::ToolResult {
                c.tool_use_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
            } else {
                None
            }
        })
    }

    fn interrupted_tool_result(id: String, name: String) -> Message {
        Message {
            role: Role::Tool,
            content: vec![Content {
                content_type: ContentType::ToolResult,
                text: Some("[Tool execution was interrupted]".to_string()),
                id: None,
                name: Some(name),
                input: None,
                tool_use_id: Some(id),
                content: None,
                signature: None,
                thinking: None,
                is_error: true,
            }],
        }
    }

    /// Make history valid for OpenAI-compat providers (DeepSeek / OpenCode Zen):
    /// - every `tool` message answers an open `tool_calls` id from the preceding assistant
    /// - tool results stay contiguous after that assistant (pull forward if a user/system
    ///   landed between them — e.g. harness nudges or compaction)
    /// - dangling tool_uses get synthetic interrupted results
    /// - orphan tool results (no matching call) are dropped
    ///
    /// Returns `(synthesized_results, dropped_orphans)`.
    pub fn repair_tool_pairing(&mut self) -> (usize, usize) {
        use std::collections::{HashMap, HashSet};

        let old = std::mem::take(&mut self.messages);
        let mut out: Vec<Message> = Vec::with_capacity(old.len());
        let mut pending: Vec<(String, String)> = Vec::new(); // (id, name) open tool_uses
        let mut synthesized = 0usize;
        let mut dropped = 0usize;
        let mut i = 0usize;

        while i < old.len() {
            let msg = &old[i];
            match msg.role {
                Role::Assistant => {
                    // Close any still-open calls before a new assistant turn.
                    for (id, name) in pending.drain(..) {
                        out.push(Self::interrupted_tool_result(id, name));
                        synthesized += 1;
                    }
                    pending = Self::tool_uses_in(msg);
                    out.push(msg.clone());
                    i += 1;

                    // Pull immediately-following tool results (and any that appear
                    // after an intervening non-assistant gap) forward while they
                    // answer `pending`.
                    if !pending.is_empty() {
                        let mut answered: HashSet<String> =
                            pending.iter().map(|(id, _)| id.clone()).collect();
                        // First consume contiguous tools.
                        while i < old.len() && old[i].role == Role::Tool {
                            if let Some(id) = Self::tool_result_id(&old[i]) {
                                if answered.contains(id) {
                                    answered.remove(id);
                                    pending.retain(|(p, _)| p != id);
                                    out.push(old[i].clone());
                                    i += 1;
                                    continue;
                                }
                            }
                            // Orphan or duplicate at this position — drop.
                            dropped += 1;
                            i += 1;
                        }
                        // If a User/System (not Assistant) sits between the assistant
                        // and remaining tool results, pull matching results forward.
                        if !pending.is_empty() && i < old.len() {
                            if matches!(old[i].role, Role::User | Role::System) {
                                let gap_start = i;
                                // Find how far the gap goes before next Assistant
                                // or end; scan for answering tools inside.
                                let mut j = i;
                                while j < old.len() && !matches!(old[j].role, Role::Assistant) {
                                    j += 1;
                                }
                                let mut pulled: HashMap<String, Message> = HashMap::new();
                                let mut keep_gap: Vec<Message> = Vec::new();
                                for k in gap_start..j {
                                    if old[k].role == Role::Tool {
                                        if let Some(id) = Self::tool_result_id(&old[k]) {
                                            if pending.iter().any(|(p, _)| p == id)
                                                && !pulled.contains_key(id)
                                            {
                                                pulled.insert(id.to_string(), old[k].clone());
                                                continue;
                                            }
                                        }
                                        dropped += 1;
                                        continue;
                                    }
                                    keep_gap.push(old[k].clone());
                                }
                                for (id, name) in pending.drain(..) {
                                    if let Some(m) = pulled.remove(&id) {
                                        out.push(m);
                                    } else {
                                        out.push(Self::interrupted_tool_result(id, name));
                                        synthesized += 1;
                                    }
                                }
                                out.extend(keep_gap);
                                i = j;
                            }
                        }
                        // Still-open after contiguous tools and no pullable gap.
                        for (id, name) in pending.drain(..) {
                            out.push(Self::interrupted_tool_result(id, name));
                            synthesized += 1;
                        }
                    }
                }
                Role::Tool => {
                    // Tool with no open assistant tool_calls — orphan.
                    dropped += 1;
                    i += 1;
                }
                _ => {
                    out.push(msg.clone());
                    i += 1;
                }
            }
        }

        for (id, name) in pending.drain(..) {
            out.push(Self::interrupted_tool_result(id, name));
            synthesized += 1;
        }

        self.messages = out;
        (synthesized, dropped)
    }

    /// OpenCode-style dangling tool settlement (subset of [`repair_tool_pairing`]).
    pub fn settle_dangling_tools(&mut self) -> usize {
        let (synthesized, _dropped) = self.repair_tool_pairing();
        synthesized
    }

    pub fn add_assistant(&mut self, msg: &AssistantMessage) {
        if let Some(ref usage) = msg.usage {
            self.total_input_tokens += usage.input_tokens as usize;
            self.total_output_tokens += usage.output_tokens as usize;
        }
        self.messages.push(Message {
            role: Role::Assistant,
            content: msg.content.clone(),
        });
    }

    pub fn estimated_context_tokens(&self, tool_defs_json: &str) -> usize {
        let sys = token_count::estimate_tokens(&self.system_prompt);
        let msgs = token_count::estimate_message_tokens(&self.messages);
        let tools = token_count::estimate_tokens(tool_defs_json);
        sys + msgs + tools + 20
    }

    pub fn context_limit(&self) -> usize {
        token_count::get_context_limit(&self.model)
    }

    pub fn context_usage_fraction(&self, tool_defs_json: &str) -> f64 {
        let used = self.estimated_context_tokens(tool_defs_json);
        let limit = self.context_limit();
        if limit == 0 {
            return 0.0;
        }
        (used as f64) / (limit as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_tools(calls: &[(&str, &str)]) -> AssistantMessage {
        AssistantMessage {
            content: calls
                .iter()
                .map(|(id, name)| Content {
                    content_type: ContentType::ToolUse,
                    id: Some((*id).into()),
                    name: Some((*name).into()),
                    input: Some(serde_json::json!({})),
                    ..Default::default()
                })
                .collect(),
            stop_reason: None,
            usage: None,
            model: "m".into(),
            id: None,
        }
    }

    #[test]
    fn settle_dangling_tools_injects_interrupted_results() {
        let mut state = AgentState::new("m".into(), "p".into());
        state.add_assistant(&assistant_tools(&[("call_1", "bash")]));
        assert_eq!(state.settle_dangling_tools(), 1);
        assert_eq!(state.settle_dangling_tools(), 0); // already settled
        let last = state.messages.last().unwrap();
        assert_eq!(last.role, Role::Tool);
        assert!(last.content[0]
            .text
            .as_deref()
            .unwrap_or("")
            .contains("interrupted"));
        assert!(last.content[0].is_error);
    }

    #[test]
    fn settle_skips_tools_that_already_have_results() {
        let mut state = AgentState::new("m".into(), "p".into());
        state.add_assistant(&assistant_tools(&[("call_1", "bash")]));
        state.add_tool_result("call_1".into(), "bash".into(), "ok".into(), false);
        assert_eq!(state.settle_dangling_tools(), 0);
    }

    #[test]
    fn repair_drops_orphan_tool_results() {
        let mut state = AgentState::new("m".into(), "p".into());
        state.add_message(Message {
            role: Role::User,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some("hi".into()),
                ..Default::default()
            }],
        });
        // Orphan tool result (compaction cut away its assistant tool_calls).
        state.add_tool_result("orphan".into(), "repl".into(), "leak".into(), false);
        state.add_message(Message {
            role: Role::User,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some("continue".into()),
                ..Default::default()
            }],
        });
        let (syn, drop) = state.repair_tool_pairing();
        assert_eq!(syn, 0);
        assert_eq!(drop, 1);
        assert_eq!(state.messages.len(), 2);
        assert!(state.messages.iter().all(|m| m.role != Role::Tool));
    }

    #[test]
    fn repair_pulls_tool_results_before_intervening_user() {
        let mut state = AgentState::new("m".into(), "p".into());
        state.add_assistant(&assistant_tools(&[("call_1", "repl")]));
        // Harness nudge landed between tool_calls and the result — OpenAI 400.
        state.add_message(Message {
            role: Role::User,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some("[harness] nudge".into()),
                ..Default::default()
            }],
        });
        state.add_tool_result("call_1".into(), "repl".into(), "ok".into(), false);

        let (syn, drop) = state.repair_tool_pairing();
        assert_eq!(syn, 0);
        assert_eq!(drop, 0);
        assert_eq!(state.messages[0].role, Role::Assistant);
        assert_eq!(state.messages[1].role, Role::Tool);
        assert_eq!(state.messages[2].role, Role::User);
        assert_eq!(
            state.messages[1].content[0].tool_use_id.as_deref(),
            Some("call_1")
        );
    }
}
