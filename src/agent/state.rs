use crate::ai::types::*;


#[derive(Debug, Clone)]
pub struct AgentState {
    pub system_prompt: String,
    pub model: String,
    pub provider: String,
    pub messages: Vec<Message>,
    pub thinking_budget: Option<u32>,
}

impl AgentState {
    pub fn new(model: String, provider: String) -> Self {
        Self {
            system_prompt: String::new(),
            model,
            provider,
            messages: Vec::new(),
            thinking_budget: None,
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
        self.messages.push(Message {
            role: Role::Assistant,
            content: msg.content.clone(),
        });
    }
}
