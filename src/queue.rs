//! Priority + postpone ready queue (Conductor QueueDAO-inspired, file-backed).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub priority: i32,
    /// Unix secs; item is invisible until then.
    pub available_at: i64,
    pub payload: String,
}

fn queue_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("ready-queue.json")
}

fn load(path: &Path) -> Vec<QueueItem> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(path: &Path, items: &[QueueItem]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

fn now() -> i64 {
    chrono::Local::now().timestamp()
}

pub fn push(id: &str, priority: i32, payload: &str, postpone_secs: u64) -> Result<(), String> {
    let path = queue_path();
    let mut items = load(&path);
    items.retain(|i| i.id != id);
    items.push(QueueItem {
        id: id.to_string(),
        priority,
        available_at: now() + postpone_secs as i64,
        payload: payload.to_string(),
    });
    items.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.available_at.cmp(&b.available_at))
    });
    save(&path, &items)
}

pub fn postpone(id: &str, secs: u64) -> Result<(), String> {
    let path = queue_path();
    let mut items = load(&path);
    if let Some(i) = items.iter_mut().find(|i| i.id == id) {
        i.available_at = now() + secs as i64;
        save(&path, &items)
    } else {
        Err(format!("queue item `{id}` not found"))
    }
}

/// Pop highest-priority available item.
pub fn pop() -> Result<Option<QueueItem>, String> {
    let path = queue_path();
    let mut items = load(&path);
    let now = now();
    if let Some(idx) = items.iter().position(|i| i.available_at <= now) {
        let item = items.remove(idx);
        save(&path, &items)?;
        Ok(Some(item))
    } else {
        Ok(None)
    }
}

pub fn list_ready() -> Vec<QueueItem> {
    let now = now();
    let mut items = load(&queue_path());
    items.retain(|i| i.available_at <= now);
    items.sort_by(|a, b| b.priority.cmp(&a.priority));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_postpone_pop() {
        crate::with_temp_cwd(|_p| {
            push("a", 1, "x", 0).unwrap();
            push("b", 10, "y", 0).unwrap();
            let item = pop().unwrap().unwrap();
            assert_eq!(item.id, "b");
        });
    }
}
