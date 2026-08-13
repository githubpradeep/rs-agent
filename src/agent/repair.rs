//! Tool-call repair — resolve names, coerce args, surface rewrite hints (OpenCode-style).

use crate::agent::registry::ToolRegistry;
use crate::agent::tool::SharedTool;
use serde_json::{json, Map, Value};

/// Sentinel keys when streamed tool arguments failed to parse as JSON.
pub const ARG_PARSE_ERROR_KEY: &str = "__rs_agent_arg_parse_error";
pub const ARG_PARSE_RAW_KEY: &str = "__rs_agent_arg_raw";

/// Common wrong names → canonical tool names (weak / cross-harness models).
const NAME_ALIASES: &[(&str, &str)] = &[
    ("shell", "bash"),
    ("run", "bash"),
    ("exec", "bash"),
    ("terminal", "bash"),
    ("command", "bash"),
    ("search", "grep"),
    ("rg", "grep"),
    ("glob", "find"),
    ("list", "ls"),
    ("listdir", "ls"),
    ("dir", "ls"),
    ("create", "write"),
    ("create_file", "write"),
    ("write_file", "write"),
    ("str_replace", "edit"),
    ("strreplace", "edit"),
    ("replace", "edit"),
    ("search_replace", "edit"),
    ("patch", "apply_patch"),
    ("apply_patch", "apply_patch"),
    ("cat", "read"),
    ("open", "read"),
    ("view", "read"),
    ("fetch", "webfetch"),
    ("web_fetch", "webfetch"),
    ("web_search", "websearch"),
    ("search_web", "websearch"),
    ("python", "repl"),
    ("code_execution", "repl"),
    ("todo", "todowrite"),
    ("todos", "todowrite"),
    ("todo_write", "todowrite"),
    ("update_todo", "todowrite"),
    ("ask", "question"),
    ("ask_user", "question"),
    ("clarify", "question"),
    ("subagent", "task"),
    ("agent_query", "task"),
    ("delegate", "task"),
    ("handoff_notes", "handoff"),
    ("pass_off", "handoff"),
    ("beads", "bead"),
    ("issue", "bead"),
    ("refuse", "escalate"),
    ("needs_human", "escalate"),
];

/// Heuristic: free/tiny/flash models need stricter harness behavior.
pub fn is_weak_model(model: &str) -> bool {
    let m = model.to_lowercase();
    let needles = [
        "flash",
        "-free",
        "/free",
        ":free",
        "mini",
        "nano",
        "tiny",
        "lite",
        "haiku",
        "gemma-2b",
        "gemma-7b",
        "qwen2.5-7b",
        "qwen2.5-14b",
        "llama-3.1-8b",
        "llama-3.2-1b",
        "llama-3.2-3b",
        "openrouter/free",
    ];
    needles.iter().any(|n| m.contains(n))
}

/// Short user-facing warning for free/tiny models.
pub fn weak_model_user_warning(model: &str) -> Option<String> {
    if !is_weak_model(model) {
        return None;
    }
    Some(format!(
        "⚠ Weak/free model `{model}`: expect thrash on edits/builds. \
         Prefer anthropic/claude-sonnet or bedrock opus for real coding. \
         Weak-model mode (sequential tools + repair) is on."
    ))
}

/// Extra system note injected for weak models.
pub fn weak_model_system_note() -> &'static str {
    r#"WEAK-MODEL MODE (harness):
- Call ONE tool at a time; wait for the result before the next tool.
- Use exact tool names: read, write, edit, bash, grep, ls, find, webfetch, websearch, repl.
- write requires {"file_path":"...","content":"..."}. edit requires file_path, old_string, new_string.
- If a tool returns an error, FIX the arguments and retry — do not invent success.
- Prefer short bash commands; prefer edit over rewriting whole files."#
}

