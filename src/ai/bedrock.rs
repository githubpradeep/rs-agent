use crate::ai::provider::{BoxStream, Provider};
use crate::ai::types::*;
use async_trait::async_trait;
use futures::StreamExt;
use hmac::{Hmac, Mac};
use reqwest::Client as HttpClient;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct BedrockProvider {
    pub region: String,
    pub name: String,
}

impl Default for BedrockProvider {
    fn default() -> Self {
        Self {
            region: load_region(),
            name: "bedrock".to_string(),
        }
    }
}

impl BedrockProvider {
    pub fn new(region: Option<String>, name: Option<String>) -> Self {
        Self {
            region: region.unwrap_or_else(load_region),
            name: name.unwrap_or_else(|| "bedrock".to_string()),
        }
    }

    fn api_url(&self, model: &str, stream: bool) -> String {
        let path = if stream { "converse-stream" } else { "converse" };
        format!(
            "https://bedrock-runtime.{region}.amazonaws.com/model/{model}/{path}",
            region = self.region,
            model = model,
        )
    }

    async fn send_converse(&self, request: ChatRequest) -> ProviderResult<AssistantMessage> {
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let body = build_converse_body(&request)?;
        let body_bytes = serde_json::to_vec(&body).map_err(|e| ProviderError::Other(e.to_string()))?;

        let url = self.api_url(&request.model, false);

        let credentials = load_credentials()?;
        let signed_request = sign_request(
            &url,
            "POST",
            "application/json",
            &body_bytes,
            &credentials,
            &self.region,
            "bedrock",
        )?;

        let resp = http_client
            .post(&url)
            .headers(signed_request.headers.clone())
            .body(body_bytes.clone())
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
            let text = resp.text().await.unwrap_or_default();
            return Err(if status.as_u16() == 429 {
                ProviderError::RateLimited(60.0)
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                ProviderError::Auth(text)
            } else {
                ProviderError::Http(status.as_u16(), text)
            });
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        parse_converse_response(data)
    }

    async fn send_converse_stream(
        &self,
        request: ChatRequest,
    ) -> ProviderResult<BoxStream> {
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let body = build_converse_body(&request)?;
        let body_bytes = serde_json::to_vec(&body).map_err(|e| ProviderError::Other(e.to_string()))?;

        let url = self.api_url(&request.model, true);

        let credentials = load_credentials()?;
        let signed_request = sign_request(
            &url,
            "POST",
            "application/json",
            &body_bytes,
            &credentials,
            &self.region,
            "bedrock",
        )?;

        let response = http_client
            .post(&url)
            .headers(signed_request.headers.clone())
            .body(body_bytes.clone())
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
            let text = response.text().await.unwrap_or_default();
            return Err(if status.as_u16() == 429 {
                ProviderError::RateLimited(60.0)
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                ProviderError::Auth(text)
            } else {
                ProviderError::Http(status.as_u16(), text)
            });
        }

        let byte_stream = response.bytes_stream();
        let stream = futures::stream::unfold(
            (byte_stream, Vec::new()),
            |(mut stream, mut buf)| async move {
                loop {
                    if buf.len() >= 4 {
                        let total_len = u32::from_be_bytes(buf[..4].try_into().unwrap()) as usize;
                        if buf.len() >= total_len {
                            let msg_bytes = buf[..total_len].to_vec();
                            buf.drain(..total_len);
                            let result = match parse_converse_stream_event(&msg_bytes) {
                                Some(delta) => Ok(delta),
                                None => continue,
                            };
                            return Some((result, (stream, buf)));
                        }
                    }
                    match stream.next().await {
                        Some(Ok(chunk)) => {
                            buf.extend_from_slice(&chunk);
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(ProviderError::Stream(e.to_string())),
                                (stream, buf),
                            ));
                        }
                        None => return None,
                    }
                }
            },
        );

        let boxed: BoxStream = Box::pin(stream);
        Ok(boxed)
    }
}

