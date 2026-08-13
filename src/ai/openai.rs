use crate::ai::provider::{BoxStream, Provider};
use crate::ai::sse::sse_delta_stream;
use crate::ai::types::*;
use async_trait::async_trait;
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

#[derive(Clone, Copy, Default)]
struct ConvertOpts {
    /// DeepSeek thinking-mode: always include reasoning_content (even "").
    force_reasoning_content: bool,
}

fn needs_reasoning_echo(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("deepseek") || m.contains("reasoner") || m.contains("r1")
}

fn convert_message_with_opts(msg: &Message, opts: ConvertOpts) -> serde_json::Value {
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
            let text_parts: Vec<&str> = msg
                .content
                .iter()
                .filter(|c| c.content_type == ContentType::Text)
                .filter_map(|c| c.text.as_deref())
                .collect();
            let text = text_parts.join("");

            // Preserve thinking-mode reasoning for providers (DeepSeek via OpenCode,
            // etc.) that require reasoning_content to be echoed on later turns.
            let reasoning: String = msg
                .content
                .iter()
                .filter(|c| c.content_type == ContentType::Thinking)
                .filter_map(|c| c.thinking.as_deref())
                .collect::<Vec<_>>()
                .join("");

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

            // DeepSeek rejects assistant messages where neither content nor
            // tool_calls is set (reasoning_content alone is not enough). Never
            // emit content:null unless tool_calls are present.
            let mut result = serde_json::json!({"role": "assistant"});
            if !text.is_empty() {
                result["content"] = serde_json::json!(text);
            } else if tool_calls.is_empty() {
                result["content"] = serde_json::json!("");
            } else {
                result["content"] = serde_json::Value::Null;
            }
            if !tool_calls.is_empty() {
                result["tool_calls"] = serde_json::json!(tool_calls);
            }
            // OpenCode ProviderTransform: always echo reasoning_content for
            // DeepSeek-class models — empty string still must be present.
            if !reasoning.is_empty() || opts.force_reasoning_content {
                result["reasoning_content"] = serde_json::json!(reasoning);
            }
            return result;
        }
        _ => {
            let text = msg
                .content
                .iter()
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

/// Map a thinking token budget to OpenAI `reasoning_effort`.
fn reasoning_effort_from_budget(budget: u32) -> &'static str {
    if budget < 5_000 {
        "low"
    } else if budget < 15_000 {
        "medium"
    } else {
        "high"
    }
}

