use crate::ai::types::*;
use async_trait::async_trait;

pub type StreamResult = Result<StreamDelta, ProviderError>;
pub type BoxStream = std::pin::Pin<Box<dyn futures::Stream<Item = StreamResult> + Send>>;

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn api_key_env_var(&self) -> &str;
    fn base_url(&self) -> &str;

    async fn chat(
        &self,
        api_key: &str,
        request: ChatRequest,
    ) -> ProviderResult<AssistantMessage>;

    async fn chat_stream(
        &self,
        api_key: &str,
        request: ChatRequest,
    ) -> ProviderResult<BoxStream>;

    async fn fetch_models(&self, _api_key: &str) -> ProviderResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn supports_thinking(&self) -> bool {
        false
    }

    fn default_max_tokens(&self) -> u32 {
        4096
    }
}
