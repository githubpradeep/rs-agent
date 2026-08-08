use crate::ai::types::{ContentType, Message, Role};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Parent session id when this session was created via `/fork`.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Optional short label for a forked branch.
    #[serde(default)]
    pub branch_label: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub provider: String,
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
    /// Snapshot of the Deep Context call tree (`CallTreeInner`, serialized) at the
    /// end of the last turn, if any. Lets `/tree` show a last-known summary
    /// after resuming a session, even before a new turn has run.
    #[serde(default)]
    pub call_tree: Option<serde_json::Value>,
    /// In-session todo list from the `todowrite` tool.
    #[serde(default)]
    pub todos: Option<Vec<crate::tools::todowrite::TodoItem>>,
    /// Session-scoped `/goal` (restored on `--resume` if still active/paused).
    #[serde(default)]
    pub goal: Option<crate::agent::goal::GoalState>,
    /// Bound seat name for persistent identity.
    #[serde(default)]
    pub seat: Option<String>,
    /// Last handoff notes (agent-authored continuity).
    #[serde(default)]
    pub handoff: Option<crate::agent::handoff::HandoffNotes>,
}

impl SessionData {
    /// Derive a short title from the first user message, if any. Returns
    /// `None` if there's no user message with text content yet.
    pub fn auto_title_from_messages(&self) -> Option<String> {
        auto_title_from_messages(&self.messages)
    }

    /// Set `title` from the first user message if it isn't already set.
    /// No-op if `title` is already `Some` or no user message exists yet.
    pub fn ensure_title(&mut self) {
        if self.title.is_none() {
            self.title = self.auto_title_from_messages();
        }
    }
}

/// Derive a short (<=60 char) title from the first user message's text.
fn auto_title_from_messages(messages: &[Message]) -> Option<String> {
    let first_user_text = messages.iter().find_map(|m| {
        if m.role != Role::User {
            return None;
        }
        m.content.iter().find_map(|c| c.text.clone())
    })?;

    let collapsed: String = first_user_text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }

    const MAX_LEN: usize = 60;
    let title: String = trimmed.chars().take(MAX_LEN).collect();
    if trimmed.chars().count() > MAX_LEN {
        Some(format!("{}…", title))
    } else {
        Some(title)
    }
}

/// Lightweight summary of a saved session, suitable for listing UIs
/// (`/sessions`, `--list-sessions`) without loading the full transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub model: String,
    pub updated_at: String,
    pub message_count: usize,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub branch_label: Option<String>,
}

/// Render a session transcript as a Markdown document (for export/sharing).
pub fn export_markdown(data: &SessionData) -> String {
    let mut out = String::new();
    let title = data.title.clone().unwrap_or_else(|| data.id.clone());
    out.push_str(&format!("# {}\n\n", title));
    out.push_str(&format!("- **Session ID:** {}\n", data.id));
    out.push_str(&format!("- **Model:** {} ({})\n", data.model, data.provider));
    out.push_str(&format!("- **Created:** {}\n", data.created_at));
    out.push_str(&format!("- **Updated:** {}\n", data.updated_at));
    out.push_str(&format!(
        "- **Tokens:** {} in / {} out\n\n",
        data.total_input_tokens, data.total_output_tokens
    ));
    out.push_str("---\n\n");

    for msg in &data.messages {
        let role = match msg.role {
            Role::System => "System",
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
        };
        let mut parts: Vec<String> = Vec::new();
        for c in &msg.content {
            match c.content_type {
                ContentType::Text => {
                    if let Some(ref t) = c.text {
                        if !t.trim().is_empty() {
                            parts.push(t.clone());
                        }
                    }
                }
                ContentType::Thinking => {
                    if let Some(ref t) = c.thinking {
                        if !t.trim().is_empty() {
                            parts.push(format!("<details><summary>thinking</summary>\n\n{}\n\n</details>", t));
                        }
                    }
                }
                ContentType::ToolUse => {
                    let name = c.name.as_deref().unwrap_or("tool");
                    let input = c
                        .input
                        .as_ref()
                        .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
                        .unwrap_or_default();
                    parts.push(format!("**Tool call `{name}`**\n\n```json\n{input}\n```"));
                }
                ContentType::ToolResult => {
                    let name = c.name.as_deref().unwrap_or("tool");
                    let body = c.text.as_deref().unwrap_or("");
                    let err = if c.is_error { " (error)" } else { "" };
                    parts.push(format!("**Tool result `{name}`{err}**\n\n```\n{body}\n```"));
                }
                _ => {}
            }
        }
        if parts.is_empty() {
            continue;
        }
        out.push_str(&format!("## {}\n\n", role));
        for part in parts {
            out.push_str(&part);
            out.push_str("\n\n");
        }
    }

    out
}