#[async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn api_key_env_var(&self) -> &str {
        "AWS_ACCESS_KEY_ID"
    }

    fn base_url(&self) -> &str {
        &self.region
    }

    async fn chat(&self, _api_key: &str, request: ChatRequest) -> ProviderResult<AssistantMessage> {
        self.send_converse(request).await
    }

    async fn chat_stream(
        &self,
        _api_key: &str,
        request: ChatRequest,
    ) -> ProviderResult<BoxStream> {
        self.send_converse_stream(request).await
    }

    async fn fetch_models(&self, _api_key: &str) -> ProviderResult<Vec<String>> {
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let url = format!(
            "https://bedrock.{region}.amazonaws.com/foundation-models",
            region = self.region
        );

        let credentials = load_credentials()?;
        let signed_request = sign_request(
            &url,
            "GET",
            "application/json",
            &[],
            &credentials,
            &self.region,
            "bedrock",
        )?;

        let resp = http_client
            .get(&url)
            .headers(signed_request.headers.clone())
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Http(status.as_u16(), text));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let models = data["modelSummaries"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["modelId"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }

    fn supports_thinking(&self) -> bool {
        false
    }

    fn default_max_tokens(&self) -> u32 {
        8192
    }
}

// ── AWS SigV4 signing ─────────────────────────────────────────────

struct AwsCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

struct SignedRequest {
    headers: reqwest::header::HeaderMap,
}

fn aws_config_path() -> std::path::PathBuf {
    std::env::var("AWS_CONFIG_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".aws").join("config")
        })
}

fn aws_credentials_path() -> std::path::PathBuf {
    std::env::var("AWS_SHARED_CREDENTIALS_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".aws").join("credentials")
        })
}

fn load_region() -> String {
    std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .or_else(|_| read_region_from_config())
        .unwrap_or_else(|_| "us-east-1".to_string())
}

fn read_region_from_config() -> Result<String, std::env::VarError> {
    let profile = std::env::var("AWS_PROFILE").unwrap_or_else(|_| "default".to_string());
    let config_path = aws_config_path();
    let text = std::fs::read_to_string(config_path).map_err(|_| std::env::VarError::NotPresent)?;
    let section = if profile == "default" {
        "default".to_string()
    } else {
        format!("profile {}", profile)
    };
    parse_ini_value(&text, &section, "region")
        .ok_or(std::env::VarError::NotPresent)
}

pub fn export_credentials_from_file() {
    let profile = std::env::var("AWS_PROFILE").unwrap_or_else(|_| "default".to_string());
    let cred_path = aws_credentials_path();
    let text = std::fs::read_to_string(cred_path).ok();
    let region = load_region();
    if let Some(ref text) = text {
        if let Some(ak) = parse_ini_value(text, &profile, "aws_access_key_id") {
            std::env::set_var("AWS_ACCESS_KEY_ID", &ak);
        }
        if let Some(sk) = parse_ini_value(text, &profile, "aws_secret_access_key") {
            std::env::set_var("AWS_SECRET_ACCESS_KEY", &sk);
        }
        if let Some(st) = parse_ini_value(text, &profile, "aws_session_token") {
            std::env::set_var("AWS_SESSION_TOKEN", &st);
        }
    }
    std::env::set_var("AWS_REGION", &region);
}

fn load_credentials() -> ProviderResult<AwsCredentials> {
    let profile = std::env::var("AWS_PROFILE").unwrap_or_else(|_| "default".to_string());

    let from_file = || -> Option<AwsCredentials> {
        let cred_path = aws_credentials_path();
        let text = std::fs::read_to_string(cred_path).ok()?;
        let access_key_id = parse_ini_value(&text, &profile, "aws_access_key_id")?;
        let secret_access_key = parse_ini_value(&text, &profile, "aws_secret_access_key")?;
        let session_token = parse_ini_value(&text, &profile, "aws_session_token");
        Some(AwsCredentials {
            access_key_id,
            secret_access_key,
            session_token,
        })
    };

    let from_env = || -> Option<AwsCredentials> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
            .or_else(|_| std::env::var("AWS_ACCESS_KEY")).ok()?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .or_else(|_| std::env::var("AWS_SECRET_KEY")).ok()?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        Some(AwsCredentials { access_key_id, secret_access_key, session_token })
    };

    from_env().or_else(from_file).ok_or_else(|| {
        ProviderError::Auth(
            "AWS credentials not found. Set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY env vars \
             or configure ~/.aws/credentials."
                .to_string(),
        )
    })
}

