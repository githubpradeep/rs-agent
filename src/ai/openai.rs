use crate::ai::provider::{BoxStream, Provider};
use crate::ai::types::*;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use std::time::Duration;

pub struct OpenAIProvider {
    pub base_url: String,
    pub name: String,
    pub key_env: String,
}

impl Default for OpenAIProvider {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            name: "openai".to_string(),
            key_env: "OPENAI_API_KEY".to_string(),
        }
    }
}

impl OpenAIProvider {
    pub fn new(base_url: Option<String>, name: Option<String>, key_env: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            name: name.unwrap_or_else(|| "openai".to_string()),
            key_env: key_env.unwrap_or_else(|| "OPENAI_API_KEY".to_string()),
        }
    }
}

fn convert_message(msg: &Message) -> serde_json::Value {
    let role_str = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    match msg.role {
        Role::Tool => {
            let c = &msg.content[0];
            return serde_json::json!({
                "role": "tool",
                "tool_call_id": c.tool_use_id.as_deref().unwrap_or(""),
                "content": c.text.as_deref().unwrap_or("")
            });
        }
        Role::Assistant => {
            let text_parts: Vec<&str> = msg.content.iter()
                .filter(|c| c.content_type == ContentType::Text)
                .filter_map(|c| c.text.as_deref())
                .collect();
            let text = text_parts.join("");

            let tool_calls: Vec<serde_json::Value> = msg.content.iter()
                .filter(|c| c.content_type == ContentType::ToolUse)
                .map(|c| {
                    serde_json::json!({
                        "id": c.id.as_deref().unwrap_or(""),
                        "type": "function",
                        "function": {
                            "name": c.name.as_deref().unwrap_or(""),
                            "arguments": serde_json::to_string(&c.input.as_ref().unwrap_or(&serde_json::Value::Object(serde_json::Map::new()))).unwrap_or_default()
                        }
                    })
                })
                .collect();

            let mut result = serde_json::json!({"role": "assistant", "content": serde_json::Value::Null});
            if !text.is_empty() {
                result["content"] = serde_json::json!(text);
            }
            if !tool_calls.is_empty() {
                result["tool_calls"] = serde_json::json!(tool_calls);
            }
            return result;
        }
        _ => {
            let text = msg.content.iter()
                .filter(|c| c.content_type == ContentType::Text)
                .filter_map(|c| c.text.as_deref())
                .collect::<Vec<_>>()
                .join("");
            return serde_json::json!({"role": role_str, "content": text});
        }
    }
}

fn convert_tools(tools: &[ToolDef]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema
                }
            })
        })
        .collect()
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn api_key_env_var(&self) -> &str {
        &self.key_env
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn chat(&self, api_key: &str, request: ChatRequest) -> ProviderResult<AssistantMessage> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": request.messages.iter().map(convert_message).collect::<Vec<_>>(),
            "max_tokens": request.max_tokens,
            "stream": false
        });

        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(top_p) = request.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::json!(convert_tools(&request.tools));
        }
        if let Some(system) = &request.system {
            body["messages"]
                .as_array_mut()
                .unwrap()
                .insert(0, serde_json::json!({"role": "system", "content": system}));
        }

        let resp = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
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

        parse_openai_response(data)
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

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": request.messages.iter().map(convert_message).collect::<Vec<_>>(),
            "max_tokens": request.max_tokens,
            "stream": true,
            "stream_options": {"include_usage": true}
        });

        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(top_p) = request.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::json!(convert_tools(&request.tools));
        }
        if let Some(system) = &request.system {
            body["messages"]
                .as_array_mut()
                .unwrap()
                .insert(0, serde_json::json!({"role": "system", "content": system}));
        }

        let response = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
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
                        if let Some(delta) = parse_openai_stream_line(line) {
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
        self.name.contains("o") || self.name.contains("reasoning")
    }

    fn default_max_tokens(&self) -> u32 {
        8192
    }

    async fn fetch_models(&self, api_key: &str) -> ProviderResult<Vec<String>> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let resp = client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
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

