use crate::ai::provider::{BoxStream, Provider};
use crate::ai::types::*;
use async_trait::async_trait;
use futures::StreamExt;
use regex::Regex;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

pub struct OpenCodeCliProvider {
    pub name: String,
    pub bin: String,
    pub default_model: String,
}

impl Default for OpenCodeCliProvider {
    fn default() -> Self {
        Self {
            name: "opencode-cli".to_string(),
            bin: "opencode".to_string(),
            default_model: "opencode/deepseek-v4-flash-free".to_string(),
        }
    }
}

impl OpenCodeCliProvider {
    pub fn new(bin: Option<String>, default_model: Option<String>) -> Self {
        Self {
            name: "opencode-cli".to_string(),
            bin: bin.unwrap_or_else(|| "opencode".to_string()),
            default_model: default_model
                .unwrap_or_else(|| "opencode/deepseek-v4-flash-free".to_string()),
        }
    }

    fn build_prompt(&self, request: &ChatRequest) -> String {
        let mut sections = Vec::new();

        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let tool_call_example = "<tool_call>{\"name\":\"tool_name\",\"arguments\":{...}}</tool_call>";
        sections.push(
            format!(
                "# rs-agent bridge instructions\n\n\
                 You are being used as the model backend for rs-agent through the OpenCode CLI.\n\
                 OpenCode's own tools are disabled. Do NOT try to use OpenCode tools.\n\n\
                 Working directory: {cwd}\n\n\
                 To use a tool, output a <tool_call> block:\n\
                 {tool_call_example}\n\n\
                 Rules:\n\
                 - Use tools from \"Available tools\" below.\n\
                 - DO NOT include file contents in your text response. Use the write tool for file creation.\n\
                 - After tool results, continue and either answer or request another tool call.\n\
                 - When using the write tool, use the working directory above as the base path.",
                cwd = cwd,
                tool_call_example = tool_call_example,
            )
        );

        if let Some(system) = &request.system {
            if !system.is_empty() {
                sections.push(format!(
                    "# rs-agent system prompt\n\n{}",
                    system
                ));
            }
        }

        if !request.tools.is_empty() {
            let tools_json: Vec<serde_json::Value> = request
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema
                    })
                })
                .collect();
            sections.push(format!(
                "# Available tools\n\n{}",
                serde_json::to_string_pretty(&tools_json).unwrap_or_default()
            ));
        } else {
            sections.push(
                "# Available tools\n\nNo tools are available for this turn."
                    .to_string(),
            );
        }

        if !request.messages.is_empty() {
            let mut transcript = Vec::new();
            for msg in &request.messages {
                let role = match msg.role {
                    Role::User => "USER",
                    Role::Assistant => "ASSISTANT",
                    Role::Tool => "TOOL RESULT",
                    Role::System => "SYSTEM",
                };
                for block in &msg.content {
                    match block.content_type {
                        ContentType::Text => {
                            if let Some(text) = &block.text {
                                transcript.push(format!("{}: {}", role, text));
                            }
                        }
                        ContentType::ToolUse => {
                            if let (Some(id), Some(name), Some(input)) =
                                (&block.id, &block.name, &block.input)
                            {
                                transcript.push(format!(
                                    "ASSISTANT: <tool_call>{}</tool_call>",
                                    serde_json::json!({"name": name, "arguments": input, "id": id})
                                ));
                            }
                        }
                        ContentType::ToolResult => {
                            if let (Some(id), Some(text)) = (&block.tool_use_id, &block.text) {
                                let tool_name = block.name.as_deref().unwrap_or("?");
                                transcript.push(format!(
                                    "TOOL RESULT (id={}, tool={}): {}",
                                    id, tool_name, text
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if transcript.is_empty() {
                sections.push("# Conversation transcript\n\n(no prior messages)".to_string());
            } else {
                sections.push(format!(
                    "# Conversation transcript\n\n{}",
                    transcript.join("\n\n---\n\n")
                ));
            }
        } else {
            sections.push(
                "# Conversation transcript\n\n(no prior messages)"
                    .to_string(),
            );
        }

        sections.push("Now produce the next assistant message for rs-agent.".to_string());
        sections.join("\n\n---\n\n")
    }

    fn parse_tool_calls(text: &str) -> Vec<(String, String, serde_json::Value)> {
        let re = Regex::new(r"<tool_call>([\s\S]*?)</tool_call>").unwrap();
        let mut calls = Vec::new();
        for cap in re.captures_iter(text) {
            if let Some(json_str) = cap.get(1) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str.as_str()) {
                    let name = value["name"].as_str().unwrap_or("").to_string();
                    let args = value["arguments"].clone();
                    let id = value["id"]
                        .as_str()
                        .unwrap_or(&format!("call_{}", calls.len()))
                        .to_string();
                    calls.push((id, name, args));
                }
            }
        }
        if calls.is_empty() {
            if let Some(start) = text.find("<tool_call>") {
                let json_part = &text[start + "<tool_call>".len()..];
                if let Some(end) = json_part.find("</tool_call>") {
                    let json_str = &json_part[..end];
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                        let name = value["name"].as_str().unwrap_or("").to_string();
                        let args = value["arguments"].clone();
                        let id = value["id"]
                            .as_str()
                            .unwrap_or("call_0")
                            .to_string();
                        calls.push((id, name, args));
                    }
                } else {
                    let trimmed = json_part.trim();
                    if trimmed.ends_with('}') || trimmed.ends_with("}}") {
                        let end = if trimmed.ends_with("}}") {
                            trimmed.len()
                        } else if trimmed.ends_with('}') {
                            trimmed.rfind('}').map(|i| i + 1).unwrap_or(trimmed.len())
                        } else {
                            trimmed.len()
                        };
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[..end]) {
                            let name = value["name"].as_str().unwrap_or("").to_string();
                            let args = value["arguments"].clone();
                            let id = value["id"]
                                .as_str()
                                .unwrap_or("call_0")
                                .to_string();
                            calls.push((id, name, args));
                        }
                    }
                }
            }
        }
        if calls.is_empty() {
            let bare_re = Regex::new(r#"\{\s*"name"\s*:"#).unwrap();
            if let Some(m) = bare_re.find(text) {
                let start = m.start();
                let mut depth = 0i32;
                let mut end = text.len();
                for (i, ch) in text[start..].char_indices() {
                    if ch == '{' { depth += 1; }
                    else if ch == '}' {
                        depth -= 1;
                        if depth == 0 {
                            end = start + i + 1;
                            break;
                        }
                    }
                }
                if depth == 0 {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text[start..end]) {
                        if let Some(name) = value["name"].as_str() {
                            if value["arguments"].is_object() {
                                let id = value["id"]
                                    .as_str()
                                    .unwrap_or("call_0")
                                    .to_string();
                                calls.push((id, name.to_string(), value["arguments"].clone()));
                            }
                        }
                    }
                }
            }
        }
        calls
    }
}

#[async_trait]
impl Provider for OpenCodeCliProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn api_key_env_var(&self) -> &str {
        "OPENCODE_API_KEY"
    }

    fn base_url(&self) -> &str {
        "cli:opencode"
    }

    async fn chat(&self, _api_key: &str, request: ChatRequest) -> ProviderResult<AssistantMessage> {
        let model = request.model.clone();
        let mut stream = self.chat_stream("", request).await?;
        let mut content = Vec::new();
        let mut text_buf = String::new();
        let mut stop_reason = None;

        while let Some(result) = stream.next().await {
            match result {
                Ok(delta) => match delta.r#type {
                    DeltaType::Text { text } => text_buf.push_str(&text),
                    DeltaType::ToolCallStart { id, name, input } => {
                        content.push(Content {
                            content_type: ContentType::ToolUse,
                            text: None,
                            id: Some(id),
                            name: Some(name),
                            input: Some(
                                serde_json::from_str(&input)
                                    .unwrap_or(serde_json::Value::Null),
                            ),
                            tool_use_id: None,
                            content: None,
                            signature: None,
                            thinking: None,
                        });
                    }
                    DeltaType::Stop { stop_reason: reason } => {
                        stop_reason = reason;
                    }
                    _ => {}
                },
                Err(e) => return Err(e),
            }
        }

        if !text_buf.is_empty() {
            content.insert(
                0,
                Content {
                    content_type: ContentType::Text,
                    text: Some(text_buf),
                    id: None,
                    name: None,
                    input: None,
                    tool_use_id: None,
                    content: None,
                    signature: None,
                    thinking: None,
                },
            );
        }

        Ok(AssistantMessage {
            content,
            stop_reason,
            usage: None,
            model: model,
            id: None,
        })
    }

    async fn chat_stream(
        &self,
        _api_key: &str,
        request: ChatRequest,
    ) -> ProviderResult<BoxStream> {
        let prompt = self.build_prompt(&request);
        let model = request.model.clone();
        let bin = self.bin.clone();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            let temp_dir = match tempfile::tempdir() {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(Err(ProviderError::Other(format!(
                        "Failed to create temp dir: {}",
                        e
                    ))));
                    return;
                }
            };

            let agent_dir = temp_dir.path().join(".opencode").join("agents");
            if let Err(e) = tokio::fs::create_dir_all(&agent_dir).await {
                let _ = tx.send(Err(ProviderError::Other(format!(
                    "Failed to create agent dir: {}",
                    e
                ))));
                return;
            }

            let agent_content = "---\ndescription: rs-agent bridge agent. All OpenCode tools are denied.\nmode: primary\npermission:\n  read: deny\n  edit: deny\n  glob: deny\n  grep: deny\n  list: deny\n  bash: deny\n  task: deny\n  external_directory: deny\n  todowrite: deny\n  webfetch: deny\n  websearch: deny\n  lsp: deny\n  skill: deny\n  question: deny\n---\nYou are the rs-agent side of a bridge. OpenCode tools are disabled. Reply in plain text, or emit <tool_call>{\"name\":\"...\",\"arguments\":{...}}</tool_call> when you need to request a tool. Do NOT try to use OpenCode tools.\n";
            if let Err(e) = tokio::fs::write(agent_dir.join("pi-model.md"), agent_content).await {
                let _ = tx.send(Err(ProviderError::Other(format!(
                    "Failed to write agent config: {}",
                    e
                ))));
                return;
            }

            let mut child = match Command::new(&bin)
                .arg("run")
                .arg("--pure")
                .arg("-m")
                .arg(&model)
                .arg("--agent")
                .arg("pi-model")
                .arg("--format")
                .arg("json")
                .arg("--dir")
                .arg(temp_dir.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Err(ProviderError::Other(format!(
                        "Failed to spawn opencode: {}",
                        e
                    ))));
                    return;
                }
            };

            if let Some(stdin) = child.stdin.as_mut() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(prompt.as_bytes()).await;
            }
            drop(child.stdin.take());

            let stderr = child.stderr.take().unwrap();
            let stderr_reader = tokio::io::BufReader::new(stderr);
            let mut stderr_lines = stderr_reader.lines();
            let stderr_handle = tokio::spawn(async move {
                let mut buf = String::new();
                while let Ok(Some(line)) = stderr_lines.next_line().await {
                    if !buf.is_empty() { buf.push('\n'); }
                    buf.push_str(&line);
                }
                buf
            });

            let stdout = child.stdout.take().unwrap();
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut tool_call_buffer = String::new();
            let mut content_index = 0u32;
            let mut tool_call_pending = false;
            let mut had_output = false;

            loop {
                tokio::select! {
                    line_result = lines.next_line() => {
                        match line_result {
                            Ok(Some(line)) => {
                                let trimmed = line.trim().to_string();
                                if trimmed.is_empty() {
                                    continue;
                                }

                                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&trimmed) {
                                    had_output = true;
                                    match event["type"].as_str() {
                                        Some("text") => {
                                            if let Some(part_text) = event["part"]["text"].as_str() {
                                                let has_tag = part_text.contains("<tool_call>") || tool_call_buffer.contains("<tool_call>");
                                                let has_bare = part_text.contains("{\"name\"") || tool_call_buffer.contains("{\"name\"");
                                                if has_tag {
                                                    tool_call_buffer.push_str(part_text);
                                                    let re = Regex::new(r"<tool_call>([\s\S]*?)</tool_call>").unwrap();
                                                    let mut calls = Vec::new();
                                                    let mut last_end = 0;
                                                    for cap in re.captures_iter(&tool_call_buffer) {
                                                        let m = cap.get(0).unwrap();
                                                        if m.start() > last_end {
                                                            let before = &tool_call_buffer[last_end..m.start()];
                                                            let _ = tx.send(Ok(StreamDelta {
                                                                content_index,
                                                                r#type: DeltaType::Text { text: before.to_string() },
                                                            }));
                                                        }
                                                        if let Some(json_str) = cap.get(1) {
                                                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str.as_str()) {
                                                                let name = value["name"].as_str().unwrap_or("").to_string();
                                                                let args = value["arguments"].clone();
                                                                let id = value["id"]
                                                                    .as_str()
                                                                    .unwrap_or(&format!("call_{}", calls.len()))
                                                                    .to_string();
                                                                calls.push((id, name, args));
                                                            }
                                                        }
                                                        last_end = m.end();
                                                    }
                                                    for (id, name, args) in &calls {
                                                        tool_call_pending = true;
                                                        let args_str = serde_json::to_string(args).unwrap_or_default();
                                                        let _ = tx.send(Ok(StreamDelta {
                                                            content_index,
                                                            r#type: DeltaType::ToolCallStart {
                                                                id: id.clone(),
                                                                name: name.clone(),
                                                                input: args_str.clone(),
                                                            },
                                                        }));
                                                        content_index += 1;
                                                    }
                                                    if !calls.is_empty() {
                                                        let remaining = &tool_call_buffer[last_end..];
                                                        tool_call_buffer = remaining.to_string();
                                                    }
                                                } else if has_bare {
                                                    if let Some(pos) = part_text.find("{\"name\"") {
                                                        if pos > 0 {
                                                            let before = &part_text[..pos];
                                                            let _ = tx.send(Ok(StreamDelta {
                                                                content_index,
                                                                r#type: DeltaType::Text { text: before.to_string() },
                                                            }));
                                                        }
                                                        tool_call_buffer.push_str(&part_text[pos..]);
                                                    } else {
                                                        tool_call_buffer.push_str(part_text);
                                                    }
                                                } else {
                                                    let _ = tx.send(Ok(StreamDelta {
                                                        content_index,
                                                        r#type: DeltaType::Text { text: part_text.to_string() },
                                                    }));
                                                }
                                            }
                                        }
                                        Some("step_finish") => {
                                            if !tool_call_buffer.is_empty() {
                                                let calls = OpenCodeCliProvider::parse_tool_calls(&tool_call_buffer);
                                                if !calls.is_empty() {
                                                    for (id, name, args) in &calls {
                                                        tool_call_pending = true;
                                                        let args_str = serde_json::to_string(args).unwrap_or_default();
                                                        let _ = tx.send(Ok(StreamDelta {
                                                            content_index,
                                                            r#type: DeltaType::ToolCallStart {
                                                                id: id.clone(),
                                                                name: name.clone(),
                                                                input: args_str.clone(),
                                                            },
                                                        }));
                                                        content_index += 1;
                                                    }
                                                } else {
                                                    let _ = tx.send(Ok(StreamDelta {
                                                        content_index,
                                                        r#type: DeltaType::Text { text: tool_call_buffer.clone() },
                                                    }));
                                                }
                                                tool_call_buffer.clear();
                                            }
                                            if !tool_call_pending {
                                                let _ = tx.send(Ok(StreamDelta {
                                                    content_index: 0,
                                                    r#type: DeltaType::Stop {
                                                        stop_reason: Some(StopReason::EndTurn),
                                                    },
                                                }));
                                            } else {
                                                let _ = tx.send(Ok(StreamDelta {
                                                    content_index: 0,
                                                    r#type: DeltaType::Stop {
                                                        stop_reason: Some(StopReason::ToolUse),
                                                    },
                                                }));
                                            }
                                        }
                                        Some("error") => {
                                            let msg = event["error"]["message"]
                                                .as_str()
                                                .unwrap_or("unknown error")
                                                .to_string();
                                            let _ = tx.send(Err(ProviderError::Other(msg)));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                let _ = tx.send(Err(ProviderError::Other(format!("read error: {}", e))));
                                break;
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(120)) => {
                        let _ = child.kill().await;
                        let _ = tx.send(Err(ProviderError::Timeout));
                        break;
                    }
                }
            }

            let status = child.wait().await;
            let stderr_out = stderr_handle.await.unwrap_or_default();

            if !had_output && !stderr_out.is_empty() {
                let _ = tx.send(Err(ProviderError::Other(format!(
                    "opencode process exited with stderr: {}",
                    stderr_out
                ))));
            } else if let Ok(Some(exit_code)) = status.map(|s| s.code()) {
                if exit_code != 0 && !had_output {
                    let msg = if stderr_out.is_empty() {
                        format!("opencode process exited with code {}", exit_code)
                    } else {
                        format!("opencode process exited with code {}: {}", exit_code, stderr_out)
                    };
                    let _ = tx.send(Err(ProviderError::Other(msg)));
                }
            }
        });

        let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
        let boxed: BoxStream = Box::pin(stream);
        Ok(boxed)
    }

    fn supports_thinking(&self) -> bool {
        false
    }

    fn default_max_tokens(&self) -> u32 {
        16384
    }
}