fn parse_ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_target_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1].trim();
            in_target_section = section == *name;
            continue;
        }
        if in_target_section {
            if let Some(eq) = line.find('=') {
                let k = line[..eq].trim();
                let v = line[eq + 1..].trim();
                if k == key {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn sign_request(
    url: &str,
    method: &str,
    content_type: &str,
    body: &[u8],
    credentials: &AwsCredentials,
    region: &str,
    service: &str,
) -> ProviderResult<SignedRequest> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| ProviderError::Other(e.to_string()))?;

    let timestamp = format_timestamp(now);
    let date = &timestamp[..8];

    let parsed_url = reqwest::Url::parse(url)
        .map_err(|e| ProviderError::Other(e.to_string()))?;
    let host = parsed_url.host_str().unwrap_or("").to_string();
    let canonical_query = parsed_url.query().unwrap_or("");

    // SigV4 canonical URI must use RFC 3986 percent-encoding for all
    // characters except unreserved (A-Z, a-z, 0-9, -, _, ., ~)
    let canonical_uri = percent_encode_path(parsed_url.path());

    let body_hash = hex::encode(Sha256::digest(body));

    let signed_headers_str = if credentials.session_token.is_some() {
        "content-type;host;x-amz-date;x-amz-security-token"
    } else {
        "content-type;host;x-amz-date"
    };

    let mut canonical_headers = format!(
        "content-type:{}\nhost:{}\nx-amz-date:{}\n",
        content_type, host, timestamp,
    );

    if let Some(token) = &credentials.session_token {
        canonical_headers.push_str(&format!("x-amz-security-token:{}\n", token));
    }

    let canonical_request = format!(
        "{method}\n{uri}\n{query}\n{headers}\n{signed}\n{body_hash}",
        method = method,
        uri = canonical_uri,
        query = canonical_query,
        headers = canonical_headers,
        signed = signed_headers_str,
        body_hash = body_hash,
    );

    let algorithm = "AWS4-HMAC-SHA256";
    let credential_scope = format!("{}/{}/{}/aws4_request", date, region, service);
    let string_to_sign = format!(
        "{algorithm}\n{timestamp}\n{scope}\n{hash}",
        algorithm = algorithm,
        timestamp = timestamp,
        scope = credential_scope,
        hash = hex::encode(Sha256::digest(canonical_request.as_bytes())),
    );

    let signing_key = derive_signing_key(&credentials.secret_access_key, date, region, service);
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    let authorization = format!(
        "{algorithm} Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={signature}",
        algorithm = algorithm,
        access_key = credentials.access_key_id,
        scope = credential_scope,
        signed = signed_headers_str,
        signature = signature,
    );

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "content-type",
        reqwest::header::HeaderValue::from_str(content_type)
            .map_err(|e| ProviderError::Other(e.to_string()))?,
    );
    headers.insert(
        "x-amz-date",
        reqwest::header::HeaderValue::from_str(&timestamp)
            .map_err(|e| ProviderError::Other(e.to_string()))?,
    );
    headers.insert(
        "authorization",
        reqwest::header::HeaderValue::from_str(&authorization)
            .map_err(|e| ProviderError::Other(e.to_string()))?,
    );
    headers.insert(
        "host",
        reqwest::header::HeaderValue::from_str(&host)
            .map_err(|e| ProviderError::Other(e.to_string()))?,
    );

    if let Some(token) = &credentials.session_token {
        headers.insert(
            "x-amz-security-token",
            reqwest::header::HeaderValue::from_str(token)
                .map_err(|e| ProviderError::Other(e.to_string()))?,
        );
    }

    Ok(SignedRequest { headers })
}

fn format_timestamp(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let micros = duration.subsec_micros();
    chrono::DateTime::from_timestamp(secs as i64, micros * 1000)
        .unwrap()
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_secret = format!("AWS4{}", secret);
    let k_date = hmac_sha256(k_secret.as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key length OK");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn percent_encode_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

// ── Bedrock Converse API body builder ─────────────────────────────

fn build_converse_body(request: &ChatRequest) -> ProviderResult<serde_json::Value> {
    let mut body = serde_json::Map::new();

    let messages = convert_messages(&request.messages, &request.system)?;
    body.insert("messages".to_string(), serde_json::json!(messages));

    if let Some(system) = &request.system {
        if !system.is_empty() {
            body.insert("system".to_string(), serde_json::json!([{"text": system}]));
        }
    }

    let mut inference_config = serde_json::json!({
        "maxTokens": request.max_tokens,
    });

    if let Some(top_p) = request.top_p {
        inference_config["topP"] = serde_json::json!(top_p);
    }

    body.insert("inferenceConfig".to_string(), inference_config);

    if !request.tools.is_empty() {
        let tools: Vec<serde_json::Value> = request
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "toolSpec": {
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": {
                            "json": t.input_schema
                        }
                    }
                })
            })
            .collect();
        body.insert("toolConfig".to_string(), serde_json::json!({
            "tools": tools
        }));
    }

    Ok(serde_json::Value::Object(body))
}

