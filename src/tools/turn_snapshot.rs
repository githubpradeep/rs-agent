//! Content-hash turn snapshots for revert after agent mutations.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TURNS: usize = 20;

#[derive(Debug, Clone)]
pub struct TrackedFile {
    pub path: PathBuf,
    pub content_hash: String,
    pub existed: bool,
    pub prior_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct TurnSnapshot {
    pub id: String,
    pub files: HashMap<String, TrackedFile>,
}

#[derive(Default)]
struct StoreInner {
    session_id: String,
    current: Option<TurnSnapshot>,
    history: Vec<TurnSnapshot>,
}

static STORE: Mutex<StoreInner> = Mutex::new(StoreInner {
    session_id: String::new(),
    current: None,
    history: Vec::new(),
});

fn snapshots_root() -> PathBuf {
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".rs-agent").join("snapshots")
}

fn content_hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())[..16].to_string()
}

fn new_turn_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("turn_{ms}")
}

/// Bind snapshots to a session id (creates dirs lazily).
pub fn set_session(session_id: &str) {
    if let Ok(mut g) = STORE.lock() {
        g.session_id = session_id.to_string();
    }
}

/// Begin a new turn (call at start of each agent user turn).
pub fn begin_turn() {
    if let Ok(mut g) = STORE.lock() {
        if let Some(prev) = g.current.take() {
            if !prev.files.is_empty() {
                g.history.push(prev);
                while g.history.len() > MAX_TURNS {
                    if let Some(old) = g.history.first() {
                        let _ = remove_turn_dir(&g.session_id, &old.id);
                    }
                    g.history.remove(0);
                }
            }
        }
        g.current = Some(TurnSnapshot {
            id: new_turn_id(),
            files: HashMap::new(),
        });
    }
}

fn turn_dir(session_id: &str, turn_id: &str) -> PathBuf {
    snapshots_root().join(session_id).join(turn_id)
}

fn remove_turn_dir(session_id: &str, turn_id: &str) -> std::io::Result<()> {
    let dir = turn_dir(session_id, turn_id);
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// Record prior file contents before mutation (idempotent per path per turn).
pub fn track(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(p)
    };
    let key = abs.to_string_lossy().to_string();

    let mut g = STORE.lock().map_err(|e| e.to_string())?;
    if g.current.is_none() {
        g.current = Some(TurnSnapshot {
            id: new_turn_id(),
            files: HashMap::new(),
        });
    }
    let session = g.session_id.clone();
    let turn = g.current.as_ref().unwrap().id.clone();
    if g.current.as_ref().unwrap().files.contains_key(&key) {
        return Ok(());
    }

    let (existed, prior_bytes, hash) = if abs.exists() {
        let bytes = fs::read(&abs).map_err(|e| e.to_string())?;
        let hash = content_hash(&bytes);
        (true, Some(bytes), hash)
    } else {
        (false, None, "missing".into())
    };

    // Persist prior bytes to disk for durability across process if needed.
    let dir = turn_dir(&session, &turn);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe_name = key.replace('/', "_").replace('\\', "_");
    if let Some(ref bytes) = prior_bytes {
        fs::write(dir.join(format!("{safe_name}.bin")), bytes).map_err(|e| e.to_string())?;
    }
    fs::write(
        dir.join(format!("{safe_name}.meta")),
        format!("path={key}\nexisted={existed}\nhash={hash}\n"),
    )
    .map_err(|e| e.to_string())?;

    g.current.as_mut().unwrap().files.insert(
        key.clone(),
        TrackedFile {
            path: abs,
            content_hash: hash,
            existed,
            prior_bytes,
        },
    );
    Ok(())
}

/// Restore the last completed or current turn's tracked files.
pub fn restore_last_turn() -> Result<usize, String> {
    let mut g = STORE.lock().map_err(|e| e.to_string())?;
    let snap = if let Some(cur) = g.current.take() {
        if cur.files.is_empty() {
            g.history.pop()
        } else {
            Some(cur)
        }
    } else {
        g.history.pop()
    };
    let Some(snap) = snap else {
        return Err("No turn snapshot to restore.".into());
    };
    let mut n = 0usize;
    for (_k, tracked) in snap.files {
        if tracked.existed {
            if let Some(bytes) = tracked.prior_bytes {
                if let Some(parent) = tracked.path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&tracked.path, bytes).map_err(|e| e.to_string())?;
                n += 1;
            }
        } else if tracked.path.exists() {
            fs::remove_file(&tracked.path).map_err(|e| e.to_string())?;
            n += 1;
        }
    }
    let _ = remove_turn_dir(&g.session_id, &snap.id);
    Ok(n)
}

/// Summary of files tracked in the current turn.
pub fn current_tracked_summary() -> String {
    let Ok(g) = STORE.lock() else {
        return String::new();
    };
    let Some(cur) = &g.current else {
        return String::new();
    };
    if cur.files.is_empty() {
        return String::new();
    }
    let paths: Vec<_> = cur
        .files
        .values()
        .map(|t| t.path.display().to_string())
        .collect();
    format!(
        "snapshot: tracked {} file(s) this turn ({})",
        paths.len(),
        paths.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn track_and_restore() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, b"v1").unwrap();
        set_session("test-sess");
        begin_turn();
        track(file.to_str().unwrap()).unwrap();
        fs::write(&file, b"v2").unwrap();
        let n = restore_last_turn().unwrap();
        assert_eq!(n, 1);
        assert_eq!(fs::read_to_string(&file).unwrap(), "v1");
    }
}