/// Resolve a model-supplied tool name to a registered tool (aliases + case).
pub fn resolve_tool<'a>(registry: &'a ToolRegistry, name: &str) -> Result<&'a SharedTool, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(unknown_tool_message(registry, "(empty)"));
    }
    if let Some(t) = registry.get(trimmed) {
        return Ok(t);
    }
    let lower = trimmed.to_lowercase();
    if let Some(t) = registry.get(&lower) {
        return Ok(t);
    }
    // Alias table
    if let Some((_, canon)) = NAME_ALIASES.iter().find(|(a, _)| *a == lower.as_str()) {
        if let Some(t) = registry.get(canon) {
            return Ok(t);
        }
    }
    // Case-insensitive scan
    for t in registry.iter() {
        if t.name().eq_ignore_ascii_case(trimmed) {
            return Ok(t);
        }
    }
    // Prefix / contains fuzzy: unique match only
    let candidates: Vec<_> = registry
        .iter()
        .filter(|t| {
            let n = t.name().to_lowercase();
            n.starts_with(&lower) || lower.starts_with(&n) || n.contains(&lower)
        })
        .collect();
    if candidates.len() == 1 {
        return Ok(candidates[0]);
    }
    Err(unknown_tool_message(registry, trimmed))
}

fn unknown_tool_message(registry: &ToolRegistry, name: &str) -> String {
    let mut names: Vec<_> = registry.iter().map(|t| t.name().to_string()).collect();
    names.sort();
    format!(
        "Unknown tool `{name}`. Available tools: {}.\n\
         Rewrite the tool call using one of these exact names and valid JSON arguments.\n\
         Example: write({{\"file_path\":\"path/to/file\",\"content\":\"...\"}})",
        names.join(", ")
    )
}

