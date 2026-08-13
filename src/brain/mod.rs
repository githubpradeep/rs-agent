//! Project brain — doctrine markdown + short operational facts primed on wake.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fact {
    pub written_at: String,
    pub text: String,
    #[serde(default)]
    pub falsified: bool,
    #[serde(default)]
    pub id: Option<String>,
}

impl Fact {
    pub fn new(text: String) -> Self {
        Self {
            written_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            text,
            falsified: false,
            id: Some(format!("f{}", chrono::Local::now().format("%H%M%S"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerEntry {
    pub at: String,
    pub bead: String,
    pub summary: String,
    #[serde(default)]
    pub git_sha: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

pub fn brain_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("brain")
}

pub fn facts_path() -> PathBuf {
    brain_dir().join("facts.jsonl")
}

pub fn ledger_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("ledger.jsonl")
}

fn project_rs_agent() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
}

pub fn remember(text: &str) -> Result<Fact, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("fact must not be empty".into());
    }
    let dir = brain_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir brain: {e}"))?;
    let path = facts_path();
    let fact = Fact::new(text.to_string());
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open facts: {e}"))?;
    let line = serde_json::to_string(&fact).map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| format!("write fact: {e}"))?;
    Ok(fact)
}

/// Mark facts matching id substring or text substring as falsified (rewrite file).
pub fn falsify(query: &str) -> Result<usize, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("falsify query must not be empty".into());
    }
    let path = facts_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(0);
    };
    let q = query.to_lowercase();
    let mut n = 0usize;
    let mut out_lines = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut fact: Fact = match serde_json::from_str(line) {
            Ok(f) => f,
            Err(_) => {
                out_lines.push(line.to_string());
                continue;
            }
        };
        let id_hit = fact
            .id
            .as_deref()
            .map(|id| id.eq_ignore_ascii_case(query) || id.to_lowercase().contains(&q))
            .unwrap_or(false);
        let text_hit = fact.text.to_lowercase().contains(&q);
        if !fact.falsified && (id_hit || text_hit) {
            fact.falsified = true;
            n += 1;
        }
        out_lines.push(serde_json::to_string(&fact).unwrap_or_else(|_| line.to_string()));
    }
    if n > 0 {
        fs::write(&path, out_lines.join("\n") + "\n").map_err(|e| format!("write facts: {e}"))?;
    }
    Ok(n)
}

pub fn recent_facts(limit: usize) -> Vec<Fact> {
    load_facts(&facts_path(), limit)
}

pub fn load_facts(path: &Path, limit: usize) -> Vec<Fact> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut items: Vec<Fact> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|f: &Fact| !f.falsified)
        .collect();
    if items.len() > limit {
        items = items.split_off(items.len() - limit);
    }
    items
}

fn git_head_sha() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Append closed-bead provenance to `.rs-agent/ledger.jsonl`.
pub fn record_close(bead_id: &str, kind: &str, summary: &str) -> Result<(), String> {
    let dir = project_rs_agent();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir .rs-agent: {e}"))?;
    let entry = LedgerEntry {
        at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        bead: bead_id.to_string(),
        summary: summary.chars().take(400).collect(),
        git_sha: git_head_sha(),
        kind: Some(kind.to_string()),
    };
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path())
        .map_err(|e| format!("open ledger: {e}"))?;
    let line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| format!("write ledger: {e}"))?;

    let s = summary.trim();
    if s.len() > 12 && !s.eq_ignore_ascii_case("done") {
        let _ = remember(&format!("Closed {bead_id} ({kind}): {s}"));
    }
    Ok(())
}

pub fn recent_ledger(limit: usize) -> Vec<LedgerEntry> {
    let Ok(text) = fs::read_to_string(ledger_path()) else {
        return Vec::new();
    };
    let mut items: Vec<LedgerEntry> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if items.len() > limit {
        items = items.split_off(items.len() - limit);
    }
    items
}

pub fn format_ledger_wake(limit: usize) -> Option<String> {
    let items = recent_ledger(limit);
    if items.is_empty() {
        return None;
    }
    let mut out = String::from("## Recent ledger (closed work)\n");
    for e in &items {
        let sha = e.git_sha.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "- [{}] {} {} @{} — {}\n",
            e.at,
            e.bead,
            e.kind.as_deref().unwrap_or("?"),
            sha,
            e.summary.trim()
        ));
    }
    Some(out)
}

/// Load up to `limit` chars of doctrine from `brain/*.md` (sorted by name).
pub fn doctrine_excerpt(limit_chars: usize) -> Option<String> {
    let dir = brain_dir();
    if !dir.is_dir() {
        return None;
    }
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    files.sort();
    if files.is_empty() {
        return None;
    }
    let mut out = String::from("### Doctrine (brain/)\n");
    let mut used = out.len();
    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("doc.md");
        let Ok(body) = fs::read_to_string(&path) else {
            continue;
        };
        let snippet: String = body.chars().take(1_500).collect();
        let block = format!("#### {name}\n{snippet}\n");
        if used + block.len() > limit_chars {
            break;
        }
        out.push_str(&block);
        used += block.len();
    }
    if out.len() <= "### Doctrine (brain/)\n".len() {
        None
    } else {
        Some(out)
    }
}

/// Wake-packet brain section (facts + doctrine + ledger), capped.
pub fn format_wake_block(fact_limit: usize, doctrine_chars: usize) -> Option<String> {
    let facts = recent_facts(fact_limit);
    let doctrine = doctrine_excerpt(doctrine_chars);
    let ledger = format_ledger_wake(6);
    if facts.is_empty() && doctrine.is_none() && ledger.is_none() {
        return None;
    }
    let mut out = String::from("## Project brain\n");
    if let Some(d) = doctrine {
        out.push_str(&d);
        out.push('\n');
    }
    if !facts.is_empty() {
        out.push_str("### Operational facts (until falsified)\n");
        for f in &facts {
            let id = f.id.as_deref().unwrap_or("-");
            out.push_str(&format!("- [{id}] [{}] {}\n", f.written_at, f.text.trim()));
        }
    }
    if let Some(l) = ledger {
        out.push('\n');
        out.push_str(&l);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("facts.jsonl");
        let f = Fact::new("Metal shaders need TG=256".into());
        fs::write(&path, format!("{}\n", serde_json::to_string(&f).unwrap())).unwrap();
        let loaded = load_facts(&path, 5);
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].text.contains("TG=256"));
    }
}
