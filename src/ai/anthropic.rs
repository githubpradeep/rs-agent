use crate::ai::provider::{BoxStream, Provider};
use crate::ai::types::*;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use std::time::Duration;

pub struct AnthropicProvider {
    pub base_url: String,
    pub name: String,
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com/v1".to_string(),
            name: "anthropic".to_string(),
        }
    }
}

impl AnthropicProvider {
    pub fn new(base_url: Option<String>, name: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com/v1".to_string()),
            name: name.unwrap_or_else(|| "anthropic".to_string()),
        }
    }
}

fn convert_to_anthropic_messages(messages: &[Message], system: &Option<String>) -> (Option<String>, Vec<serde_json::Value>) {
    let mut system_text = system.clone();
    let mut anthropic_messages = Vec::new();

    for msg in messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "user",
            Role::System => {
                if let Some(c) = msg.content.first() {
                    system_text = Some(format!("{}\n{}", system_text.as_deref().unwrap_or(""), c.text.as_deref().unwrap_or("")));
                }
                continue;
            }
        };

        let mut content = Vec::new();
        for c in &msg.content {
            match c.content_type {
                ContentType::Text => {
                    content.push(serde_json::json!({
                        "type": "text",
                        "text": c.text.as_deref().unwrap_or("")
                    }));
                }
                ContentType::ToolUse => {
                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": c.id.as_deref().unwrap_or(""),
                        "name": c.name.as_deref().unwrap_or(""),
                        "input": c.input.as_ref().unwrap_or(&serde_json::Value::Null)
                    }));
                }
                ContentType::ToolResult => {
                    content.push(serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": c.tool_use_id.as_deref().unwrap_or(""),
                        "content": c.text.as_deref().unwrap_or(""),
                        "is_error": false
                    }));
                }
                ContentType::Thinking | ContentType::RedactedThinking => {
                    if let Some(t) = &c.thinking {
                        content.push(serde_json::json!({
                            "type": "thinking",
                            "thinking": t
                        }));
                    }
                    if let Some(s) = &c.signature {
                        content.push(serde_json::json!({
                            "type": "signature",
                            "signature": s
                        }));
                    }
                }
            }
        }

        if let Role::Tool = msg.role {
            if let Some(c) = msg.content.first() {
                anthropic_messages.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": c.tool_use_id.as_deref().unwrap_or(""),
                        "content": c.text.as_deref().unwrap_or("")
                    }]
                }));
                continue;
            }
        }

        if content.is_empty() {
            continue;
        }

        anthropic_messages.push(serde_json::json!({
            "role": role,
            "content": content
        }));
    }

    (system_text, anthropic_messages)
}

