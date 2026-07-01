use crate::ai::token_count;
use crate::ai::types::*;


#[derive(Debug, Clone)]
pub struct AgentState {
    pub system_prompt: String,
    pub model: String,
    pub provider: String,
    pub messages: Vec<Message>,
    pub thinking_budget: Option<u32>,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
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
        }
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    pub fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn add_tool_result(&mut self, tool_use_id: String, tool_name: String, content: String, _is_error: bool) {
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
            }],
        };
        self.messages.push(msg);
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