/// True when streamed args were not valid JSON.
pub fn is_arg_parse_error(input: &Value) -> Option<(String, String)> {
    let obj = input.as_object()?;
    let err = obj.get(ARG_PARSE_ERROR_KEY)?.as_str()?.to_string();
    let raw = obj
        .get(ARG_PARSE_RAW_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((err, raw))
}

pub fn make_arg_parse_error_value(err: &str, raw: &str) -> Value {
    json!({
        ARG_PARSE_ERROR_KEY: err,
        ARG_PARSE_RAW_KEY: raw,
    })
}

/// Normalize + soft-validate against the tool's JSON schema `required` list.
/// Returns Ok(normalized_args) or Err(repair message for the model).
pub fn prepare_tool_args(
    tool: &dyn crate::agent::tool::AgentTool,
    args: Value,
) -> Result<Value, String> {
    let mut args = crate::tools::normalize_file_tool_args(args);
    // Also apply generic aliases for bash etc.
    args = normalize_generic_args(args, tool.name());

    if let Some((err, raw)) = is_arg_parse_error(&args) {
        return Err(format!(
            "Tool `{name}` arguments were not valid JSON ({err}).\n\
             Raw input (truncated): {raw}\n\
             Resend the tool call with a single JSON object matching the schema.\n\
             Schema required fields: {req}",
            name = tool.name(),
            raw = truncate_chars(&raw, 500),
            req = required_fields_hint(&tool.input_schema()),
        ));
    }

    let missing = missing_required(&tool.input_schema(), &args);
    if !missing.is_empty() {
        return Err(format!(
            "Tool `{}` called with invalid arguments: missing required field(s): {}.\n\
             Got keys: {}.\n\
             Fix the JSON and call `{}` again. {}",
            tool.name(),
            missing.join(", "),
            arg_keys(&args),
            tool.name(),
            schema_hint(&tool.input_schema()),
        ));
    }
    Ok(args)
}

fn normalize_generic_args(args: Value, tool_name: &str) -> Value {
    let Value::Object(mut map) = args else {
        return args;
    };
    match tool_name {
        "bash" => {
            alias_into(&mut map, "command", &["cmd", "script", "code", "input"]);
        }
        "grep" => {
            alias_into(&mut map, "pattern", &["query", "regex", "search", "q"]);
            alias_into(&mut map, "path", &["file", "file_path", "dir", "directory"]);
        }
        "find" => {
            alias_into(&mut map, "pattern", &["glob", "query", "name"]);
            alias_into(&mut map, "path", &["dir", "directory", "file_path"]);
        }
        "ls" => {
            alias_into(&mut map, "path", &["dir", "directory", "file_path"]);
        }
        "webfetch" => {
            alias_into(&mut map, "url", &["uri", "link", "href"]);
        }
        "websearch" => {
            alias_into(&mut map, "query", &["q", "search", "prompt", "text"]);
        }
        "repl" => {
            alias_into(&mut map, "code", &["python", "script", "source", "command"]);
        }
        _ => {}
    }
    Value::Object(map)
}

fn alias_into(map: &mut Map<String, Value>, canonical: &str, aliases: &[&str]) {
    if map.get(canonical).map(|v| !v.is_null()).unwrap_or(false) {
        return;
    }
    for alias in aliases {
        if let Some(v) = map.remove(*alias) {
            if !v.is_null() {
                map.insert(canonical.to_string(), v);
                return;
            }
        }
    }
}

fn required_fields_hint(schema: &Value) -> String {
    match schema.get("required").and_then(|r| r.as_array()) {
        Some(arr) if !arr.is_empty() => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => "(see tool schema)".into(),
    }
}

fn missing_required(schema: &Value, args: &Value) -> Vec<String> {
    let Some(req) = schema.get("required").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    let obj = args.as_object();
    req.iter()
        .filter_map(|v| v.as_str())
        .filter(|k| {
            obj.map(|o| {
                o.get(*k)
                    .map(|v| v.is_null() || (v.is_string() && v.as_str() == Some("")))
                    .unwrap_or(true)
            })
            .unwrap_or(true)
        })
        .map(|s| s.to_string())
        .collect()
}

fn arg_keys(args: &Value) -> String {
    args.as_object()
        .map(|m| {
            let mut keys: Vec<_> = m.keys().cloned().collect();
            keys.sort();
            if keys.is_empty() {
                "(none)".into()
            } else {
                keys.join(", ")
            }
        })
        .unwrap_or_else(|| "(not an object)".into())
}

fn schema_hint(schema: &Value) -> String {
    let props = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|p| p.keys().cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    if props.is_empty() {
        String::new()
    } else {
        format!("Properties: {props}.")
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

/// Hash tool name + args for doom-loop detection.
pub fn tool_call_fingerprint(name: &str, args: &Value) -> String {
    format!("{name}|{args}")
}

/// Coarse key for near-duplicate thrash: tool + primary path / command prefix.
pub fn tool_near_dupe_key(name: &str, args: &Value) -> String {
    let primary = match name {
        "edit" | "write" | "apply_patch" | "read" => args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "bash" => args
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| {
                let t = c.trim();
                t.chars().take(80).collect::<String>()
            })
            .unwrap_or_default(),
        "grep" => format!(
            "{}|{}",
            args.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
            args.get("path").and_then(|v| v.as_str()).unwrap_or("")
        ),
        _ => String::new(),
    };
    format!("{name}|{primary}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool::{AgentTool, ToolExecuteResult};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct DummyWrite;
    #[async_trait]
    impl AgentTool for DummyWrite {
        fn name(&self) -> &str {
            "write"
        }
        fn description(&self) -> &str {
            "write"
        }
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["file_path", "content"]
            })
        }
        async fn execute(&self, _: &str, _: Value) -> ToolExecuteResult {
            ToolExecuteResult::ok("ok")
        }
    }

    #[test]
    fn weak_model_detects_flash_free() {
        assert!(is_weak_model("opencode/deepseek-v4-flash-free"));
        assert!(is_weak_model("claude-haiku-4"));
        assert!(!is_weak_model("claude-opus-4-6"));
        assert!(!is_weak_model("us.anthropic.claude-sonnet-4-5"));
    }

    #[test]
    fn resolves_alias_and_case() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(DummyWrite));
        assert_eq!(resolve_tool(&reg, "WRITE").unwrap().name(), "write");
        assert_eq!(resolve_tool(&reg, "create_file").unwrap().name(), "write");
        assert!(resolve_tool(&reg, "nope").is_err());
    }

    #[test]
    fn prepare_args_accepts_path_alias() {
        let tool = DummyWrite;
        let args = json!({"path": "a.py", "contents": "x"});
        let out = prepare_tool_args(&tool, args).unwrap();
        assert_eq!(out["file_path"], "a.py");
        assert_eq!(out["content"], "x");
    }

    #[test]
    fn prepare_args_rejects_missing_required() {
        let tool = DummyWrite;
        let err = prepare_tool_args(&tool, json!({"file_path": "a.py"})).unwrap_err();
        assert!(err.contains("content"));
        assert!(err.contains("missing"));
    }

    #[test]
    fn prepare_args_rejects_parse_error_sentinel() {
        let tool = DummyWrite;
        let v = make_arg_parse_error_value("eof", "{not json");
        let err = prepare_tool_args(&tool, v).unwrap_err();
        assert!(err.contains("not valid JSON"));
    }
}