fn convert_messages(
    messages: &[Message],
    system: &Option<String>,
) -> ProviderResult<Vec<serde_json::Value>> {
    let mut result = Vec::new();
    let mut system_text = system.clone().unwrap_or_default();

    for msg in messages {
        if let Role::System = msg.role {
            for c in &msg.content {
                if let Some(text) = &c.text {
                    system_text.push('\n');
                    system_text.push_str(text);
                }
            }
            continue;
        }

        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "user",
            Role::System => continue,
        };

        let mut content = Vec::new();
        for c in &msg.content {
            match c.content_type {
                ContentType::Text => {
                    if let Some(text) = &c.text {
                        content.push(serde_json::json!({
                            "text": text
                        }));
                    }
                }
                ContentType::ToolUse => {
                    content.push(serde_json::json!({
                        "toolUse": {
                            "toolUseId": c.id.as_deref().unwrap_or(""),
                            "name": c.name.as_deref().unwrap_or(""),
                            "input": c.input.as_ref().unwrap_or(&serde_json::Value::Null)
                        }
                    }));
                }
                ContentType::ToolResult => {
                    content.push(serde_json::json!({
                        "toolResult": {
                            "toolUseId": c.tool_use_id.as_deref().unwrap_or(""),
                            "content": [{"text": c.text.as_deref().unwrap_or("")}],
                            "status": "success"
                        }
                    }));
                }
                ContentType::Thinking | ContentType::RedactedThinking => {
                    if let Some(t) = &c.thinking {
                        content.push(serde_json::json!({
                            "text": t
                        }));
                    }
                }
            }
        }

        if !content.is_empty() {
            result.push(serde_json::json!({
                "role": role,
                "content": content
            }));
        }
    }

    Ok(result)
}

// ── Response parsing ──────────────────────────────────────────────

