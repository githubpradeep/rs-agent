//! Append-only trajectory. Compaction projects a summary plus an index;
//! `recall` reads a raw step back.

use crate::ai::types::{Message, Role};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const TEXT_CAP: usize = 80_000;
const PREVIEW_CHARS: usize = 100;
const INDEX_TAIL: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchivedStep {
    pub id: String,
    pub role: String,
    pub preview: String,
    pub text: String,
}

static SESSION: Mutex<String> = Mutex::new(String::new());
static ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_session(id: &str) {
    if let Ok(mut g) = SESSION.lock() {
        *g = id.to_string();
    }
}

fn session_key() -> String {
    SESSION
        .lock()
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.clone())
        .unwrap_or_else(|| format!("pid-{}", std::process::id()))
}

fn root_dir() -> PathBuf {
    if let Ok(g) = ROOT.lock() {
        if let Some(p) = g.clone() {
            return p;
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".rs-agent").join("trajectories")
}

fn traj_path() -> PathBuf {
    let safe: String = session_key()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    root_dir().join(format!("{safe}.jsonl"))
}

/// Test hook: archive and recall under `dir` instead of `~/.rs-agent`.
#[cfg(test)]
pub fn set_root_for_test(dir: PathBuf) {
    if let Ok(mut g) = ROOT.lock() {
        *g = Some(dir);
    }
}

fn one_line(text: &str, max: usize) -> String {
    let flat = text.replace(['\n', '\r'], " ");
    let t = flat.trim();
    let clipped: String = t.chars().take(max).collect();
    if t.chars().count() > max {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn message_text(msg: &Message) -> (String, String) {
    let mut role = match msg.role {
        Role::User => "user".to_string(),
        Role::Assistant => "assistant".to_string(),
        Role::Tool => "tool".to_string(),
        Role::System => "system".to_string(),
    };
    let mut parts = Vec::new();
    for c in &msg.content {
        if let Some(name) = c.name.as_deref().filter(|n| !n.is_empty()) {
            if role == "tool" {
                role = format!("tool:{name}");
            }
            parts.push(format!("[{name}]"));
        }
        if let Some(t) = c.text.as_deref().filter(|t| !t.is_empty()) {
            parts.push(t.to_string());
        }
    }
    (role, parts.join("\n"))
}

fn next_index(path: &std::path::Path) -> usize {
    let Ok(raw) = fs::read_to_string(path) else {
        return 1;
    };
    let mut max_n = 0usize;
    for line in raw.lines() {
        if let Ok(step) = serde_json::from_str::<ArchivedStep>(line) {
            if let Some(n) = step.id.strip_prefix('s').and_then(|n| n.parse().ok()) {
                max_n = max_n.max(n);
            }
        }
    }
    max_n + 1
}

/// Append messages that are about to leave the live window. Returns the new steps.
pub fn archive(messages: &[Message]) -> Result<Vec<ArchivedStep>, String> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let path = traj_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir trajectory: {e}"))?;
    }
    let mut n = next_index(&path);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open trajectory: {e}"))?;
    let mut out = Vec::new();
    for msg in messages {
        let (role, text) = message_text(msg);
        let chars = text.chars().count();
        let stored = if chars > TEXT_CAP {
            let head: String = text.chars().take(TEXT_CAP).collect();
            format!("{head}\n…[trajectory step truncated, {chars} chars]")
        } else {
            text
        };
        let step = ArchivedStep {
            id: format!("s{n}"),
            role,
            preview: one_line(&stored, PREVIEW_CHARS),
            text: stored,
        };
        n += 1;
        let line = serde_json::to_string(&step).map_err(|e| e.to_string())?;
        writeln!(file, "{line}").map_err(|e| format!("write trajectory: {e}"))?;
        out.push(step);
    }
    Ok(out)
}

pub fn recall(id: &str) -> Result<String, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("recall requires a step id such as s12".into());
    }
    let path = traj_path();
    let raw = fs::read_to_string(&path)
        .map_err(|_| format!("no trajectory for this session ({})", path.display()))?;
    for line in raw.lines() {
        if let Ok(step) = serde_json::from_str::<ArchivedStep>(line) {
            if step.id == id {
                return Ok(format!(
                    "id: {}\nrole: {}\n\n{}",
                    step.id, step.role, step.text
                ));
            }
        }
    }
    Err(format!("trajectory step `{id}` not found"))
}

