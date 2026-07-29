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
    pub timeout_secs: u64,
}

impl Default for OpenCodeCliProvider {
    fn default() -> Self {
        Self {
            name: "opencode-cli".to_string(),
            bin: "opencode".to_string(),
            default_model: "opencode/deepseek-v4-flash-free".to_string(),
            timeout_secs: 300,
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
            timeout_secs: 300,
        }
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
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
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                    Role::System => "system",
                };
                for block in &msg.content {
                    match block.content_type {
                        ContentType::Text => {
                            if let Some(text) = &block.text {
                                // XML-ish tags — weak models echo "ASSISTANT:" / "---" from
                                // the old format back into the stream and thrash.
                                transcript.push(format!(
                                    "<turn role=\"{role}\">\n{}\n</turn>",
                                    text.trim_end()
                                ));
                            }
                        }
                        ContentType::ToolUse => {
                            if let (Some(id), Some(name), Some(input)) =
                                (&block.id, &block.name, &block.input)
                            {
                                transcript.push(format!(
                                    "<turn role=\"assistant\">\n<tool_call>{}</tool_call>\n</turn>",
                                    serde_json::json!({"name": name, "arguments": input, "id": id})
                                ));
                            }
                        }
                        ContentType::ToolResult => {
                            if let (Some(id), Some(text)) = (&block.tool_use_id, &block.text) {
                                let tool_name = block.name.as_deref().unwrap_or("?");
                                transcript.push(format!(
                                    "<turn role=\"tool\" id=\"{id}\" name=\"{tool_name}\">\n{}\n</turn>",
                                    text.trim_end()
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
                    "# Conversation transcript\n\n\
                     (Do NOT copy or echo <turn> tags, role labels, or separators. \
                     Continue with new assistant text or a <tool_call> only.)\n\n{}",
                    transcript.join("\n\n")
                ));
            }
        } else {
            sections.push(
                "# Conversation transcript\n\n(no prior messages)"
                    .to_string(),
            );
        }

        sections.push(
            "Now produce the next assistant message for rs-agent.\n\
             Output plain text and/or <tool_call>{{...}}</tool_call> only.\n\
             Never echo prior turns, <turn> tags, or markdown `---` separators."
                .to_string(),
        );
        // Avoid `---` separators — free models parrot them as "---\\n\\nASSISTANT:".
        sections.join("\n\n====\n\n")
    }

    /// Drop transcript scaffolding that weak models copy from the bridge prompt.
    fn sanitize_bridge_text(text: &str) -> Option<String> {
        if Self::is_transcript_echo_only(text) {
            return None;
        }
        let mut out = String::new();
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t == "---" || t == "====" {
                continue;
            }
            if let Some(rest) = t
                .strip_prefix("ASSISTANT:")
                .or_else(|| t.strip_prefix("USER:"))
                .or_else(|| t.strip_prefix("SYSTEM:"))
            {
                let rest = rest.trim();
                if !rest.is_empty() {
                    out.push_str(rest);
                    out.push('\n');
                }
                continue;
            }
            if t.starts_with("TOOL RESULT") {
                continue;
            }
            if t.starts_with("<turn") || t == "</turn>" {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        // Chunks without newlines (stream fragments)
        if out.is_empty() && !text.contains('\n') {
            let t = text.trim();
            if let Some(rest) = t.strip_prefix("ASSISTANT:") {
                let rest = rest.trim();
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
        let trimmed = out.trim();
        if trimmed.is_empty() || Self::is_transcript_echo_only(trimmed) {
            None
        } else {
            Some(out)
        }
    }

    fn is_transcript_echo_only(text: &str) -> bool {
        let mut rest = text.to_string();
        for pat in [
            "---",
            "====",
            "ASSISTANT:",
            "USER:",
            "SYSTEM:",
            "<turn",
            "</turn>",
        ] {
            rest = rest.replace(pat, "");
        }
        let rest = Regex::new(r"(?i)TOOL RESULT\b[^\n:]*:")
            .unwrap()
            .replace_all(&rest, "");
        rest.chars().all(|c| c.is_whitespace())
    }

    fn parse_tool_calls(text: &str) -> Vec<(String, String, serde_json::Value)> {
        let stripped = text.replace("<tool_call>", "").replace("</tool_call>", "");
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
            if let Some(m) = bare_re.find(&stripped) {
                let start = m.start();
                let mut depth = 0i32;
                let mut end = stripped.len();
                for (i, ch) in stripped[start..].char_indices() {
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
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&stripped[start..end]) {
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
        // Fallback: parse opencode-native <tool_name> + <tool_arguments> format
        if calls.is_empty() {
            calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        }
        calls
    }

    fn find_balanced_json_in(s: &str, start: usize) -> Option<(usize, String)> {
        let bytes = s.as_bytes();
        let mut brace_pos = start;
        while brace_pos < bytes.len() && bytes[brace_pos].is_ascii_whitespace() {
            brace_pos += 1;
        }
        if brace_pos >= bytes.len() || bytes[brace_pos] != b'{' {
            return None;
        }
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        for i in brace_pos..bytes.len() {
            if escaped { escaped = false; continue; }
            match bytes[i] {
                b'"' if !in_string => in_string = true,
                b'"' if in_string => in_string = false,
                b'\\' if in_string => escaped = true,
                b'{' if !in_string => depth += 1,
                b'}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((i + 1, s[brace_pos..i + 1].to_string()));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn parse_native_tool_calls(text: &str) -> Vec<(String, String, serde_json::Value)> {
        let mut calls = Vec::new();
        let name_re = Regex::new(r"(?s)<tool_name>\s*(.*?)\s*</tool_name>").unwrap();
        let args_open_re = Regex::new(r"<tool_arguments>\s*").unwrap();
        let name_matches: Vec<_> = name_re.captures_iter(text).collect();
        let args_matches: Vec<_> = args_open_re.captures_iter(text).collect();

        // Strategy 1: pair <tool_name> with <tool_arguments>{json}</tool_arguments>
        if name_matches.len() == 1 && !args_matches.is_empty() {
            let tool_name = name_matches[0].get(1).unwrap().as_str().trim().to_string();
            if !tool_name.is_empty() {
                for args_cap in &args_matches {
                    let args_start = args_cap.get(0).unwrap().end();
                    if let Some((obj_end, json_str)) = Self::find_balanced_json_in(text, args_start) {
                        let after = text[obj_end..].trim();
                        if after.starts_with("</tool_arguments>") {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                let id = value.get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("call_0")
                                    .to_string();
                                calls.push((id, tool_name.clone(), value));
                                return calls;
                            }
                        }
                    }
                }
            }
        }

        // Strategy 2: Extract JSON from <tool_arguments>, derive name from <tool_name> or JSON
        for args_cap in &args_matches {
            let args_start = args_cap.get(0).unwrap().end();
            if let Some((_obj_end, json_str)) = Self::find_balanced_json_in(text, args_start) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    let has_name_args = value["name"].as_str().map_or(false, |n| !n.is_empty())
                        && value["arguments"].is_object();
                    let name = if has_name_args {
                        value["name"].as_str().unwrap().to_string()
                    } else {
                        name_re.captures(text)
                            .and_then(|c| c.get(1))
                            .map(|m| m.as_str().trim().to_string())
                            .unwrap_or_default()
                    };
                    if !name.is_empty() {
                        let args = if has_name_args {
                            value["arguments"].clone()
                        } else {
                            value.clone()
                        };
                        let id = value["id"]
                            .as_str()
                            .unwrap_or("call_0")
                            .to_string();
                        calls.push((id, name, args));
                        return calls;
                    }
                }
            }
        }

        // Strategy 2b: <tool_name>name</tool_name> followed by bare JSON (no <tool_arguments>)
        if calls.is_empty() && !name_matches.is_empty() {
            for name_cap in &name_matches {
                let name_end = name_cap.get(0).unwrap().end();
                let tool_name = name_cap.get(1).unwrap().as_str().trim().to_string();
                if tool_name.is_empty() {
                    continue;
                }
                if let Some((obj_end, json_str)) = Self::find_balanced_json_in(text, name_end) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        let has_name = value.get("name").and_then(|v| v.as_str()).map_or(false, |n| !n.is_empty());
                        let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("call_0").to_string();
                        if has_name && value.get("arguments").map_or(false, |a| a.is_object()) {
                            calls.push((id, value["name"].as_str().unwrap().to_string(), value["arguments"].clone()));
                        } else {
                            calls.push((id, tool_name, value));
                        }
                        // Continue scanning after this JSON for more tool calls
                        let remaining = if obj_end < text.len() { &text[obj_end..] } else { "" };
                        if !remaining.trim().is_empty() {
                            let extra = Self::parse_native_tool_calls(remaining);
                            calls.extend(extra);
                        }
                        return calls;
                    }
                }
            }
        }

        // Strategy 3: fallback — look for JSON objects with "name" and "arguments" anywhere
        if calls.is_empty() {
            let bare_re = Regex::new(r#"\{\s*"name"\s*:"#).unwrap();
            if let Some(m) = bare_re.find(text) {
                if let Some((_obj_end, json_str)) = Self::find_balanced_json_in(text, m.start()) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        let name = value["name"].as_str().unwrap_or("").to_string();
                        if !name.is_empty() && value["arguments"].is_object() {
                            let id = value["id"].as_str().unwrap_or("call_0").to_string();
                            calls.push((id, name, value["arguments"].clone()));
                        }
                    }
                }
            }
        }

        // Strategy 4: opencode-native <tool_request> XML format
        // <tool_request id="N" tool="name"><parameters><key>value</key>...</parameters></tool_request>
        if calls.is_empty() {
            let tool_req_re = Regex::new(r#"(?s)<tool_request\s+id="([^"]*)"\s+tool="([^"]*)"\s*>"#).unwrap();
            let params_re = Regex::new(r"(?s)<parameters>(.*?)</parameters>").unwrap();
            let param_re = Regex::new(r"(?s)<([a-zA-Z_][a-zA-Z0-9_]*)>(.*?)</[a-zA-Z_][a-zA-Z0-9_]*>").unwrap();

            for req_cap in tool_req_re.captures_iter(text) {
                let id = req_cap.get(1).unwrap().as_str().to_string();
                let name = req_cap.get(2).unwrap().as_str().to_string();
                let req_start = req_cap.get(0).unwrap().start();
                let rest = &text[req_start..];

                if let Some(params_cap) = params_re.captures(rest) {
                    let params_xml = params_cap.get(1).unwrap().as_str();
                    let mut args = serde_json::Map::new();
                    for param_cap in param_re.captures_iter(params_xml) {
                        let key = param_cap.get(1).unwrap().as_str().to_string();
                        let raw_val = param_cap.get(2).unwrap().as_str();
                        // Try number, then keep as string
                        let val: serde_json::Value = if let Ok(n) = raw_val.parse::<i64>() {
                            serde_json::Value::Number(n.into())
                        } else if let Ok(f) = raw_val.parse::<f64>() {
                            serde_json::Value::Number(serde_json::Number::from_f64(f).unwrap_or(
                                serde_json::Number::from_f64(0.0).unwrap()
                            ))
                        } else {
                            serde_json::Value::String(raw_val.to_string())
                        };
                        args.insert(key, val);
                    }
                    calls.push((id, name, serde_json::Value::Object(args)));
                }
            }
        }

        // Strategy 5: MiniMax / ling `<arg_key>` / `<arg_value>` inside `<tool_call>`
        //   <tool_call>webfetch
        //   <arg_key>url</arg_key>
        //   <arg_value>https://...</arg_value>
        //   </tool_call>
        if calls.is_empty() {
            calls = Self::parse_arg_key_tool_calls(text);
        }

        calls
    }

    /// Parse MiniMax/ling-style tool calls: tool name + `<arg_key>`/`<arg_value>` pairs.
    fn parse_arg_key_tool_calls(text: &str) -> Vec<(String, String, serde_json::Value)> {
        let mut calls = Vec::new();
        if !text.contains("<arg_key>") {
            return calls;
        }

        let block_re = Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap();
        let key_re = Regex::new(r"(?s)<arg_key>\s*(.*?)\s*</arg_key>").unwrap();
        let val_re = Regex::new(r"(?s)<arg_value>\s*(.*?)\s*</arg_value>").unwrap();

        let mut bodies: Vec<String> = block_re
            .captures_iter(text)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect();

        // Streaming path may already have stripped the outer `<tool_call>` tags.
        if bodies.is_empty() {
            bodies.push(text.to_string());
        }

        for (idx, body) in bodies.into_iter().enumerate() {
            let trimmed = body.trim();
            if trimmed.starts_with('{')
                || trimmed.contains("<tool_name>")
                || trimmed.contains("<tool_request")
                || trimmed.contains("<tool_arguments>")
                || !trimmed.contains("<arg_key>")
            {
                continue;
            }

            // Tool name = first token before any XML tag.
            let before_tag = trimmed.split('<').next().unwrap_or("").trim();
            let name = before_tag
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| c == '"' || c == '\'')
                .to_string();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                continue;
            }

            let keys: Vec<String> = key_re
                .captures_iter(trimmed)
                .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
                .filter(|k| !k.is_empty())
                .collect();
            let vals: Vec<String> = val_re
                .captures_iter(trimmed)
                .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                .collect();
            if keys.is_empty() || keys.len() != vals.len() {
                continue;
            }

            let mut args = serde_json::Map::new();
            for (k, v) in keys.iter().zip(vals.iter()) {
                args.insert(k.clone(), Self::coerce_xml_arg_value(v));
            }
            calls.push((
                format!("call_{idx}"),
                name,
                serde_json::Value::Object(args),
            ));
        }

        calls
    }

    fn coerce_xml_arg_value(raw: &str) -> serde_json::Value {
        let raw = raw.trim();
        if let Ok(n) = raw.parse::<i64>() {
            return serde_json::Value::Number(n.into());
        }
        if let Ok(f) = raw.parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return serde_json::Value::Number(n);
            }
        }
        if raw.eq_ignore_ascii_case("true") {
            return serde_json::Value::Bool(true);
        }
        if raw.eq_ignore_ascii_case("false") {
            return serde_json::Value::Bool(false);
        }
        serde_json::Value::String(raw.to_string())
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
                        is_error: false,
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
                    is_error: false,
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
        let timeout_secs = self.timeout_secs;

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
                                                let has_tool_call = part_text.contains("<tool_call>") || tool_call_buffer.contains("<tool_call>");
                                                let has_native = part_text.contains("<tool_name>") || part_text.contains("<tool_arguments>") || part_text.contains("<tool_request")
                                                    || part_text.contains("<arg_key>") || part_text.contains("<arg_value>")
                                                    || tool_call_buffer.contains("<tool_name>") || tool_call_buffer.contains("<tool_arguments>") || tool_call_buffer.contains("<tool_request")
                                                    || tool_call_buffer.contains("<arg_key>") || tool_call_buffer.contains("<arg_value>");
                                                let has_bare = part_text.contains("{\"name\"") || tool_call_buffer.contains("{\"name\"");
                                                if has_tool_call {
                                                    tool_call_buffer.push_str(part_text);
                                                    let re = Regex::new(r"<tool_call>([\s\S]*?)</tool_call>").unwrap();
                                                    let mut calls = Vec::new();
                                                    let mut last_end = 0;
                                                    for cap in re.captures_iter(&tool_call_buffer) {
                                                        let m = cap.get(0).unwrap();
                                                        if m.start() > last_end {
                                                            let before = &tool_call_buffer[last_end..m.start()];
                                                            if let Some(cleaned) =
                                                                OpenCodeCliProvider::sanitize_bridge_text(before)
                                                            {
                                                                let _ = tx.send(Ok(StreamDelta {
                                                                    content_index,
                                                                    r#type: DeltaType::Text { text: cleaned },
                                                                }));
                                                                content_index += 1;
                                                            }
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
                                                            } else {
                                                                // JSON parse failed — captured content may be native /
                                                                // arg_key format wrapped in <tool_call> tags
                                                                let native = OpenCodeCliProvider::parse_native_tool_calls(json_str.as_str());
                                                                if !native.is_empty() {
                                                                    calls.extend(native);
                                                                } else {
                                                                    let fallback = OpenCodeCliProvider::parse_tool_calls(
                                                                        &format!("<tool_call>{}</tool_call>", json_str.as_str()),
                                                                    );
                                                                    calls.extend(fallback);
                                                                }
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
                                                } else if has_native {
                                                    tool_call_buffer.push_str(part_text);
                                                    let native_calls = OpenCodeCliProvider::parse_native_tool_calls(&tool_call_buffer);
                                                    if !native_calls.is_empty() {
                                                        for (id, name, args) in &native_calls {
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
                                                        tool_call_buffer.clear();
                                                    }
                                                } else if has_bare {
                                                    if let Some(pos) = part_text.find("{\"name\"") {
                                                        if pos > 0 {
                                                            let before = &part_text[..pos];
                                                            if let Some(cleaned) =
                                                                OpenCodeCliProvider::sanitize_bridge_text(before)
                                                            {
                                                                let _ = tx.send(Ok(StreamDelta {
                                                                    content_index,
                                                                    r#type: DeltaType::Text { text: cleaned },
                                                                }));
                                                                content_index += 1;
                                                            }
                                                        }
                                                        tool_call_buffer.push_str(&part_text[pos..]);
                                                    } else {
                                                        tool_call_buffer.push_str(part_text);
                                                    }
                                                } else if let Some(cleaned) =
                                                    OpenCodeCliProvider::sanitize_bridge_text(part_text)
                                                {
                                                    let _ = tx.send(Ok(StreamDelta {
                                                        content_index,
                                                        r#type: DeltaType::Text { text: cleaned },
                                                    }));
                                                    content_index += 1;
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
                                                    tool_call_buffer.clear();
                                                } else if tool_call_buffer.contains("<tool_call>") {
                                                    // Try to merge {"name":...} from outside with {"arguments":...} inside <tool_call>
                                                    let stripped = tool_call_buffer
                                                        .replace("<tool_call>", "")
                                                        .replace("</tool_call>", "");
                                                    // Find name from any {"name":...} object in the buffer
                                                    let mut tool_name = String::new();
                                                    let mut tool_args: Option<serde_json::Value> = None;
                                                    // Extract all JSON objects from stripped text and merge
                                                    let mut pos = 0usize;
                                                    let bytes = stripped.as_bytes();
                                                    while pos < bytes.len() {
                                                        // Find next '{'
                                                        if let Some(open) = stripped[pos..].find('{') {
                                                            let abs_open = pos + open;
                                                            let mut depth = 0i32;
                                                            let mut close = bytes.len();
                                                            for (i, ch) in stripped[abs_open..].char_indices() {
                                                                if ch == '{' { depth += 1; }
                                                                else if ch == '}' {
                                                                    depth -= 1;
                                                                    if depth == 0 {
                                                                        close = abs_open + i + 1;
                                                                        break;
                                                                    }
                                                                }
                                                            }
                                                            if depth == 0 {
                                                                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stripped[abs_open..close]) {
                                                                    if let Some(n) = val["name"].as_str() {
                                                                        if !n.is_empty() {
                                                                            tool_name = n.to_string();
                                                                        }
                                                                    }
                                                                    if val["arguments"].is_object() {
                                                                        tool_args = Some(val["arguments"].clone());
                                                                    }
                                                                }
                                                                pos = close;
                                                            } else {
                                                                pos = abs_open + 1;
                                                            }
                                                        } else {
                                                            break;
                                                        }
                                                    }
                                                    if !tool_name.is_empty() && tool_args.is_some() {
                                                        tool_call_pending = true;
                                                        let args_str = serde_json::to_string(&tool_args.unwrap()).unwrap_or_default();
                                                        let id = format!("call_{}", content_index);
                                                        let _ = tx.send(Ok(StreamDelta {
                                                            content_index,
                                                            r#type: DeltaType::ToolCallStart {
                                                                id,
                                                                name: tool_name,
                                                                input: args_str,
                                                            },
                                                        }));
                                                        content_index += 1;
                                                    } else if let Some(ref args) = tool_args {
                                                        // Have arguments but no name — try to extract name from JSON keys
                                                        if let Some(name_from_args) = args.get("name").and_then(|v| v.as_str()) {
                                                            tool_call_pending = true;
                                                            let args_str = serde_json::to_string(args).unwrap_or_default();
                                                            let id = format!("call_{}", content_index);
                                                            let _ = tx.send(Ok(StreamDelta {
                                                                content_index,
                                                                r#type: DeltaType::ToolCallStart {
                                                                    id,
                                                                    name: name_from_args.to_string(),
                                                                    input: args_str,
                                                                },
                                                            }));
                                                            content_index += 1;
                                                        }
                                                    }
                                                    if !tool_call_pending {
                                                        // Try native <tool_name>/<tool_arguments> format
                                                        let native_calls = OpenCodeCliProvider::parse_native_tool_calls(&tool_call_buffer);
                                                        if !native_calls.is_empty() {
                                                            for (id, name, args) in &native_calls {
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
                                                        }
                                                    }
                                                    if !tool_call_pending {
                                                        // Failed to extract — emit as text so user sees the raw tool call
                                                        let _ = tx.send(Ok(StreamDelta {
                                                            content_index,
                                                            r#type: DeltaType::Text { text: tool_call_buffer.clone() },
                                                        }));
                                                    }
                                                    tool_call_buffer.clear();
                                                } else if tool_call_buffer.contains("<tool_name>") || tool_call_buffer.contains("<tool_arguments>") {
                                                    // Try native format
                                                    let native_calls = OpenCodeCliProvider::parse_native_tool_calls(&tool_call_buffer);
                                                    if !native_calls.is_empty() {
                                                        for (id, name, args) in &native_calls {
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
                                                    }
                                                    if !tool_call_pending {
                                                        let _ = tx.send(Ok(StreamDelta {
                                                            content_index,
                                                            r#type: DeltaType::Text { text: tool_call_buffer.clone() },
                                                        }));
                                                    }
                                                    tool_call_buffer.clear();
                                                } else {
                                                    let _ = tx.send(Ok(StreamDelta {
                                                        content_index,
                                                        r#type: DeltaType::Text { text: tool_call_buffer.clone() },
                                                    }));
                                                    tool_call_buffer.clear();
                                                }
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
                    _ = tokio::time::sleep(Duration::from_secs(timeout_secs)) => {
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

    async fn fetch_models(&self, _api_key: &str) -> ProviderResult<Vec<String>> {
        list_opencode_models(&self.bin).await
    }
}

/// Run `opencode models` and return model ids (`provider/model` lines).
pub async fn list_opencode_models(bin: &str) -> ProviderResult<Vec<String>> {
    let output = tokio::process::Command::new(bin)
        .arg("models")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProviderError::Other(
                    "opencode binary not found on PATH. Install OpenCode CLI to list models."
                        .to_string(),
                )
            } else {
                ProviderError::Other(format!("failed to run `{bin} models`: {e}"))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ProviderError::Other(format!(
            "`{bin} models` failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut models: Vec<String> = stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#') && l.contains('/'))
        .map(|l| l.to_string())
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

/// Sync helper for catalog seeding (best-effort; empty if opencode missing).
pub fn list_opencode_models_blocking(bin: &str) -> Vec<String> {
    let output = std::process::Command::new(bin)
        .arg("models")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut models: Vec<String> = stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && l.contains('/'))
        .map(|l| l.to_string())
        .collect();
    models.sort();
    models.dedup();
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_native_tool_name_bare_json() {
        // Format: <tool_name>name</tool_name>\n\n{<json>}
        let text = "<tool_name>bash</tool_name>\n\n{\n\"command\": \"ls\",\n\"timeout\": 30000\n}\n";
        let calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        assert_eq!(calls.len(), 1, "should find one tool call");
        assert_eq!(calls[0].1, "bash");
        assert_eq!(calls[0].2["command"], "ls");
        assert_eq!(calls[0].2["timeout"], 30000);
    }

    #[test]
    fn test_parse_native_tool_name_inline_json() {
        // Format: <tool_name>read</tool_name>\n{"file_path":"/path/to/file"}
        let text = "<tool_name>read</tool_name>\n{\"file_path\":\"/path/to/file\"}\n";
        let calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        assert_eq!(calls.len(), 1, "should find one tool call");
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[0].2["file_path"], "/path/to/file");
    }

    #[test]
    fn test_parse_native_tool_arguments_wrap() {
        // Format: <tool_arguments>{json}</tool_arguments>\n<tool_name>name</tool_name>
        let text = "<tool_arguments>{\"path\":\".\"}</tool_arguments>\n<tool_name>ls</tool_name>\n";
        let calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        assert_eq!(calls.len(), 1, "should find one tool call");
        assert_eq!(calls[0].1, "ls");
        assert_eq!(calls[0].2["path"], ".");
    }

    #[test]
    fn test_parse_native_multiple_calls() {
        // Two <tool_name> + bare JSON blocks
        let text = "<tool_name>read</tool_name>\n{\"file_path\":\"a.rs\"}\n\n<tool_name>read</tool_name>\n{\"file_path\":\"b.rs\"}\n";
        let calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        assert_eq!(calls.len(), 2, "should find two tool calls");
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[0].2["file_path"], "a.rs");
        assert_eq!(calls[1].1, "read");
        assert_eq!(calls[1].2["file_path"], "b.rs");
    }

    #[test]
    fn test_parse_tool_call_fallback() {
        // Verify parse_tool_calls calls parse_native_tool_calls as fallback
        let text = "<tool_name>bash</tool_name>\n{\"command\":\"ls\"}\n";
        let calls = OpenCodeCliProvider::parse_tool_calls(text);
        assert_eq!(calls.len(), 1, "parse_tool_calls should find native format via fallback");
        assert_eq!(calls[0].1, "bash");
    }

    #[test]
    fn test_find_balanced_json_whitespace() {
        // JSON with leading whitespace
        let s = "   \n\n{\"a\":1}";
        let result = OpenCodeCliProvider::find_balanced_json_in(s, 0);
        assert!(result.is_some(), "should find JSON after whitespace");
        let (end, json_str) = result.unwrap();
        assert_eq!(json_str, "{\"a\":1}");
        assert_eq!(end, s.len());
    }

    #[test]
    fn test_find_balanced_json_nested() {
        // JSON with nested objects
        let s = "{\"outer\":{\"inner\":\"value\"}}";
        let result = OpenCodeCliProvider::find_balanced_json_in(s, 0);
        assert!(result.is_some());
        let (end, json_str) = result.unwrap();
        assert_eq!(json_str, s);
        assert_eq!(end, s.len());
    }

    #[test]
    fn test_native_inside_tool_call_wrapper() {
        // This is the EXACT format from the user's bug report:
        // <tool_call> wrapping <tool_arguments> + <tool_name>
        let text = "<tool_call>\n<tool_arguments>{\"path\":\"/Users/benches\"}</tool_arguments>\n<tool_name>ls</tool_name>\n</tool_call>";
        // First verify parse_native_tool_calls works on the inner content
        let inner = "\n<tool_arguments>{\"path\":\"/Users/benches\"}</tool_arguments>\n<tool_name>ls</tool_name>\n";
        let inner_calls = OpenCodeCliProvider::parse_native_tool_calls(inner);
        assert_eq!(inner_calls.len(), 1, "inner native content should parse");
        assert_eq!(inner_calls[0].1, "ls");
        assert_eq!(inner_calls[0].2["path"], "/Users/benches");

        // Now verify parse_tool_calls handles the wrapped form
        let calls = OpenCodeCliProvider::parse_tool_calls(text);
        assert_eq!(calls.len(), 1, "wrapped form should be parseable via parse_tool_calls");
        assert_eq!(calls[0].1, "ls");
    }

    #[test]
    fn test_xml_tool_request_format() {
        // Exact format from the user's bug report
        let text = r#"<tool_request id="0" tool="read">
  <parameters>
    <file_path>/Users/pradeep.borado/work/scripts/metal-operators/tests/pca_test.rs</file_path>
    <limit>50</limit>
    <offset>1</offset>
  </parameters>
</tool_request>"#;
        let calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        assert_eq!(calls.len(), 1, "should find one tool call in XML format");
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[0].2["file_path"], "/Users/pradeep.borado/work/scripts/metal-operators/tests/pca_test.rs");
        assert_eq!(calls[0].2["limit"], 50);
        assert_eq!(calls[0].2["offset"], 1);
    }

    #[test]
    fn test_xml_tool_request_multiple() {
        let text = r#"<tool_request id="0" tool="read">
  <parameters>
    <file_path>/a.rs</file_path>
  </parameters>
</tool_request>
<tool_request id="1" tool="grep">
  <parameters>
    <pattern>pub fn</pattern>
    <path>/src</path>
  </parameters>
</tool_request>"#;
        let calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        assert_eq!(calls.len(), 2, "should find two tool calls");
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[1].1, "grep");
    }

    #[test]
    fn test_xml_inside_tool_call_wrapper() {
        let text = r#"<tool_call>
<tool_request id="0" tool="read">
  <parameters>
    <file_path>/a.rs</file_path>
  </parameters>
</tool_request>
</tool_call>"#;
        let calls = OpenCodeCliProvider::parse_tool_calls(text);
        assert_eq!(calls.len(), 1, "should extract XML from <tool_call> wrapper");
        assert_eq!(calls[0].1, "read");
    }

    #[test]
    fn test_arg_key_value_webfetch_format() {
        // Exact format emitted by opencode/ling-3.0-flash-free
        let text = r#"<tool_call>webfetch
<arg_key>url</arg_key>
<arg_value>https://blog.google/innovation-and-ai/technology/developers-tools/multi-token-prediction-gemma-4/</arg_value><arg_key>format</arg_key>
<arg_value>markdown</arg_value><arg_key>timeout</arg_key>
<arg_value>30</arg_value>
</tool_call>"#;
        let calls = OpenCodeCliProvider::parse_tool_calls(text);
        assert_eq!(calls.len(), 1, "should parse arg_key/arg_value webfetch call");
        assert_eq!(calls[0].1, "webfetch");
        assert_eq!(
            calls[0].2["url"],
            "https://blog.google/innovation-and-ai/technology/developers-tools/multi-token-prediction-gemma-4/"
        );
        assert_eq!(calls[0].2["format"], "markdown");
        assert_eq!(calls[0].2["timeout"], 30);
    }

    #[test]
    fn test_arg_key_value_bare_body() {
        let text = r#"webfetch
<arg_key>url</arg_key>
<arg_value>https://example.com</arg_value>
"#;
        let calls = OpenCodeCliProvider::parse_native_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "webfetch");
        assert_eq!(calls[0].2["url"], "https://example.com");
    }

