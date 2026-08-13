//! Moot — lightweight agent meeting threads under `.rs-agent/moot/`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MootEntry {
    pub at: String,
    pub from: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Moot {
    pub id: String,
    pub topic: String,
    pub status: String,
    pub created_at: String,
    #[serde(default)]
    pub entries: Vec<MootEntry>,
    #[serde(default)]
    pub closed_at: Option<String>,
}

fn moot_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("moot")
}

fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn ensure_dir() -> Result<(), String> {
    fs::create_dir_all(moot_dir()).map_err(|e| format!("mkdir moot: {e}"))
}

fn path_for(id: &str) -> PathBuf {
    moot_dir().join(format!("{id}.json"))
}

fn save(m: &Moot) -> Result<(), String> {
    ensure_dir()?;
    let text = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
    fs::write(path_for(&m.id), text).map_err(|e| format!("write moot: {e}"))
}

fn load(id: &str) -> Result<Moot, String> {
    let text = fs::read_to_string(path_for(id)).map_err(|e| format!("moot `{id}`: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("moot `{id}` corrupt: {e}"))
}

fn next_id() -> String {
    let n = fs::read_dir(moot_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
                .count()
        })
        .unwrap_or(0);
    format!("moot{}", n + 1)
}

pub fn open(topic: &str) -> Result<Moot, String> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err("moot topic must not be empty".into());
    }
    ensure_dir()?;
    let m = Moot {
        id: next_id(),
        topic: topic.to_string(),
        status: "open".into(),
        created_at: now_str(),
        entries: Vec::new(),
        closed_at: None,
    };
    save(&m)?;
    Ok(m)
}

pub fn append(id: &str, from: &str, text: &str) -> Result<Moot, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("moot entry must not be empty".into());
    }
    let mut m = load(id)?;
    if m.status != "open" {
        return Err(format!("moot `{id}` is {}", m.status));
    }
    m.entries.push(MootEntry {
        at: now_str(),
        from: from.trim().to_string(),
        text: text.to_string(),
    });
    save(&m)?;
    Ok(m)
}

pub fn close(id: &str, summary: Option<&str>) -> Result<Moot, String> {
    let mut m = load(id)?;
    if let Some(s) = summary {
        if !s.trim().is_empty() {
            m.entries.push(MootEntry {
                at: now_str(),
                from: "system".into(),
                text: format!("CLOSE: {}", s.trim()),
            });
        }
    }
    m.status = "closed".into();
    m.closed_at = Some(now_str());
    save(&m)?;
    Ok(m)
}

pub fn show(id: &str) -> Result<String, String> {
    let m = load(id)?;
    Ok(format_moot(&m))
}

pub fn list() -> String {
    let Ok(entries) = fs::read_dir(moot_dir()) else {
        return "No moots.".into();
    };
    let mut ids: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .collect();
    ids.sort();
    if ids.is_empty() {
        return "No moots.".into();
    }
    let mut out = String::from("Moots:\n");
    for id in ids {
        if let Ok(m) = load(&id) {
            out.push_str(&format!(
                "  {} [{}] {} ({} entries)\n",
                m.id,
                m.status,
                m.topic,
                m.entries.len()
            ));
        }
    }
    out
}

pub fn format_moot(m: &Moot) -> String {
    let mut out = format!(
        "Moot {} [{}]\nTopic: {}\nCreated: {}\n",
        m.id, m.status, m.topic, m.created_at
    );
    if let Some(ref c) = m.closed_at {
        out.push_str(&format!("Closed: {c}\n"));
    }
    for e in &m.entries {
        out.push_str(&format!("\n[{}] {}:\n{}\n", e.at, e.from, e.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_append_close() {
        crate::with_temp_cwd(|_| {
            let m = open("design auth").unwrap();
            append(&m.id, "Crew-1", "prefer JWT").unwrap();
            let closed = close(&m.id, Some("JWT it is")).unwrap();
            assert_eq!(closed.status, "closed");
            assert!(closed.entries.len() >= 2);
        });
    }
}
