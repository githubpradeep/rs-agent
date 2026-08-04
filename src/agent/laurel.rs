//! Recognition (laurels) — praise with no work attached.
//!
//! Append-only project log + optional seat copy. Injected on wake as
//! recognition only so agents are not incentivized to farm them.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Laurel {
    pub written_at: String,
    pub text: String,
    #[serde(default)]
    pub seat: Option<String>,
}

impl Laurel {
    pub fn new(text: String, seat: Option<String>) -> Self {
        Self {
            written_at: chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
            text,
            seat,
        }
    }
}

fn project_laurels_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("laurels.jsonl")
}

/// Append a laurel to the project log.
pub fn append(laurel: &Laurel) -> Result<(), String> {
    let path = project_laurels_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open laurels: {e}"))?;
    let line = serde_json::to_string(laurel).map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| format!("write laurels: {e}"))?;
    Ok(())
}

/// Read recent laurels from the project log (newest last).
pub fn recent(limit: usize) -> Vec<Laurel> {
    load_from_path(&project_laurels_path(), limit)
}

pub fn load_from_path(path: &Path, limit: usize) -> Vec<Laurel> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut items: Vec<Laurel> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if items.len() > limit {
        items = items.split_off(items.len() - limit);
    }
    items
}

/// Wake-packet block: recognition only, no action.
pub fn format_wake_block(items: &[Laurel]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut out = String::from(
        "## Laurels (recognition only — no action required)\n\
         These emerged spontaneously. Do not chase more laurels.\n",
    );
    for l in items {
        out.push_str(&format!("- [{}] {}\n", l.written_at, l.text.trim()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_wake_empty() {
        assert!(format_wake_block(&[]).is_none());
    }

    #[test]
    fn format_wake_has_disclaimer() {
        let items = vec![Laurel::new("nice fix".into(), None)];
        let s = format_wake_block(&items).unwrap();
        assert!(s.contains("no action required"));
        assert!(s.contains("nice fix"));
    }

    #[test]
    fn roundtrip_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("laurels.jsonl");
        let l = Laurel::new("shipped".into(), Some("fox".into()));
        let line = serde_json::to_string(&l).unwrap();
        fs::write(&path, format!("{line}\n")).unwrap();
        let loaded = load_from_path(&path, 5);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "shipped");
    }
}