/// Apply thinking / temperature fields shared by chat and chat_stream bodies.
fn apply_openai_sampling(body: &mut serde_json::Value, request: &ChatRequest) {
    if let Some(thinking) = &request.thinking {
        // Reasoning models reject temperature; use effort instead.
        body["reasoning_effort"] =
            serde_json::json!(reasoning_effort_from_budget(thinking.budget_tokens));
    } else if let Some(temp) = request.temperature {
        body["temperature"] = serde_json::json!(temp);
    }
    if request.thinking.is_none() {
        if let Some(top_p) = request.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
    }
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

        let opts = ConvertOpts {
            force_reasoning_content: needs_reasoning_echo(&request.model)
                || request.thinking.is_some(),
        };
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": request.messages.iter().map(|m| convert_message_with_opts(m, opts)).collect::<Vec<_>>(),
            "max_tokens": request.max_tokens,
            "stream": false
        });

        apply_openai_sampling(&mut body, &request);
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

    async fn chat_stream(&self, api_key: &str, request: ChatRequest) -> ProviderResult<BoxStream> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let opts = ConvertOpts {
            force_reasoning_content: needs_reasoning_echo(&request.model)
                || request.thinking.is_some(),
        };
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": request.messages.iter().map(|m| convert_message_with_opts(m, opts)).collect::<Vec<_>>(),
            "max_tokens": request.max_tokens,
            "stream": true,
            "stream_options": {"include_usage": true}
        });

        apply_openai_sampling(&mut body, &request);
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

        // Buffer across TCP chunks — SSE events are line-delimited and a chunk
        // boundary mid-`data: …` line used to drop the event (empty assistant turns).
        let boxed: BoxStream = Box::pin(sse_delta_stream(
            response.bytes_stream(),
            parse_openai_stream_line,
        ));
        Ok(boxed)
    }

    fn supports_thinking(&self) -> bool {
        // Provider-level gate: request model is not available on the trait.
        // Loop already combines this with a non-zero thinking_budget.
        true
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

    let reasoning = message["reasoning_content"]
        .as_str()
        .or_else(|| message["reasoning"].as_str());
    if let Some(thinking) = reasoning {
        if !thinking.is_empty() {
            content.push(Content {
                content_type: ContentType::Thinking,
                text: None,
                id: None,
                name: None,
                input: None,
                tool_use_id: None,
                content: None,
                signature: None,
                thinking: Some(thinking.to_string()),
                is_error: false,
            });
        }
    }

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
                is_error: false,
            });
        }
    }

    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let arguments =
                get_arguments(&tc["function"]["arguments"]).unwrap_or_else(|| "{}".to_string());
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
                is_error: false,
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
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
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

    if choice["finish_reason"]
        .as_str()
        .is_some_and(|r| !r.is_empty())
    {
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
                        r#type: DeltaType::ToolCallDelta { input: args },
                    });
                }
            }
        }
    }

    let reasoning = delta["reasoning_content"]
        .as_str()
        .or_else(|| delta["reasoning"].as_str());
    if let Some(thinking) = reasoning {
        if !thinking.is_empty() {
            return Some(StreamDelta {
                content_index: 0,
                r#type: DeltaType::Thinking {
                    thinking: thinking.to_string(),
                },
            });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_with(content: Vec<Content>) -> Message {
        Message {
            role: Role::Assistant,
            content,
        }
    }

    #[test]
    fn convert_thinking_only_sets_empty_content_not_null() {
        let msg = assistant_with(vec![Content {
            content_type: ContentType::Thinking,
            thinking: Some("planning a reply".into()),
            ..Default::default()
        }]);
        let v = convert_message_with_opts(&msg, ConvertOpts::default());
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "");
        assert_eq!(v["reasoning_content"], "planning a reply");
        assert!(v.get("tool_calls").is_none());
    }

    #[test]
    fn convert_text_and_thinking_roundtrips_reasoning() {
        let msg = assistant_with(vec![
            Content {
                content_type: ContentType::Thinking,
                thinking: Some("reason".into()),
                ..Default::default()
            },
            Content {
                content_type: ContentType::Text,
                text: Some("hello".into()),
                ..Default::default()
            },
        ]);
        let v = convert_message_with_opts(&msg, ConvertOpts::default());
        assert_eq!(v["content"], "hello");
        assert_eq!(v["reasoning_content"], "reason");
    }

    #[test]
    fn convert_deepseek_forces_empty_reasoning_content() {
        let msg = assistant_with(vec![Content {
            content_type: ContentType::Text,
            text: Some("hi".into()),
            ..Default::default()
        }]);
        let v = convert_message_with_opts(
            &msg,
            ConvertOpts {
                force_reasoning_content: true,
            },
        );
        assert_eq!(v["content"], "hi");
        assert_eq!(v["reasoning_content"], "");
    }

    #[test]
    fn convert_tool_calls_may_keep_null_content() {
        let msg = assistant_with(vec![
            Content {
                content_type: ContentType::Thinking,
                thinking: Some("need bash".into()),
                ..Default::default()
            },
            Content {
                content_type: ContentType::ToolUse,
                id: Some("call_1".into()),
                name: Some("bash".into()),
                input: Some(serde_json::json!({"command": "ls"})),
                ..Default::default()
            },
        ]);
        let v = convert_message_with_opts(&msg, ConvertOpts::default());
        assert!(v["content"].is_null());
        assert!(v["tool_calls"].is_array());
        assert_eq!(v["reasoning_content"], "need bash");
    }
}
