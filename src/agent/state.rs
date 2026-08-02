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

    /// OpenCode-style dangling tool settlement: every `tool_use` must have a
    /// matching `tool_result` before the next LLM call (Anthropic/DeepSeek reject
    /// unpaired tool_use). Inject synthetic interrupted/error results for orphans.
    pub fn settle_dangling_tools(&mut self) -> usize {
        use std::collections::HashSet;
        let mut pending: Vec<(String, String)> = Vec::new();
        let mut answered: HashSet<String> = HashSet::new();

        for msg in &self.messages {
            match msg.role {
                Role::Assistant => {
                    for c in &msg.content {
                        if c.content_type == ContentType::ToolUse {
                            if let Some(id) = c.id.as_ref() {
                                pending.push((
                                    id.clone(),
                                    c.name.clone().unwrap_or_else(|| "unknown".into()),
                                ));
                            }
                        }
                    }
                }
                Role::Tool => {
                    for c in &msg.content {
                        if c.content_type == ContentType::ToolResult {
                            if let Some(id) = c.tool_use_id.as_ref() {
                                answered.insert(id.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let mut settled = 0usize;
        for (id, name) in pending {
            if answered.contains(&id) {
                continue;
            }
            self.add_tool_result(
                id,
                name,
                "[Tool execution was interrupted]".to_string(),
                true,
            );
            settled += 1;
        }
        settled
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

    #[test]
    fn settle_dangling_tools_injects_interrupted_results() {
        let mut state = AgentState::new("m".into(), "p".into());
        state.add_assistant(&AssistantMessage {
            content: vec![
                Content {
                    content_type: ContentType::Text,
                    text: Some("calling".into()),
                    ..Default::default()
                },
                Content {
                    content_type: ContentType::ToolUse,
                    id: Some("call_1".into()),
                    name: Some("bash".into()),
                    input: Some(serde_json::json!({"command": "ls"})),
                    ..Default::default()
                },
            ],
            stop_reason: None,
            usage: None,
            model: "m".into(),
            id: None,
        });
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
        state.add_assistant(&AssistantMessage {
            content: vec![Content {
                content_type: ContentType::ToolUse,
                id: Some("call_1".into()),
                name: Some("bash".into()),
                input: Some(serde_json::json!({})),
                ..Default::default()
            }],
            stop_reason: None,
            usage: None,
            model: "m".into(),
            id: None,
        });
        state.add_tool_result("call_1".into(), "bash".into(), "ok".into(), false);
        assert_eq!(state.settle_dangling_tools(), 0);
    }
}
