use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct WebSearchArgs {
    pub query: String,
    pub num_results: Option<u32>,
    pub livecrawl: Option<String>,
    pub r#type: Option<String>,
    pub context_max_characters: Option<u32>,
}

pub struct WebSearchTool;

const PARALLEL_URL: &str = "https://search.parallel.ai/mcp";
const EXA_URL: &str = "https://mcp.exa.ai/mcp";
const MAX_NUM_RESULTS: u32 = 20;

fn select_provider(session_seed: &str) -> &'static str {
    if let Ok(val) = std::env::var("OPENCODE_WEBSEARCH_PROVIDER") {
        if val == "exa" || val == "parallel" {
            return Box::leak(val.into_boxed_str());
        }
    }
    let seed: u64 = session_seed.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    if seed % 2 == 0 { "exa" } else { "parallel" }
}

#[async_trait]
impl AgentTool for WebSearchTool {
    fn name(&self) -> &str {
        "websearch"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Returns relevant results with snippets and links. Use this for real-time information, recent events, or facts you're unsure about."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "num_results": {
                    "type": "integer",
                    "description": "Number of search results to return (default: 8, max: 20)"
                },
                "livecrawl": {
                    "type": "string",
                    "enum": ["fallback", "preferred"],
                    "description": "Live crawl mode - 'fallback' uses cached content when available, 'preferred' prioritizes live crawling"
                },
                "type": {
                    "type": "string",
                    "enum": ["auto", "fast", "deep"],
                    "description": "Search type - 'auto' balanced, 'fast' quick results, 'deep' comprehensive"
                },
                "context_max_characters": {
                    "type": "integer",
                    "description": "Maximum characters for context (default: 10000)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: WebSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        let num_results = parsed.num_results.unwrap_or(8).min(MAX_NUM_RESULTS);
        let provider = select_provider(&parsed.query);

        let result = match provider {
            "exa" => search_exa(&parsed.query, num_results, &parsed).await,
            _ => search_parallel(&parsed.query, num_results).await,
        };

        match result {
            Ok(text) => ToolExecuteResult::ok(text),
            Err(e) => ToolExecuteResult::error(format!("Search failed: {}", e)),
        }
    }
}

async fn search_parallel(query: &str, num_results: u32) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| format!("Client error: {}", e))?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search",
            "arguments": {
                "objective": query,
                "search_queries": [query],
                "session_id": "rs-agent"
            }
        }
    });

    let resp = client
        .post(PARALLEL_URL)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Read failed: {}", e))?;

    if !status.is_success() {
        return Err(format!("Parallel API returned {}: {}", status.as_u16(), text));
    }

    parse_mcp_response(&text, num_results)
}

async fn search_exa(query: &str, num_results: u32, args: &WebSearchArgs) -> Result<String, String> {
    let api_key = std::env::var("EXA_API_KEY").map_err(|_| "EXA_API_KEY not set".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| format!("Client error: {}", e))?;

    let search_type = args.r#type.as_deref().unwrap_or("auto");
    let livecrawl = args.livecrawl.as_deref().unwrap_or("fallback");
    let context_max = args.context_max_characters.unwrap_or(10_000).min(50_000);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search_exa",
            "arguments": {
                "query": query,
                "type": search_type,
                "numResults": num_results,
                "livecrawl": livecrawl,
                "contextMaxCharacters": context_max
            }
        }
    });

    let resp = client
        .post(EXA_URL)
        .header("content-type", "application/json")
        .header("x-api-key", &api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Read failed: {}", e))?;

    if !status.is_success() {
        return Err(format!("Exa API returned {}: {}", status.as_u16(), text));
    }

    parse_mcp_response(&text, num_results)
}

fn parse_mcp_response(body: &str, _num_results: u32) -> Result<String, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("JSON parse: {}", e))?;

    if let Some(error) = v.get("error") {
        return Err(format!("MCP error: {}", error));
    }

    let content = v
        .pointer("/result/content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "No content in response".to_string())?;

    let mut output = String::new();
    for item in content {
        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
            output.push_str(text);
            output.push('\n');
        }
    }

    if output.trim().is_empty() {
        return Err("No search results found. Please try a different query.".to_string());
    }

    Ok(output.trim().to_string())
}