/// Search step previews. Returns `(id, preview)` pairs, newest last.
pub fn search(query: &str, limit: usize) -> Vec<(String, String)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() || limit == 0 {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(traj_path()) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for line in raw.lines() {
        if let Ok(step) = serde_json::from_str::<ArchivedStep>(line) {
            if step.preview.to_lowercase().contains(&q) || step.text.to_lowercase().contains(&q) {
                hits.push((step.id, format!("{}: {}", step.role, step.preview)));
            }
        }
    }
    let start = hits.len().saturating_sub(limit);
    hits.into_iter().skip(start).collect()
}

pub fn format_index(steps: &[ArchivedStep]) -> String {
    if steps.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "[Trajectory index]\nRaw steps stay on disk. Use the recall tool with an id.\n",
    );
    let skip = steps.len().saturating_sub(INDEX_TAIL);
    if skip > 0 {
        out.push_str(&format!("{skip} earlier steps archived (recall by id).\n"));
    }
    for s in steps.iter().skip(skip) {
        out.push_str(&format!("{} {}: {}\n", s.id, s.role, s.preview));
    }
    out
}

/// Summary portion of a compaction projection, without the index or older tier.
pub fn summary_body(text: &str) -> &str {
    let rest = text
        .strip_prefix("[Compacted summary of earlier conversation]\n")
        .unwrap_or(text);
    rest.split("\n[Trajectory index]")
        .next()
        .unwrap_or(rest)
        .trim()
}

pub fn older_tier(previous_summary: &str) -> String {
    let body = summary_body(previous_summary);
    let clipped: String = body.chars().take(1200).collect();
    if clipped.trim().is_empty() {
        return String::new();
    }
    format!("\n[Older trajectory]\n{clipped}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::{Content, ContentType, Message, Role};

    fn msg(role: Role, text: &str) -> Message {
        Message {
            role,
            content: vec![Content {
                content_type: ContentType::Text,
                text: Some(text.to_string()),
                ..Default::default()
            }],
        }
    }

    struct RootGuard;
    impl Drop for RootGuard {
        fn drop(&mut self) {
            if let Ok(mut g) = ROOT.lock() {
                *g = None;
            }
            if let Ok(mut g) = SESSION.lock() {
                g.clear();
            }
        }
    }

    #[test]
    fn archive_then_recall_roundtrip() {
        crate::with_temp_cwd(|dir| {
            let _guard = RootGuard;
            set_root_for_test(dir.join("traj"));
            set_session("test-session");
            let steps = archive(&[
                msg(Role::User, "find the needle"),
                msg(Role::Assistant, "looking"),
            ])
            .unwrap();
            assert_eq!(steps.len(), 2);
            assert_eq!(steps[0].id, "s1");
            let body = recall("s1").unwrap();
            assert!(body.contains("find the needle"));
            let again = archive(&[msg(Role::Tool, "grep hit")]).unwrap();
            assert_eq!(again[0].id, "s3");
            assert!(search("needle", 5).iter().any(|(id, _)| id == "s1"));
        });
    }

    #[test]
    fn summary_body_drops_index() {
        let text = "[Compacted summary of earlier conversation]\n## Goal\nship it\n\n[Trajectory index]\ns1 user: hi\n";
        assert_eq!(summary_body(text), "## Goal\nship it");
        let older = older_tier(text);
        assert!(older.contains("[Older trajectory]"));
        assert!(older.contains("ship it"));
        assert!(!older.contains("s1 user"));
    }

    #[test]
    fn index_keeps_a_tail() {
        let steps: Vec<ArchivedStep> = (1..=40)
            .map(|i| ArchivedStep {
                id: format!("s{i}"),
                role: "user".into(),
                preview: format!("p{i}"),
                text: format!("p{i}"),
            })
            .collect();
        let idx = format_index(&steps);
        assert!(idx.contains("10 earlier steps"));
        assert!(idx.contains("s40"));
        assert!(!idx.contains("s1 user"));
    }
}