/// Full structured JSON export (includes tool results, call tree, todos).
pub fn export_json(data: &SessionData) -> Result<String, String> {
    serde_json::to_string_pretty(data).map_err(|e| e.to_string())
}

/// Self-contained HTML export for sharing.
pub fn export_html(data: &SessionData) -> String {
    let title = html_escape(data.title.as_deref().unwrap_or(&data.id));
    let mut body = String::new();
    for msg in &data.messages {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let mut inner = String::new();
        for c in &msg.content {
            match c.content_type {
                ContentType::Text => {
                    if let Some(ref t) = c.text {
                        if !t.trim().is_empty() {
                            inner.push_str(&format!("<pre class=\"text\">{}</pre>", html_escape(t)));
                        }
                    }
                }
                ContentType::Thinking => {
                    if let Some(ref t) = c.thinking {
                        if !t.trim().is_empty() {
                            inner.push_str(&format!(
                                "<details><summary>thinking</summary><pre>{}</pre></details>",
                                html_escape(t)
                            ));
                        }
                    }
                }
                ContentType::ToolUse => {
                    let name = html_escape(c.name.as_deref().unwrap_or("tool"));
                    let input = c
                        .input
                        .as_ref()
                        .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
                        .unwrap_or_default();
                    inner.push_str(&format!(
                        "<div class=\"tool-use\"><strong>tool {}</strong><pre>{}</pre></div>",
                        name,
                        html_escape(&input)
                    ));
                }
                ContentType::ToolResult => {
                    let name = html_escape(c.name.as_deref().unwrap_or("tool"));
                    let body_t = html_escape(c.text.as_deref().unwrap_or(""));
                    let cls = if c.is_error { "tool-result error" } else { "tool-result" };
                    inner.push_str(&format!(
                        "<div class=\"{cls}\"><strong>result {name}</strong><pre>{body_t}</pre></div>"
                    ));
                }
                _ => {}
            }
        }
        if inner.is_empty() {
            continue;
        }
        body.push_str(&format!("<section class=\"msg {role}\"><h2>{role}</h2>{inner}</section>\n"));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<title>{title}</title>
<style>
body {{ font-family: ui-sans-serif, system-ui, sans-serif; max-width: 900px; margin: 2rem auto; padding: 0 1rem; background: #0f1115; color: #e6e6e6; }}
h1 {{ font-size: 1.4rem; }}
.meta {{ color: #9aa; font-size: 0.9rem; }}
section.msg {{ border: 1px solid #2a2f3a; border-radius: 8px; padding: 0.75rem 1rem; margin: 1rem 0; }}
section.user {{ border-color: #3a5a8a; }}
section.assistant {{ border-color: #3a7a5a; }}
section.tool {{ border-color: #6a5a3a; }}
h2 {{ margin: 0 0 0.5rem; font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.04em; color: #9aa; }}
pre {{ white-space: pre-wrap; word-break: break-word; background: #161a22; padding: 0.6rem; border-radius: 6px; overflow-x: auto; }}
.tool-result.error {{ border-left: 3px solid #c44; padding-left: 0.5rem; }}
</style>
</head>
<body>
<h1>{title}</h1>
<p class="meta">Session {id} · {provider}/{model} · {created} · {tin} in / {tout} out</p>
{body}
</body>
</html>
"#,
        title = title,
        id = html_escape(&data.id),
        provider = html_escape(&data.provider),
        model = html_escape(&data.model),
        created = html_escape(&data.created_at),
        tin = data.total_input_tokens,
        tout = data.total_output_tokens,
        body = body,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub struct SessionStore {
    dir: String,
}

impl SessionStore {
    fn home_dir() -> String {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string())
    }

    pub fn new() -> Self {
        let dir = Path::new(&Self::home_dir())
            .join(".rs-agent")
            .join("sessions");
        let _ = fs::create_dir_all(&dir);
        Self {
            dir: dir.to_string_lossy().to_string(),
        }
    }

    /// Test-only constructor pointing at an arbitrary directory, so tests
    /// don't need to mutate the process-wide `HOME` env var.
    #[cfg(test)]
    fn for_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = fs::create_dir_all(&dir);
        Self {
            dir: dir.to_string_lossy().to_string(),
        }
    }

    pub fn generate_id() -> String {
        Local::now().format("session_%Y%m%d_%H%M%S").to_string()
    }

    /// Human-facing id fragment (`20260808_113045` from `session_20260808_113045`).
    pub fn short_id(id: &str) -> &str {
        id.strip_prefix("session_").unwrap_or(id)
    }

    pub fn session_path(&self, id: &str) -> String {
        Path::new(&self.dir).join(format!("{}.json", id))
            .to_string_lossy()
            .to_string()
    }

    pub fn exists(&self, id: &str) -> bool {
        Path::new(&self.session_path(id)).exists()
    }

    /// Resolve a resume query: exact id, `latest`/`last`/`-`, date suffix, or unique prefix.
    pub fn resolve(&self, query: &str) -> Result<String, String> {
        let q = query.trim();
        if q.is_empty() || matches!(q, "latest" | "last" | "-") {
            return self
                .list()?
                .into_iter()
                .next()
                .ok_or_else(|| "No saved sessions. Start a turn, then exit to create one.".into());
        }
        if self.exists(q) {
            return Ok(q.to_string());
        }
        let prefixed = if q.starts_with("session_") {
            q.to_string()
        } else {
            format!("session_{q}")
        };
        if self.exists(&prefixed) {
            return Ok(prefixed);
        }
        let ids = self.list()?;
        let matches: Vec<String> = ids
            .into_iter()
            .filter(|id| {
                id == q
                    || id.starts_with(q)
                    || Self::short_id(id).starts_with(q)
                    || id.contains(q)
            })
            .collect();
        match matches.as_slice() {
            [one] => Ok(one.clone()),
            [] => Err(format!(
                "Session `{q}` not found. Try `rs-agent --list-sessions` or `-r latest`."
            )),
            many => Err(format!(
                "Ambiguous session `{q}` ({} matches). Use a fuller id, e.g. `-r {}`.",
                many.len(),
                Self::short_id(&many[0])
            )),
        }
    }

    /// Most recently saved session id (list is newest-first).
    pub fn latest_id(&self) -> Result<Option<String>, String> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn save(&self, data: &SessionData) -> Result<(), String> {
        let path = self.session_path(&data.id);
        let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        fs::write(&path, &json).map_err(|e| format!("Failed to save session: {}", e))
    }

    pub fn load(&self, id: &str) -> Result<SessionData, String> {
        let path = self.session_path(id);
        let json = fs::read_to_string(&path).map_err(|e| format!("Session '{}' not found: {}", id, e))?;
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse session '{}': {}", id, e))
    }

    pub fn list(&self) -> Result<Vec<String>, String> {
        let dir = Path::new(&self.dir);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions: Vec<String> = fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect();
        sessions.sort_by(|a, b| b.cmp(a));
        Ok(sessions)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let path = self.session_path(id);
        fs::remove_file(&path).map_err(|e| format!("Failed to delete session '{}': {}", id, e))
    }

    /// Fork a session: copy transcript into a new id with `parent_id` set.
    pub fn fork(&self, source_id: &str, branch_label: Option<String>) -> Result<SessionData, String> {
        self.fork_at(source_id, None, branch_label)
    }

    /// Fork a session, optionally truncating to the first `at` messages
    /// (timeline "fork from here"). `at = None` copies the full transcript.
    pub fn fork_at(
        &self,
        source_id: &str,
        at: Option<usize>,
        branch_label: Option<String>,
    ) -> Result<SessionData, String> {
        let mut data = self.load(source_id)?;
        if let Some(n) = at {
            if n > data.messages.len() {
                return Err(format!(
                    "fork_at index {n} out of range (session has {} messages)",
                    data.messages.len()
                ));
            }
            data.messages.truncate(n);
        }
        let parent = data.id.clone();
        let new_id = Self::generate_id();
        // Avoid colliding with an existing id in the same second
        let new_id = if self.exists(&new_id) {
            format!("{}_{}", new_id, &uuid::Uuid::new_v4().to_string()[..8])
        } else {
            new_id
        };
        data.parent_id = Some(parent.clone());
        data.branch_label = branch_label.clone();
        data.id = new_id;
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        data.created_at = now.clone();
        data.updated_at = now;
        let base_title = data
            .title
            .clone()
            .unwrap_or_else(|| parent.clone());
        let fork_label = match (&branch_label, at) {
            (Some(label), Some(n)) if !label.is_empty() => format!("{} [@{}]", label, n),
            (Some(label), _) if !label.is_empty() => label.clone(),
            (_, Some(n)) => format!("@{}", n),
            _ => "fork".into(),
        };
        data.title = Some(format!("{} [{}]", base_title, fork_label));
        self.save(&data)?;
        Ok(data)
    }

    /// Like [`Self::list`], but loads each session and returns lightweight
    /// summaries (title, model, last-updated, message count) instead of
    /// bare IDs. Sessions that fail to parse are skipped.
    pub fn list_summaries(&self) -> Result<Vec<SessionSummary>, String> {
        let ids = self.list()?;
        let summaries = ids
            .into_iter()
            .filter_map(|id| {
                let data = self.load(&id).ok()?;
                Some(SessionSummary {
                    id: data.id,
                    title: data.title,
                    model: data.model,
                    updated_at: data.updated_at,
                    message_count: data.messages.len(),
                    parent_id: data.parent_id,
                    branch_label: data.branch_label,
                })
            })
            .collect();
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::{Content, ContentType};

    fn user_message(text: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some(text.to_string()),
                ..Default::default()
            }],
        }
    }

    fn assistant_message(text: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some(text.to_string()),
                ..Default::default()
            }],
        }
    }

    fn sample_session(id: &str) -> SessionData {
        SessionData {
            id: id.to_string(),
            title: None,
            parent_id: None,
            branch_label: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:05:00Z".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            provider: "anthropic".to_string(),
            system_prompt: "You are helpful.".to_string(),
            messages: vec![
                user_message("Fix the failing test in bash.rs please"),
                assistant_message("Sure, looking into it now."),
            ],
            total_input_tokens: 100,
            total_output_tokens: 50,
            call_tree: None,
            todos: None,
            goal: None,
            seat: None,
            handoff: None,
        }
    }

    #[test]
    fn deserializes_legacy_session_without_title() {
        let json = r#"{
            "id": "session_20260101_000000",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "model": "gpt-4o",
            "provider": "openai",
            "system_prompt": "",
            "messages": [],
            "total_input_tokens": 0,
            "total_output_tokens": 0
        }"#;
        let data: SessionData = serde_json::from_str(json).expect("should parse without title");
        assert_eq!(data.title, None);
    }

    #[test]
    fn auto_title_from_messages_uses_first_user_text() {
        let data = sample_session("session_1");
        assert_eq!(
            data.auto_title_from_messages().as_deref(),
            Some("Fix the failing test in bash.rs please")
        );
    }

    #[test]
    fn auto_title_from_messages_none_without_user_message() {
        let mut data = sample_session("session_1");
        data.messages = vec![assistant_message("no user message yet")];
        assert_eq!(data.auto_title_from_messages(), None);
    }

    #[test]
    fn auto_title_from_messages_truncates_long_text() {
        let mut data = sample_session("session_1");
        let long_text = "word ".repeat(50);
        data.messages = vec![user_message(&long_text)];
        let title = data.auto_title_from_messages().expect("some title");
        assert!(title.chars().count() <= 61); // 60 chars + ellipsis
        assert!(title.ends_with('…'));
    }

    #[test]
    fn ensure_title_sets_once_and_is_idempotent() {
        let mut data = sample_session("session_1");
        data.ensure_title();
        assert_eq!(
            data.title.as_deref(),
            Some("Fix the failing test in bash.rs please")
        );

        // Doesn't overwrite an existing title, even if messages change.
        data.messages = vec![user_message("something else entirely")];
        data.ensure_title();
        assert_eq!(
            data.title.as_deref(),
            Some("Fix the failing test in bash.rs please")
        );
    }

    #[test]
    fn export_markdown_includes_metadata_and_messages() {
        let mut data = sample_session("session_1");
        data.title = Some("Fix bash test".to_string());
        let md = export_markdown(&data);
        assert!(md.starts_with("# Fix bash test\n"));
        assert!(md.contains("**Session ID:** session_1"));
        assert!(md.contains("**Model:** claude-sonnet-4-20250514 (anthropic)"));
        assert!(md.contains("## User"));
        assert!(md.contains("Fix the failing test in bash.rs please"));
        assert!(md.contains("## Assistant"));
        assert!(md.contains("Sure, looking into it now."));
    }

    #[test]
    fn export_markdown_falls_back_to_id_without_title() {
        let data = sample_session("session_1");
        let md = export_markdown(&data);
        assert!(md.starts_with("# session_1\n"));
    }

    #[test]
    fn store_save_load_roundtrip_preserves_title() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::for_dir(tmp.path());
        let mut data = sample_session("session_roundtrip");
        data.ensure_title();
        store.save(&data).expect("save should succeed");

        let loaded = store.load("session_roundtrip").expect("load should succeed");
        assert_eq!(loaded.title, data.title);
        assert_eq!(loaded.messages.len(), data.messages.len());
    }

    #[test]
    fn resolve_latest_and_short_id() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::for_dir(tmp.path());
        let mut older = sample_session("session_20260101_120000");
        older.ensure_title();
        store.save(&older).unwrap();
        let mut newer = sample_session("session_20260808_150000");
        newer.ensure_title();
        store.save(&newer).unwrap();

        assert_eq!(SessionStore::short_id("session_20260808_150000"), "20260808_150000");
        assert_eq!(store.resolve("latest").unwrap(), "session_20260808_150000");
        assert_eq!(store.resolve("20260808_150000").unwrap(), "session_20260808_150000");
        assert_eq!(store.resolve("20260808").unwrap(), "session_20260808_150000");
    }

    #[test]
    fn deserializes_legacy_session_without_call_tree() {
        let json = r#"{
            "id": "session_20260101_000000",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "model": "gpt-4o",
            "provider": "openai",
            "system_prompt": "",
            "messages": [],
            "total_input_tokens": 0,
            "total_output_tokens": 0
        }"#;
        let data: SessionData =
            serde_json::from_str(json).expect("should parse without call_tree");
        assert!(data.call_tree.is_none());
    }

    #[test]
    fn store_save_load_roundtrip_preserves_call_tree_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::for_dir(tmp.path());
        let mut data = sample_session("session_with_tree");
        data.call_tree = Some(serde_json::json!({
            "nodes": [{
                "id": "root_0",
                "parent_id": null,
                "kind": "root",
                "task": "do the thing",
                "status": "done",
                "summary": null
            }],
            "root_id": "root_0"
        }));
        store.save(&data).expect("save should succeed");

        let loaded = store.load("session_with_tree").expect("load should succeed");
        assert_eq!(loaded.call_tree, data.call_tree);
    }

    #[test]
    fn list_summaries_returns_lightweight_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::for_dir(tmp.path());

        let mut a = sample_session("session_a");
        a.ensure_title();
        store.save(&a).unwrap();

        let mut b = sample_session("session_b");
        b.title = Some("Custom title".to_string());
        b.messages.push(user_message("another message"));
        store.save(&b).unwrap();

        let summaries = store.list_summaries().expect("list_summaries should succeed");
        assert_eq!(summaries.len(), 2);

        let summary_b = summaries.iter().find(|s| s.id == "session_b").unwrap();
        assert_eq!(summary_b.title.as_deref(), Some("Custom title"));
        assert_eq!(summary_b.message_count, 3);
        assert_eq!(summary_b.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn list_summaries_empty_dir_returns_empty_vec() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::for_dir(tmp.path().join("nonexistent"));
        let summaries = store.list_summaries().expect("should succeed on empty dir");
        assert!(summaries.is_empty());
    }

    #[test]
    fn fork_copies_messages_and_sets_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::for_dir(tmp.path());
        let mut src = sample_session("session_src");
        src.title = Some("Original".into());
        store.save(&src).unwrap();

        let forked = store.fork("session_src", Some("experiment".into())).unwrap();
        assert_ne!(forked.id, "session_src");
        assert_eq!(forked.parent_id.as_deref(), Some("session_src"));
        assert_eq!(forked.branch_label.as_deref(), Some("experiment"));
        assert_eq!(forked.messages.len(), src.messages.len());
        assert!(forked.title.as_deref().unwrap().contains("experiment"));
        assert!(store.exists(&forked.id));
    }

    #[test]
    fn fork_at_truncates_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::for_dir(tmp.path());
        let src = sample_session("session_long");
        let n = src.messages.len();
        assert!(n >= 1);
        store.save(&src).unwrap();

        let forked = store.fork_at("session_long", Some(1), Some("at1".into())).unwrap();
        assert_eq!(forked.messages.len(), 1.min(n));
        assert_eq!(forked.parent_id.as_deref(), Some("session_long"));
        assert!(forked.title.as_deref().unwrap().contains("@1"));
    }
}
