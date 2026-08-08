//! Cron-like schedules for marshal/worker wake (Conductor WorkflowSchedule-inspired).

use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub name: String,
    /// Five-field cron: minute hour dom month dow (simplified matcher).
    pub cron: String,
    pub command: String,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub catchup: bool,
    #[serde(default)]
    pub last_run_at: Option<String>,
}

fn path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("schedules.json")
}

pub fn load_all() -> Vec<Schedule> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_all(items: &[Schedule]) -> Result<(), String> {
    let p = path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Very small cron matcher: supports `*` and exact numbers for each field.
pub fn cron_matches(cron: &str, now: &chrono::DateTime<chrono::Local>) -> bool {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }
    let vals = [
        now.minute() as u32,
        now.hour() as u32,
        now.day() as u32,
        now.month() as u32,
        now.weekday().number_from_sunday() % 7, // 0=Sun
    ];
    for (i, part) in parts.iter().enumerate() {
        if *part == "*" {
            continue;
        }
        let Ok(n) = part.parse::<u32>() else {
            return false;
        };
        if n != vals[i] {
            return false;
        }
    }
    true
}

/// Return schedules that should fire now (and optionally mark last_run).
pub fn due_now(mark: bool) -> Vec<Schedule> {
    let now = chrono::Local::now();
    let mut items = load_all();
    let mut due = Vec::new();
    for s in &mut items {
        if s.paused {
            continue;
        }
        if cron_matches(&s.cron, &now) {
            // Avoid double-fire in same minute.
            let stamp = now.format("%Y-%m-%d %H:%M").to_string();
            if s.last_run_at.as_deref() == Some(stamp.as_str()) {
                continue;
            }
            if mark {
                s.last_run_at = Some(stamp);
            }
            due.push(s.clone());
        }
    }
    if mark && !due.is_empty() {
        let _ = save_all(&items);
    }
    due
}

/// PLAN_EXECUTE MVP: turn a planner bullet list into implement beads.
pub fn plan_execute_emit_beads(plan_text: &str) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for (i, line) in plan_text.lines().enumerate() {
        let line = line.trim();
        let title = line
            .trim_start_matches(|c: char| c.is_ascii_digit() || ".)-•* ".contains(c))
            .trim();
        if title.is_empty() {
            continue;
        }
        let id = format!("plan-{}", i + 1);
        // Best-effort create via beads API if available.
        match crate::beads::add_full(
            None,
            title,
            "plan_execute",
            Vec::new(),
            None,
            50,
            crate::beads::BeadKind::Implement,
            None,
        ) {
            Ok(b) => ids.push(b.id),
            Err(_) => ids.push(id),
        }
    }
    if ids.is_empty() {
        return Err("no plan lines found".into());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_star_matches() {
        let now = chrono::Local::now();
        assert!(cron_matches("* * * * *", &now));
    }
}
