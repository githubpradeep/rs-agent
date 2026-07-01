use crate::agent::tool::*;
use async_trait::async_trait;
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct WebFetchArgs {
    pub url: String,
    pub format: Option<String>,
    pub timeout: Option<u64>,
}

pub struct WebFetchTool;

const MAX_RESPONSE_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_TIMEOUT: u64 = 30;
const MAX_TIMEOUT: u64 = 120;

fn browser_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
}

fn html_to_text(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut text = String::new();
    let body_sel = Selector::parse("body").unwrap();
    if let Some(body) = doc.select(&body_sel).next() {
        extract_text(body, &mut text);
    } else {
        extract_text(doc.root_element(), &mut text);
    }
    let cleaned: Vec<&str> = text.split_whitespace().collect();
    cleaned.join(" ")
}

fn extract_text(node: scraper::ElementRef, out: &mut String) {
    for child in node.children() {
        match child.value() {
            scraper::node::Node::Text(t) => {
                let txt = t.text.trim();
                if !txt.is_empty() {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push(' ');
                    }
                    out.push_str(txt);
                }
            }
            scraper::node::Node::Element(_) => {
                if let Some(el) = scraper::ElementRef::wrap(child.clone()) {
                    let tag = el.value().name();
                    if matches!(tag, "script" | "style" | "noscript" | "iframe" | "object" | "embed") {
                        continue;
                    }
                    if matches!(tag, "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr" | "blockquote" | "hr" | "table" | "ul" | "ol") {
                        if !out.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                    if matches!(tag, "td" | "th") {
                        if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
                            out.push_str("  ");
                        }
                    }
                    if tag == "a" {
                        let href = el.value().attr("href").unwrap_or("");
                        let mut link_text = String::new();
                        extract_text(el, &mut link_text);
                        let trimmed = link_text.trim();
                        if !trimmed.is_empty() && !href.is_empty() {
                            out.push_str(&format!("{} [{}]", trimmed, href));
                        } else if !trimmed.is_empty() {
                            out.push_str(trimmed);
                        }
                    } else {
                        extract_text(el, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn html_to_markdown(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut md = String::new();
    let body_sel = Selector::parse("body").unwrap();
    if let Some(body) = doc.select(&body_sel).next() {
        node_to_markdown(body, &mut md, 0);
    } else {
        node_to_markdown(doc.root_element(), &mut md, 0);
    }
    md.trim().to_string()
}

fn node_to_markdown(node: scraper::ElementRef, out: &mut String, depth: usize) {
    for child in node.children() {
        match child.value() {
            scraper::node::Node::Text(t) => {
                let txt = t.text.trim();
                if !txt.is_empty() {
                    out.push_str(txt);
                }
            }
            scraper::node::Node::Element(_) => {
                if let Some(el) = scraper::ElementRef::wrap(child.clone()) {
                    let tag = el.value().name();
                    match tag {
                        "script" | "style" | "noscript" | "iframe" | "object" | "embed" => continue,
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                            let level = tag[1..].parse::<usize>().unwrap_or(1);
                            let prefix = "#".repeat(level);
                            let mut content = String::new();
                            node_to_markdown(el, &mut content, depth + 1);
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                out.push_str(&format!("\n{} {}\n\n", prefix, trimmed));
                            }
                        }
                        "p" => {
                            let mut content = String::new();
                            node_to_markdown(el, &mut content, depth + 1);
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                out.push_str(&format!("\n{}\n\n", trimmed));
                            }
                        }
                        "br" => {
                            out.push_str("\n");
                        }
                        "hr" => {
                            out.push_str("\n---\n\n");
                        }
                        "ul" | "ol" => {
                            out.push('\n');
                            let is_ol = tag == "ol";
                            let mut idx = 1;
                            for li_child in el.children() {
                                if let Some(li_el) = scraper::ElementRef::wrap(li_child.clone()) {
                                    if li_el.value().name() == "li" {
                                        let mut content = String::new();
                                        node_to_markdown(li_el, &mut content, depth + 1);
                                        let trimmed = content.trim();
                                        if !trimmed.is_empty() {
                                            let bullet = if is_ol {
                                                let n = idx;
                                                idx += 1;
                                                format!("{}.", n)
                                            } else {
                                                "-".to_string()
                                            };
                                            out.push_str(&format!("{} {}\n", bullet, trimmed));
                                        }
                                    }
                                }
                            }
                            out.push('\n');
                        }
                        "li" => {
                            // Handled by ul/ol
                            node_to_markdown(el, out, depth);
                        }
                        "blockquote" => {
                            let mut content = String::new();
                            node_to_markdown(el, &mut content, depth + 1);
                            for line in content.trim().lines() {
                                out.push_str(&format!("> {}\n", line));
                            }
                            out.push('\n');
                        }
                        "pre" => {
                            let mut code = String::new();
                            for code_child in el.children() {
                                if let Some(code_el) = scraper::ElementRef::wrap(code_child.clone()) {
                                    if code_el.value().name() == "code" {
                                        node_to_markdown(code_el, &mut code, depth + 1);
                                    }
                                } else if let scraper::node::Node::Text(t) = code_child.value() {
                                    code.push_str(&t.text);
                                }
                            }
                            let trimmed = code.trim();
                            if !trimmed.is_empty() {
                                out.push_str(&format!("\n```\n{}\n```\n\n", trimmed));
                            }
                        }
                        "code" => {
                            // Check if parent is <pre> (handled above)
                            let is_in_pre = el.parent().and_then(|p| {
                                scraper::ElementRef::wrap(p).map(|pe| pe.value().name() == "pre")
                            }).unwrap_or(false);
                            if !is_in_pre {
                                let mut content = String::new();
                                node_to_markdown(el, &mut content, depth + 1);
                                let trimmed = content.trim();
                                if !trimmed.is_empty() {
                                    out.push_str(&format!("`{}`", trimmed));
                                }
                            }
                        }
                        "strong" | "b" => {
                            let mut content = String::new();
                            node_to_markdown(el, &mut content, depth + 1);
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                out.push_str(&format!("**{}**", trimmed));
                            }
                        }
                        "em" | "i" => {
                            let mut content = String::new();
                            node_to_markdown(el, &mut content, depth + 1);
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                out.push_str(&format!("*{}*", trimmed));
                            }
                        }
                        "a" => {
                            let href = el.value().attr("href").unwrap_or("");
                            let mut content = String::new();
                            node_to_markdown(el, &mut content, depth + 1);
                            let trimmed = content.trim();
                            if !trimmed.is_empty() && !href.is_empty() {
                                out.push_str(&format!("[{}]({})", trimmed, href));
                            } else if !trimmed.is_empty() {
                                out.push_str(trimmed);
                            }
                        }
                        "img" => {
                            let src = el.value().attr("src").unwrap_or("");
                            let alt = el.value().attr("alt").unwrap_or("");
                            if !src.is_empty() {
                                out.push_str(&format!("![{}]({})", alt, src));
                            }
                        }
                        "table" => {
                            let mut content = String::new();
                            node_to_markdown(el, &mut content, depth + 1);
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                out.push_str(&format!("\n{}\n\n", trimmed));
                            }
                        }
                        "tr" => {
                            let mut cells = Vec::new();
                            for cell_child in el.children() {
                                if let Some(cell_el) = scraper::ElementRef::wrap(cell_child.clone()) {
                                    let tag_name = cell_el.value().name();
                                    if tag_name == "td" || tag_name == "th" {
                                        let mut content = String::new();
                                        node_to_markdown(cell_el, &mut content, depth + 1);
                                        cells.push(content.trim().to_string());
                                    }
                                }
                            }
                            if !cells.is_empty() {
                                out.push_str(&format!("| {} |\n", cells.join(" | ")));
                            }
                        }
                        "div" | "span" | "section" | "article" | "main" | "header" | "footer" | "nav" => {
                            node_to_markdown(el, out, depth);
                        }
                        _ => {
                            node_to_markdown(el, out, depth);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl AgentTool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        "Fetch and retrieve the content of a URL. Returns the content as text, markdown, or raw HTML. Use this to read web pages, documentation, APIs, and other online resources."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The HTTP or HTTPS URL to fetch content from"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "description": "Output format: 'text' for plain text, 'markdown' for markdown conversion, 'html' for raw HTML (default: 'markdown')"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 120)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: WebFetchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        // Validate URL
        if !parsed.url.starts_with("http://") && !parsed.url.starts_with("https://") {
            return ToolExecuteResult::error("Only http:// and https:// URLs are supported".to_string());
        }

        let timeout_secs = parsed.timeout.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
        let format = parsed.format.as_deref().unwrap_or("markdown");

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .danger_accept_invalid_certs(false)
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolExecuteResult::error(format!("Client error: {}", e)),
        };

        let resp = match client
            .get(&parsed.url)
            .header("user-agent", browser_user_agent())
            .header("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolExecuteResult::error(format!("Request failed: {}", e)),
        };

        let status = resp.status();
        if !status.is_success() {
            if status.as_u16() == 403 {
                let resp2 = match client
                    .get(&parsed.url)
                    .header("user-agent", "opencode")
                    .header("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => return ToolExecuteResult::error(format!("Request failed: {}", e)),
                };
                if !resp2.status().is_success() {
                    return ToolExecuteResult::error(format!("HTTP {}: {}", resp2.status().as_u16(), resp2.text().await.unwrap_or_default().chars().take(200).collect::<String>()));
                }
                return process_response(resp2, format, &parsed.url).await;
            }
            return ToolExecuteResult::error(format!("HTTP {}: {}", status.as_u16(), resp.text().await.unwrap_or_default().chars().take(200).collect::<String>()));
        }

        return process_response(resp, format, &parsed.url).await;
    }
}

async fn process_response(resp: reqwest::Response, format: &str, url: &str) -> ToolExecuteResult {
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Check content length
    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES {
            return ToolExecuteResult::error(format!("Response too large: {} bytes (max {})", len, MAX_RESPONSE_BYTES));
        }
    }

    let body = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return ToolExecuteResult::error(format!("Read failed: {}", e)),
    };

    if body.len() > MAX_RESPONSE_BYTES as usize {
        return ToolExecuteResult::error(format!("Response too large: {} bytes (max {})", body.len(), MAX_RESPONSE_BYTES));
    }

    // Reject images
    if content_type.starts_with("image/") {
        return ToolExecuteResult::error(format!("Cannot fetch images (got {})", content_type));
    }

    let text = match String::from_utf8(body.to_vec()) {
        Ok(t) => t,
        Err(_) => return ToolExecuteResult::error("Response is not valid UTF-8 text".to_string()),
    };

    let output = if content_type.contains("text/html") {
        match format {
            "text" => html_to_text(&text),
            "markdown" => html_to_markdown(&text),
            _ => text,
        }
    } else {
        // Non-HTML: return raw text
        text
    };

    ToolExecuteResult::ok(format!("Content from {}:\n\n{}", url, output.trim()))
}
