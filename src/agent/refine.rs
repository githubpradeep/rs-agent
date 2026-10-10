//! Evidence-backed harness notes. The model proposes; a person applies.
//! The base system prompt is never written.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RefineKind {
    Fact,
    SkillNote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RefineStatus {
    Pending,
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Proposal {
    pub id: String,
    pub kind: RefineKind,
    pub text: String,
    pub evidence: String,
    pub status: RefineStatus,
}

fn refine_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("refine")
}

fn proposals_path() -> PathBuf {
    refine_dir().join("proposals.json")
}

fn learned_skill_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("skills")
        .join("learned.md")
}

fn load() -> Vec<Proposal> {
    fs::read_to_string(proposals_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(items: &[Proposal]) -> Result<(), String> {
    let dir = refine_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir refine: {e}"))?;
    let body = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    fs::write(proposals_path(), body).map_err(|e| format!("write proposals: {e}"))
}

pub fn list_pending() -> Vec<Proposal> {
    load()
        .into_iter()
        .filter(|p| p.status == RefineStatus::Pending)
        .collect()
}

pub fn list_all() -> Vec<Proposal> {
    load()
}

pub fn propose(kind: RefineKind, text: &str, evidence: &str) -> Result<Proposal, String> {
    let text = text.trim();
    let evidence = evidence.trim();
    if text.chars().count() < 8 {
        return Err("refine text must be at least 8 characters".into());
    }
    if evidence.chars().count() < 8 {
        return Err("refine evidence must quote what was learned (at least 8 characters)".into());
    }
    let mut items = load();
    let n = items.len() + 1;
    let proposal = Proposal {
        id: format!("r{n}"),
        kind,
        text: text.to_string(),
        evidence: evidence.to_string(),
        status: RefineStatus::Pending,
    };
    items.push(proposal.clone());
    save(&items)?;
    Ok(proposal)
}

fn snapshot(label: &str) -> Result<PathBuf, String> {
    let dir = refine_dir().join("snapshots").join(label);
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir snapshot: {e}"))?;
    copy_or_mark(&crate::brain::facts_path(), &dir.join("facts.jsonl"))?;
    copy_or_mark(&learned_skill_path(), &dir.join("learned.md"))?;
    Ok(dir)
}

fn copy_or_mark(src: &Path, dest: &Path) -> Result<(), String> {
    if src.is_file() {
        fs::copy(src, dest).map_err(|e| format!("snapshot {}: {e}", src.display()))?;
    } else if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        fs::write(dest.with_extension("absent"), b"").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn restore_file(snap: &Path, name: &str, dest: &Path) -> Result<(), String> {
    let copied = snap.join(name);
    if copied.is_file() {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(&copied, dest).map_err(|e| format!("restore {name}: {e}"))?;
        return Ok(());
    }
    if snap.join(absent_name(name)).is_file() && dest.is_file() {
        fs::remove_file(dest).map_err(|e| format!("remove {name}: {e}"))?;
    }
    Ok(())
}

fn absent_name(name: &str) -> String {
    match name {
        "facts.jsonl" => "facts.absent".into(),
        "learned.md" => "learned.absent".into(),
        other => format!("{other}.absent"),
    }
}

pub fn apply(id: &str) -> Result<String, String> {
    let id = id.trim();
    let mut items = load();
    let proposal = items
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no refine proposal `{id}`"))?;
    if proposal.status != RefineStatus::Pending {
        return Err(format!("proposal `{id}` is already {:?}", proposal.status));
    }
    let kind = proposal.kind.clone();
    let text = proposal.text.clone();
    let evidence = proposal.evidence.clone();
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    snapshot(&format!("{stamp}-{id}"))?;
    match kind {
        RefineKind::Fact => {
            crate::brain::remember(&text)?;
        }
        RefineKind::SkillNote => {
            append_skill_note(&text, &evidence)?;
        }
    }
    proposal.status = RefineStatus::Applied;
    save(&items)?;
    Ok(format!("applied {id}"))
}

fn append_skill_note(text: &str, evidence: &str) -> Result<(), String> {
    let path = learned_skill_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir skills: {e}"))?;
    }
    if !path.is_file() {
        let header = "---\nname: learned\ndescription: Notes a person applied from /refine\ntriggers: []\n---\n\n";
        fs::write(&path, header).map_err(|e| format!("write learned skill: {e}"))?;
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let block = format!("\n## {stamp}\n{text}\n\nEvidence: {evidence}\n");
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|e| format!("open learned skill: {e}"))?;
    f.write_all(block.as_bytes())
        .map_err(|e| format!("append learned skill: {e}"))?;
    Ok(())
}

pub fn reject(id: &str) -> Result<String, String> {
    let id = id.trim();
    let mut items = load();
    let proposal = items
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no refine proposal `{id}`"))?;
    if proposal.status != RefineStatus::Pending {
        return Err(format!("proposal `{id}` is already {:?}", proposal.status));
    }
    proposal.status = RefineStatus::Rejected;
    save(&items)?;
    Ok(format!("rejected {id}"))
}

pub fn rollback() -> Result<String, String> {
    let root = refine_dir().join("snapshots");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .map_err(|_| "no refine snapshots to roll back".to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let snap = dirs
        .pop()
        .ok_or_else(|| "no refine snapshots to roll back".to_string())?;
    restore_file(&snap, "facts.jsonl", &crate::brain::facts_path())?;
    restore_file(&snap, "learned.md", &learned_skill_path())?;
    let name = snap
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("snapshot")
        .to_string();
    fs::remove_dir_all(&snap).map_err(|e| format!("drop snapshot: {e}"))?;
    Ok(format!("rolled back to {name}"))
}

pub fn format_list(items: &[Proposal]) -> String {
    if items.is_empty() {
        return "No refine proposals.".into();
    }
    let mut out = String::from("Refine proposals (apply or reject; system prompt is unchanged):\n");
    for p in items {
        let kind = match p.kind {
            RefineKind::Fact => "fact",
            RefineKind::SkillNote => "skill",
        };
        let status = match p.status {
            RefineStatus::Pending => "pending",
            RefineStatus::Applied => "applied",
            RefineStatus::Rejected => "rejected",
        };
        out.push_str(&format!(
            "{} [{} {}] {}\n  evidence: {}\n",
            p.id, status, kind, p.text, p.evidence
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_apply_rollback_leaves_system_prompt() {
        crate::with_temp_cwd(|dir| {
            let sentinel = dir.join("prompts");
            fs::create_dir_all(&sentinel).unwrap();
            let system = sentinel.join("system.md");
            fs::write(&system, "IMMUTABLE").unwrap();

            let p = propose(
                RefineKind::Fact,
                "parser rejects empty beads",
                "bead close failed on empty title",
            )
            .unwrap();
            assert_eq!(p.status, RefineStatus::Pending);
            apply(&p.id).unwrap();
            let facts = fs::read_to_string(crate::brain::facts_path()).unwrap();
            assert!(facts.contains("parser rejects empty beads"));

            let note = propose(
                RefineKind::SkillNote,
                "run cargo test -p rs-agent before close",
                "review bead asked for tests",
            )
            .unwrap();
            apply(&note.id).unwrap();
            let skill = fs::read_to_string(learned_skill_path()).unwrap();
            assert!(skill.contains("cargo test"));
            assert!(skill.starts_with("---\nname: learned"));

            rollback().unwrap();
            let skill_after = fs::read_to_string(learned_skill_path()).unwrap_or_default();
            assert!(!skill_after.contains("cargo test"));
            let facts_mid = fs::read_to_string(crate::brain::facts_path()).unwrap_or_default();
            assert!(facts_mid.contains("parser rejects empty beads"));
            rollback().unwrap();
            let facts_after = fs::read_to_string(crate::brain::facts_path()).unwrap_or_default();
            assert!(!facts_after.contains("parser rejects empty beads"));
            assert_eq!(fs::read_to_string(&system).unwrap(), "IMMUTABLE");
        });
    }

    #[test]
    fn reject_does_not_write_memory() {
        crate::with_temp_cwd(|_dir| {
            let p = propose(
                RefineKind::Fact,
                "do not keep this fact",
                "evidence from the log",
            )
            .unwrap();
            reject(&p.id).unwrap();
            assert!(!crate::brain::facts_path().exists());
            assert!(apply(&p.id).is_err());
        });
    }
}
