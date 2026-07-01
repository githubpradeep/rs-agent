use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Parallel
    }
    fn requires_permission(&self) -> bool {
        false
    }
    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
    ) -> ToolExecuteResult;
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

#[derive(Debug, Clone)]
pub struct ToolExecuteResult {
    pub content: String,
    pub is_error: bool,
    pub terminate: bool,
}

impl ToolExecuteResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            terminate: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            terminate: false,
        }
    }

    pub fn terminate(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            terminate: true,
        }
    }
}

pub type SharedTool = Arc<dyn AgentTool>;
