use crate::ai::types::{Message, Role};
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
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub provider: String,
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
    /// Snapshot of the RLM call tree (`CallTreeInner`, serialized) at the
    /// end of the last turn, if any. Lets `/tree` show a last-known summary
    /// after resuming a session, even before a new turn has run.
    #[serde(default)]
    pub call_tree: Option<serde_json::Value>,
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
        let texts: Vec<String> = msg
            .content
            .iter()
            .filter_map(|c| c.text.clone())
            .filter(|t| !t.trim().is_empty())
            .collect();
        if texts.is_empty() {
            continue;
        }
        out.push_str(&format!("## {}\n\n", role));
        for text in texts {
            out.push_str(&text);
            out.push_str("\n\n");
        }
    }

    out
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

    pub fn session_path(&self, id: &str) -> String {
        Path::new(&self.dir).join(format!("{}.json", id))
            .to_string_lossy()
            .to_string()
    }

    pub fn exists(&self, id: &str) -> bool {
        Path::new(&self.session_path(id)).exists()
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
}
