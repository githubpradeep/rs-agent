use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_input_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Content {
    #[serde(rename = "type")]
    pub content_type: ContentType,
    pub text: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<serde_json::Value>,
    pub tool_use_id: Option<String>,
    pub content: Option<Vec<Content>>,
    pub signature: Option<String>,
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    #[default]
    Text,
    ToolUse,
    ToolResult,
    Thinking,
    RedactedThinking,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Content>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    Error,
    ContentFiltered,
    Other(String),
}

#[derive(Debug, Clone)]
pub struct AssistantMessage {
    pub content: Vec<Content>,
    pub stop_reason: Option<StopReason>,
    pub usage: Option<Usage>,
    pub model: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StreamDelta {
    pub content_index: u32,
    pub r#type: DeltaType,
}

#[derive(Debug, Clone)]
pub enum DeltaType {
    Text { text: String },
    Thinking { thinking: String },
    Signature { signature: String },
    ToolCallStart { id: String, name: String, input: String },
    ToolCallDelta { input: String },
    Stop { stop_reason: Option<StopReason> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub system: Option<String>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
    pub stream: bool,
    pub thinking: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub r#type: String,
    pub budget_tokens: u32,
}

#[derive(Debug, Clone)]
pub enum ProviderError {
    Http(u16, String),
    Auth(String),
    RateLimited(f64),
    Timeout,
    Parse(String),
    Stream(String),
    Other(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;