fn parse_openai_response(data: serde_json::Value) -> ProviderResult<AssistantMessage> {
    let choice = data["choices"][0]
        .as_object()
        .ok_or_else(|| ProviderError::Parse("no choices".to_string()))?;

    let finish_reason = choice["finish_reason"].as_str().and_then(|r| match r {
        "stop" => Some(StopReason::EndTurn),
        "tool_calls" => Some(StopReason::ToolUse),
        "length" => Some(StopReason::MaxTokens),
        _ => Some(StopReason::Other(r.to_string())),
    });

    let message = &choice["message"];
    let mut content = Vec::new();

    if let Some(text) = message["content"].as_str() {
        if !text.is_empty() {
            content.push(Content {
                content_type: ContentType::Text,
                text: Some(text.to_string()),
                id: None,
                name: None,
                input: None,
                tool_use_id: None,
                content: None,
                signature: None,
                thinking: None,
            });
        }
    }

    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let arguments = get_arguments(&tc["function"]["arguments"]).unwrap_or_else(|| "{}".to_string());
            let input: serde_json::Value =
                serde_json::from_str(&arguments).unwrap_or(serde_json::Value::Null);

            content.push(Content {
                content_type: ContentType::ToolUse,
                text: None,
                id: Some(id),
                name: Some(name),
                input: Some(input),
                tool_use_id: None,
                content: None,
                signature: None,
                thinking: None,
            });
        }
    }

    let usage = data["usage"].as_object().map(|u| Usage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
        cache_read_input_tokens: u["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .map(|v| v as u32),
        cache_creation_input_tokens: None,
    });

    Ok(AssistantMessage {
        content,
        stop_reason: finish_reason,
        usage,
        model: data["model"].as_str().unwrap_or("unknown").to_string(),
        id: data["id"].as_str().map(|s| s.to_string()),
    })
}

fn get_arguments(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => {
            if s.is_empty() { None } else { Some(s.clone()) }
        }
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            Some(serde_json::to_string(value).unwrap_or_default())
        }
        _ => None,
    }
}

fn parse_openai_stream_line(line: &str) -> Option<StreamDelta> {
    let line = line.trim();
    if line.is_empty() || !line.starts_with("data: ") {
        return None;
    }

    let data = line.strip_prefix("data: ")?;
    if data == "[DONE]" {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let choices = value["choices"].as_array()?;
    let choice = choices.first()?;

    if choice["finish_reason"].as_str().is_some_and(|r| !r.is_empty()) {
        let reason = match choice["finish_reason"].as_str() {
            Some("stop") => Some(StopReason::EndTurn),
            Some("tool_calls") => Some(StopReason::ToolUse),
            Some("length") => Some(StopReason::MaxTokens),
            Some(r) => Some(StopReason::Other(r.to_string())),
            _ => None,
        };
        return Some(StreamDelta {
            content_index: 0,
            r#type: DeltaType::Stop {
                stop_reason: reason,
            },
        });
    }

    let delta = &choice["delta"];

    if let Some(tool_calls) = delta["tool_calls"].as_array() {
        if let Some(tc) = tool_calls.first() {
            let index = tc["index"].as_u64().unwrap_or(0) as u32;
            if tc["id"].as_str().is_some() {
                return Some(StreamDelta {
                    content_index: index,
                    r#type: DeltaType::ToolCallStart {
                        id: tc["id"].as_str().unwrap_or("").to_string(),
                        name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                        input: get_arguments(&tc["function"]["arguments"]).unwrap_or_default(),
                    },
                });
            }
            if let Some(args) = get_arguments(&tc["function"]["arguments"]) {
                if !args.is_empty() {
                    return Some(StreamDelta {
                        content_index: index,
                        r#type: DeltaType::ToolCallDelta {
                            input: args,
                        },
                    });
                }
            }
        }
    }

    if let Some(text) = delta["content"].as_str() {
        if !text.is_empty() {
            return Some(StreamDelta {
                content_index: 0,
                r#type: DeltaType::Text {
                    text: text.to_string(),
                },
            });
        }
    }

    None
}