fn convert_tools_to_anthropic(tools: &[ToolDef]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })
        })
        .collect()
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn api_key_env_var(&self) -> &str {
        "ANTHROPIC_API_KEY"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn chat(&self, api_key: &str, request: ChatRequest) -> ProviderResult<AssistantMessage> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let (system_text, anthropic_messages) = convert_to_anthropic_messages(&request.messages, &request.system);

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": anthropic_messages,
            "max_tokens": request.max_tokens,
        });

        if let Some(s) = system_text {
            body["system"] = serde_json::json!(s);
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::json!(convert_tools_to_anthropic(&request.tools));
        }
        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(top_p) = request.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
        if let Some(thinking) = &request.thinking {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": thinking.budget_tokens
            });
        }

        let resp = client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout
                } else {
                    ProviderError::Other(e.to_string())
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(60.0);
            let text = resp.text().await.unwrap_or_default();
            return Err(if status.as_u16() == 429 {
                ProviderError::RateLimited(retry_after)
            } else if status.as_u16() == 401 {
                ProviderError::Auth(text)
            } else {
                ProviderError::Http(status.as_u16(), text)
            });
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        parse_anthropic_response(data)
    }

    async fn chat_stream(
        &self,
        api_key: &str,
        request: ChatRequest,
    ) -> ProviderResult<BoxStream> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let (system_text, anthropic_messages) = convert_to_anthropic_messages(&request.messages, &request.system);

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": anthropic_messages,
            "max_tokens": request.max_tokens,
            "stream": true
        });

        if let Some(s) = system_text {
            body["system"] = serde_json::json!(s);
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::json!(convert_tools_to_anthropic(&request.tools));
        }
        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(top_p) = request.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
        if let Some(thinking) = &request.thinking {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": thinking.budget_tokens
            });
        }

        let response = client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout
                } else {
                    ProviderError::Other(e.to_string())
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(60.0);
            let text = response.text().await.unwrap_or_default();
            return Err(if status.as_u16() == 429 {
                ProviderError::RateLimited(retry_after)
            } else if status.as_u16() == 401 {
                ProviderError::Auth(text)
            } else {
                ProviderError::Http(status.as_u16(), text)
            });
        }

        let stream = response.bytes_stream().flat_map(move |chunk| {
            let result = match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut deltas = Vec::new();
                    for line in text.lines() {
                        if let Some(delta) = parse_anthropic_stream_event(line) {
                            deltas.push(delta);
                        }
                    }
                    Ok(deltas)
                }
                Err(e) => Err(ProviderError::Stream(e.to_string())),
            };
            futures::stream::iter(match result {
                Ok(deltas) => deltas.into_iter().map(Ok).collect::<Vec<_>>(),
                Err(e) => vec![Err(e)],
            })
        });

        let boxed: BoxStream = Box::pin(stream);
        Ok(boxed)
    }

    fn supports_thinking(&self) -> bool {
        true
    }

    fn default_max_tokens(&self) -> u32 {
        8192
    }

    async fn fetch_models(&self, api_key: &str) -> ProviderResult<Vec<String>> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let resp = client
            .get(format!("{}/models", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let models = data["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }
}

fn parse_anthropic_response(data: serde_json::Value) -> ProviderResult<AssistantMessage> {
    let mut content = Vec::new();

    if let Some(blocks) = data["content"].as_array() {
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => {
                    content.push(Content {
                        content_type: ContentType::Text,
                        text: block["text"].as_str().map(|s| s.to_string()),
                        id: None,
                        name: None,
                        input: None,
                        tool_use_id: None,
                        content: None,
                        signature: None,
                        thinking: None,
                    });
                }
                Some("tool_use") => {
                    content.push(Content {
                        content_type: ContentType::ToolUse,
                        text: None,
                        id: block["id"].as_str().map(|s| s.to_string()),
                        name: block["name"].as_str().map(|s| s.to_string()),
                        input: Some(block["input"].clone()),
                        tool_use_id: None,
                        content: None,
                        signature: None,
                        thinking: None,
                    });
                }
                Some("thinking") => {
                    content.push(Content {
                        content_type: ContentType::Thinking,
                        text: None,
                        id: None,
                        name: None,
                        input: None,
                        tool_use_id: None,
                        content: None,
                        signature: block["signature"].as_str().map(|s| s.to_string()),
                        thinking: block["thinking"].as_str().map(|s| s.to_string()),
                    });
                }
                _ => {}
            }
        }
    }

    let stop_reason = data["stop_reason"].as_str().and_then(|r| match r {
        "end_turn" => Some(StopReason::EndTurn),
        "tool_use" => Some(StopReason::ToolUse),
        "max_tokens" => Some(StopReason::MaxTokens),
        "stop_sequence" => Some(StopReason::StopSequence),
        _ => Some(StopReason::Other(r.to_string())),
    });

    let usage = data["usage"].as_object().map(|u| Usage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
        cache_read_input_tokens: u["cache_read_input_tokens"]
            .as_u64()
            .map(|v| v as u32),
        cache_creation_input_tokens: u["cache_creation_input_tokens"]
            .as_u64()
            .map(|v| v as u32),
    });

    Ok(AssistantMessage {
        content,
        stop_reason,
        usage,
        model: data["model"].as_str().unwrap_or("unknown").to_string(),
        id: data["id"].as_str().map(|s| s.to_string()),
    })
}

fn parse_anthropic_stream_event(line: &str) -> Option<StreamDelta> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    if line.starts_with("event: ") {
        return None;
    }

    if line.starts_with("data: ") {
        let data = line.strip_prefix("data: ")?;
        let value: serde_json::Value = serde_json::from_str(data).ok()?;

        return match value["type"].as_str() {
            Some("content_block_delta") => {
                let index = value["index"].as_u64().unwrap_or(0) as u32;
                match value["delta"]["type"].as_str() {
                    Some("text_delta") => Some(StreamDelta {
                        content_index: index,
                        r#type: DeltaType::Text {
                            text: value["delta"]["text"].as_str().unwrap_or("").to_string(),
                        },
                    }),
                    Some("thinking_delta") => Some(StreamDelta {
                        content_index: index,
                        r#type: DeltaType::Thinking {
                            thinking: value["delta"]["thinking"].as_str().unwrap_or("").to_string(),
                        },
                    }),
                    Some("signature_delta") => Some(StreamDelta {
                        content_index: index,
                        r#type: DeltaType::Signature {
                            signature: value["delta"]["signature"].as_str().unwrap_or("").to_string(),
                        },
                    }),
                    Some("input_json_delta") => Some(StreamDelta {
                        content_index: index,
                        r#type: DeltaType::ToolCallDelta {
                            input: value["delta"]["partial_json"].as_str().unwrap_or("").to_string(),
                        },
                    }),
                    _ => None,
                }
            }
            Some("content_block_start") => {
                let index = value["index"].as_u64().unwrap_or(0) as u32;
                match value["content_block"]["type"].as_str() {
                    Some("tool_use") => Some(StreamDelta {
                        content_index: index,
                        r#type: DeltaType::ToolCallStart {
                            id: value["content_block"]["id"].as_str().unwrap_or("").to_string(),
                            name: value["content_block"]["name"].as_str().unwrap_or("").to_string(),
                            input: String::new(),
                        },
                    }),
                    _ => None,
                }
            }
            Some("message_delta") => {
                let stop_reason = value["delta"]["stop_reason"].as_str().and_then(|r| match r {
                    "end_turn" => Some(StopReason::EndTurn),
                    "tool_use" => Some(StopReason::ToolUse),
                    "max_tokens" => Some(StopReason::MaxTokens),
                    "stop_sequence" => Some(StopReason::StopSequence),
                    r => Some(StopReason::Other(r.to_string())),
                });
                Some(StreamDelta {
                    content_index: 0,
                    r#type: DeltaType::Stop { stop_reason },
                })
            }
            _ => None,
        };
    }

    None
}