fn parse_converse_response(data: serde_json::Value) -> ProviderResult<AssistantMessage> {
    let mut content = Vec::new();

    if let Some(blocks) = data["output"]["message"]["content"].as_array() {
        for block in blocks {
            if let Some(text) = block["text"].as_str() {
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
            if let Some(tool_use) = block["toolUse"].as_object() {
                content.push(Content {
                    content_type: ContentType::ToolUse,
                    text: None,
                    id: tool_use["toolUseId"].as_str().map(|s| s.to_string()),
                    name: tool_use["name"].as_str().map(|s| s.to_string()),
                    input: Some(tool_use["input"].clone()),
                    tool_use_id: None,
                    content: None,
                    signature: None,
                    thinking: None,
                });
            }
        }
    }

    let stop_reason = data["stopReason"].as_str().map(|r| match r {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "content_filtered" => StopReason::ContentFiltered,
        "stop_sequence" => StopReason::StopSequence,
        r => StopReason::Other(r.to_string()),
    });

    let usage = data["usage"].as_object().map(|u| Usage {
        input_tokens: u["inputTokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["outputTokens"].as_u64().unwrap_or(0) as u32,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    });

    Ok(AssistantMessage {
        content,
        stop_reason,
        usage,
        model: data["model"].as_str().unwrap_or("unknown").to_string(),
        id: data["messageId"]
            .as_str()
            .or_else(|| data["output"]["message"]["id"].as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_event_stream_message(data: &[u8]) -> Option<(String, serde_json::Value)> {
    if data.len() < 12 {
        return None;
    }

    let total_length = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
    let headers_length = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;

    if data.len() < total_length {
        return None;
    }

    let mut pos: usize = 12;
    let headers_end = pos + headers_length;
    let mut event_type = String::new();

    while pos + 1 < headers_end {
        let name_len = data[pos] as usize;
        pos += 1;
        if pos + name_len > headers_end {
            break;
        }
        let name = std::str::from_utf8(&data[pos..pos + name_len]).ok()?;
        pos += name_len;

        if pos >= headers_end {
            break;
        }
        let value_type = data[pos];
        pos += 1;

        let value = parse_header_value(data, &mut pos, value_type, headers_end);
        if name == ":event-type" {
            if let Some(val) = value.and_then(|v| v.as_str().map(|s| s.to_string())) {
                event_type = val;
            }
        }
    }

    if event_type.is_empty() {
        return None;
    }

    let payload_start = pos;
    let payload_end = total_length - 4;

    if payload_start >= payload_end {
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&data[payload_start..payload_end]).ok()?;
    Some((event_type, json))
}

fn parse_header_value(
    data: &[u8],
    pos: &mut usize,
    value_type: u8,
    end: usize,
) -> Option<serde_json::Value> {
    match value_type {
        0 => Some(serde_json::Value::Bool(true)),
        1 => Some(serde_json::Value::Bool(false)),
        2 => {
            if *pos + 1 > end {
                return None;
            }
            let v = data[*pos];
            *pos += 1;
            Some(serde_json::json!(v))
        }
        3 => {
            if *pos + 2 > end {
                return None;
            }
            let v = i16::from_be_bytes(data[*pos..*pos + 2].try_into().ok()?);
            *pos += 2;
            Some(serde_json::json!(v))
        }
        4 => {
            if *pos + 4 > end {
                return None;
            }
            let v = i32::from_be_bytes(data[*pos..*pos + 4].try_into().ok()?);
            *pos += 4;
            Some(serde_json::json!(v))
        }
        5 => {
            if *pos + 8 > end {
                return None;
            }
            let v = i64::from_be_bytes(data[*pos..*pos + 8].try_into().ok()?);
            *pos += 8;
            Some(serde_json::json!(v))
        }
        6 | 7 => {
            if *pos + 2 > end {
                return None;
            }
            let len = u16::from_be_bytes(data[*pos..*pos + 2].try_into().ok()?) as usize;
            *pos += 2;
            if *pos + len > end {
                return None;
            }
            if value_type == 6 {
                let bytes = data[*pos..*pos + len].to_vec();
                *pos += len;
                Some(serde_json::Value::Array(
                    bytes.into_iter().map(|b| serde_json::json!(b)).collect(),
                ))
            } else {
                let s = std::str::from_utf8(&data[*pos..*pos + len])
                    .ok()?
                    .to_string();
                *pos += len;
                Some(serde_json::Value::String(s))
            }
        }
        8 => {
            if *pos + 8 > end {
                return None;
            }
            *pos += 8;
            Some(serde_json::Value::Null)
        }
        9 => {
            if *pos + 16 > end {
                return None;
            }
            *pos += 16;
            Some(serde_json::Value::Null)
        }
        _ => None,
    }
}

fn parse_converse_stream_event(data: &[u8]) -> Option<StreamDelta> {
    let (event_type, value) = parse_event_stream_message(data)?;

    match event_type.as_str() {
        "contentBlockStart" => {
            let index = value["contentBlockIndex"].as_u64().unwrap_or(0) as u32;
            if let Some(tool_use) = value["start"]["toolUse"].as_object() {
                Some(StreamDelta {
                    content_index: index,
                    r#type: DeltaType::ToolCallStart {
                        id: tool_use["toolUseId"].as_str().unwrap_or("").to_string(),
                        name: tool_use["name"].as_str().unwrap_or("").to_string(),
                        input: String::new(),
                    },
                })
            } else {
                None
            }
        }
        "contentBlockDelta" => {
            let index = value["contentBlockIndex"].as_u64().unwrap_or(0) as u32;
            let delta = &value["delta"];
            if let Some(text) = delta["text"].as_str() {
                Some(StreamDelta {
                    content_index: index,
                    r#type: DeltaType::Text {
                        text: text.to_string(),
                    },
                })
            } else if delta["toolUse"].is_object() {
                Some(StreamDelta {
                    content_index: index,
                    r#type: DeltaType::ToolCallDelta {
                        input: delta["toolUse"]["input"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                    },
                })
            } else {
                None
            }
        }
        "messageStop" => {
            let stop_reason = value["stopReason"].as_str().map(|r| match r {
                "end_turn" => StopReason::EndTurn,
                "tool_use" => StopReason::ToolUse,
                "max_tokens" => StopReason::MaxTokens,
                "content_filtered" => StopReason::ContentFiltered,
                "stop_sequence" => StopReason::StopSequence,
                r => StopReason::Other(r.to_string()),
            });
            Some(StreamDelta {
                content_index: 0,
                r#type: DeltaType::Stop { stop_reason },
            })
        }
        _ => None,
    }
}
