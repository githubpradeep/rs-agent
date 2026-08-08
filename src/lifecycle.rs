//! Session / seat lifecycle bus (herdr blocked/working/idle/done vocabulary).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// High-signal lifecycle published by the agent loop / TUI / workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Blocked,
    Working,
    Done,
    #[default]
    Idle,
    Unknown,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Working => "working",
            Self::Done => "done",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "blocked" | "ask" | "stuck" => Self::Blocked,
            "working" | "running" | "thinking" => Self::Working,
            "done" | "finished" => Self::Done,
            "idle" | "ready" => Self::Idle,
            _ => Self::Unknown,
        }
    }

    pub fn matches_until(self, until: Lifecycle) -> bool {
        self == until
            || (until == Lifecycle::Idle && matches!(self, Lifecycle::Idle | Lifecycle::Done))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleSnapshot {
    pub lifecycle: Lifecycle,
    pub detail: String,
    pub updated_at: String,
    pub session_id: Option<String>,
    pub seat: Option<String>,
}

impl LifecycleSnapshot {
    pub fn new(lifecycle: Lifecycle, detail: impl Into<String>) -> Self {
        Self {
            lifecycle,
            detail: detail.into(),
            updated_at: chrono::Local::now().to_rfc3339(),
            session_id: None,
            seat: None,
        }
    }
}

struct BusInner {
    snap: LifecycleSnapshot,
    seq: u64,
    /// When true, in-app UI is focused — suppress external notifications.
    focused: bool,
}

fn bus() -> &'static Mutex<BusInner> {
    static BUS: OnceLock<Mutex<BusInner>> = OnceLock::new();
    BUS.get_or_init(|| {
        Mutex::new(BusInner {
            snap: LifecycleSnapshot::new(Lifecycle::Idle, "ready"),
            seq: 0,
            focused: true,
        })
    })
}

/// Publish a lifecycle transition. Returns true if the value changed.
pub fn publish(lifecycle: Lifecycle, detail: impl Into<String>) -> bool {
    let detail = detail.into();
    let mut g = bus().lock().unwrap_or_else(|e| e.into_inner());
    let changed = g.snap.lifecycle != lifecycle || g.snap.detail != detail;
    if changed {
        g.snap.lifecycle = lifecycle;
        g.snap.detail = detail;
        g.snap.updated_at = chrono::Local::now().to_rfc3339();
        g.seq = g.seq.wrapping_add(1);
        let _ = write_snapshot_file(&g.snap);
    }
    changed
}

pub fn set_session(session_id: Option<String>, seat: Option<String>) {
    let mut g = bus().lock().unwrap_or_else(|e| e.into_inner());
    g.snap.session_id = session_id;
    g.snap.seat = seat;
    let _ = write_snapshot_file(&g.snap);
}

pub fn set_focused(focused: bool) {
    let mut g = bus().lock().unwrap_or_else(|e| e.into_inner());
    g.focused = focused;
}

pub fn is_focused() -> bool {
    bus()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .focused
}

pub fn snapshot() -> LifecycleSnapshot {
    bus()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .snap
        .clone()
}

pub fn seq() -> u64 {
    bus().lock().unwrap_or_else(|e| e.into_inner()).seq
}

fn status_file_path() -> PathBuf {
    let dir = crate::config::Config::user_config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    dir.join("lifecycle.json")
}

fn write_snapshot_file(snap: &LifecycleSnapshot) -> std::io::Result<()> {
    let path = status_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(snap).unwrap_or_default())?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn read_snapshot_file() -> Option<LifecycleSnapshot> {
    let raw = std::fs::read_to_string(status_file_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Block until lifecycle matches `until` or timeout.
pub fn wait_until(until: Lifecycle, timeout: Duration) -> Result<LifecycleSnapshot, String> {
    let start = Instant::now();
    loop {
        if let Some(snap) = read_snapshot_file() {
            if snap.lifecycle.matches_until(until) {
                return Ok(snap);
            }
        } else {
            let snap = snapshot();
            if snap.lifecycle.matches_until(until) {
                return Ok(snap);
            }
        }
        if start.elapsed() >= timeout {
            return Err(format!(
                "timeout waiting for lifecycle={}: last={:?}",
                until.as_str(),
                snapshot().lifecycle
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_match() {
        assert_eq!(Lifecycle::parse("blocked"), Lifecycle::Blocked);
        assert!(Lifecycle::Done.matches_until(Lifecycle::Idle));
        assert!(!Lifecycle::Working.matches_until(Lifecycle::Blocked));
    }
}