    #[test]
    fn test_find_balanced_json_with_braces_in_string() {
        // JSON with braces inside string values
        let s = "{\"code\":\"fn foo() { return 1; }\"}";
        let result = OpenCodeCliProvider::find_balanced_json_in(s, 0);
        assert!(result.is_some());
        let (end, json_str) = result.unwrap();
        assert_eq!(json_str, s);
        assert_eq!(end, s.len());
    }

    #[test]
    fn drops_echoed_assistant_scaffolding() {
        assert!(OpenCodeCliProvider::sanitize_bridge_text("---\n\nASSISTANT: ").is_none());
        assert!(OpenCodeCliProvider::sanitize_bridge_text("\n\n---\n\nASSISTANT: ").is_none());
        let kept = OpenCodeCliProvider::sanitize_bridge_text(
            "---\n\nASSISTANT: Let me check the shader next.",
        )
        .unwrap();
        assert!(kept.contains("Let me check the shader"));
        assert!(!kept.contains("ASSISTANT:"));
        assert!(!kept.contains("---"));
    }

    #[test]
    fn prompt_avoids_triple_dash_separators() {
        let provider = OpenCodeCliProvider::default();
        let req = ChatRequest {
            model: "x".into(),
            messages: vec![Message {
                role: Role::User,
                content: vec![Content {
                    content_type: ContentType::Text,
                    text: Some("hi".into()),
                    ..Default::default()
                }],
            }],
            system: None,
            tools: vec![],
            max_tokens: 100,
            temperature: None,
            top_p: None,
            stop_sequences: None,
            stream: false,
            thinking: None,
        };
        let prompt = provider.build_prompt(&req);
        assert!(!prompt.contains("\n---\n"), "prompt should not use --- separators");
        assert!(prompt.contains("<turn role=\"user\">"));
        assert!(prompt.contains("Never echo"));
    }
}
