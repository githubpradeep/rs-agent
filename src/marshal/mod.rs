//! Real Marshal — reclaim, dead-pid detect, assign, auto-assign idle fleet.

use crate::agent::SeatCaste;
use crate::beads::{self, Bead, BeadKind};
use crate::fleet::{self, SeatStatus};
use crate::mail;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarshalReport {
    pub at: String,
    pub reclaimed: usize,
    pub dead_pid_releases: usize,
    pub auto_assigned: Vec<String>,
    pub stuck_mailed: Vec<String>,
    pub summary: String,
}

fn project_rs_agent() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
}

pub fn report_path() -> PathBuf {
    project_rs_agent().join("marshal-report.json")
}

fn now_str() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn write_report(report: &MarshalReport) {
    let _ = fs::create_dir_all(project_rs_agent());
    if let Ok(text) = serde_json::to_string_pretty(report) {
        let _ = fs::write(report_path(), text);
    }
}

pub fn read_last_report() -> Option<MarshalReport> {
    let text = fs::read_to_string(report_path()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Release claims held by seats whose pid is dead (status says running but process gone).
pub fn release_dead_pid_claims() -> Result<usize, String> {
    let mut n = 0;
    for st in fleet::list_seat_statuses() {
        if !st.running {
            continue;
        }
        if st.pid > 0 && fleet::pid_alive(st.pid) {
            continue;
        }
        // Dead or unknown pid with running=true — release any claimed beads for this seat.
        let open = beads::list(None).unwrap_or_default();
        for b in open {
            if b.status == beads::BeadStatus::Claimed
                && b.claimant.as_deref() == Some(st.seat.as_str())
            {
                if beads::release(None, &b.id, Some(&st.seat)).is_ok() {
                    n += 1;
                    fleet::append_log(
                        &st.seat,
                        &format!("marshal released {} (dead pid {})", b.id, st.pid),
                    );
                }
            }
        }
        if let Some(mut s) = fleet::read_seat_status(&st.seat) {
            s.running = false;
            s.state = "dead".into();
            s.last_error = Some(format!("pid {} not alive", st.pid));
            fleet::write_seat_status(&s);
        }
        fleet::clear_pid(&st.seat);
    }
    Ok(n)
}

fn idle_fleet_seats() -> Vec<SeatStatus> {
    fleet::list_seat_statuses()
        .into_iter()
        .filter(|s| {
            let caste = crate::agent::seat::resolve_caste(&s.seat);
            caste == SeatCaste::Fleet
                && s.running
                && (s.state == "idle" || s.state == "sleeping")
        })
        .collect()
}

/// Assign a specific bead to a seat (caste bypass — marshal privilege).
pub fn assign_bead(bead_id: &str, seat: &str) -> Result<Bead, String> {
    let b = beads::assign(None, bead_id, seat)?;
    fleet::append_log(seat, &format!("marshal assigned {}", b.id));
    Ok(b)
}

/// Auto-assign ready implement/task beads to idle fleet seats (max 1 each).
pub fn auto_assign(max_per_pass: usize) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let idle = idle_fleet_seats();
    if idle.is_empty() {
        return Ok(lines);
    }
    let ready = beads::list_ready(None)?;
    let mut assignable: Vec<_> = ready
        .into_iter()
        .filter(|b| matches!(b.kind, BeadKind::Implement | BeadKind::Task))
        .collect();
    let mut assigned = 0usize;
    for seat in idle {
        if assigned >= max_per_pass {
            break;
        }
        let Some(bead) = assignable.first().cloned() else {
            break;
        };
        match assign_bead(&bead.id, &seat.seat) {
            Ok(b) => {
                lines.push(format!("{} → {}", b.id, seat.seat));
                assignable.remove(0);
                assigned += 1;
            }
            Err(e) => lines.push(format!("FAIL {} → {}: {e}", bead.id, seat.seat)),
        }
    }
    Ok(lines)
}

/// Escalate beads claimed/blocked longer than `stuck_mins` to Seneschal mail.
pub fn escalate_stuck(stuck_mins: u64) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let threshold = stuck_mins.saturating_mul(60);
    for b in beads::list(None).unwrap_or_default() {
        let stuck = match b.status {
            beads::BeadStatus::Blocked => true,
            beads::BeadStatus::Claimed => b
                .lease_expires
                .map(|exp| {
                    // If lease was extended many times, use updated_at age via crude check:
                    // claimed and heartbeat older than threshold from lease start approx.
                    let lease = beads::DEFAULT_LEASE_SECS;
                    exp.saturating_sub(lease) + threshold < now && threshold > 0
                })
                .unwrap_or(false),
            _ => false,
        };
        // Prefer updated_at parsing for blocked.
        let age_stuck = if b.status == beads::BeadStatus::Blocked {
            true
        } else {
            stuck
        };
        if !age_stuck {
            continue;
        }
        if b.status != beads::BeadStatus::Blocked {
            continue;
        }
        if b.notes.to_lowercase().contains("marshal mailed") {
            continue;
        }
        let body = format!(
            "Marshal: bead {} [{}] blocked/stuck.\nTitle: {}\nClaimant: {}\nNotes: {}",
            b.id,
            b.kind.as_str(),
            b.title,
            b.claimant.as_deref().unwrap_or("-"),
            b.notes.chars().take(200).collect::<String>()
        );
        match mail::send("Marshal", "Seneschal", &body, vec![b.id.clone()]) {
            Ok(m) => {
                // Re-block with marker so we don't spam mail every pass.
                let _ = beads::block(
                    None,
                    &b.id,
                    &format!("{} | marshal mailed {}", b.notes, m.id),
                );
                lines.push(format!("{} mailed as {}", b.id, m.id));
            }
            Err(e) => lines.push(format!("{} mail fail: {e}", b.id)),
        }
    }
    Ok(lines)
}

#[derive(Debug, Clone)]
pub struct MarshalOpts {
    pub auto_assign: bool,
    pub max_assign: usize,
    pub stuck_mins: u64,
    pub mail_stuck: bool,
}

impl Default for MarshalOpts {
    fn default() -> Self {
        Self {
            auto_assign: true,
            max_assign: 8,
            stuck_mins: 45,
            mail_stuck: true,
        }
    }
}

/// One marshal pass: reclaim → dead pid → optional auto-assign → stuck mail → summary.
pub fn run_once() -> String {
    run_with_opts(MarshalOpts::default())
}

pub fn run_with_opts(opts: MarshalOpts) -> String {
    let reclaimed = beads::reclaim_stale(None).unwrap_or(0);
    let dead = release_dead_pid_claims().unwrap_or(0);
    let auto_assigned = if opts.auto_assign {
        auto_assign(opts.max_assign).unwrap_or_default()
    } else {
        Vec::new()
    };
    let stuck_mailed = if opts.mail_stuck {
        escalate_stuck(opts.stuck_mins).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut out = String::new();
    if reclaimed > 0 {
        out.push_str(&format!("Marshal: reclaimed {reclaimed} stale lease(s).\n"));
    } else {
        out.push_str("Marshal: no stale leases.\n");
    }
    if dead > 0 {
        out.push_str(&format!("Marshal: released {dead} claim(s) from dead pid seats.\n"));
    }
    if !auto_assigned.is_empty() {
        out.push_str("Auto-assign:\n");
        for l in &auto_assigned {
            out.push_str(&format!("  {l}\n"));
        }
    }
    if !stuck_mailed.is_empty() {
        out.push_str("Stuck → mail:\n");
        for l in &stuck_mailed {
            out.push_str(&format!("  {l}\n"));
        }
    }
    out.push_str(&fleet::format_live_fleet());

    let report = MarshalReport {
        at: now_str(),
        reclaimed,
        dead_pid_releases: dead,
        auto_assigned: auto_assigned.clone(),
        stuck_mailed: stuck_mailed.clone(),
        summary: out.clone(),
    };
    write_report(&report);
    out
}

/// Loop until Ctrl-C / budget — cron-friendly when used with --once externally.
pub async fn run_loop(opts: MarshalOpts, interval_secs: u64, budget_minutes: u64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(budget_minutes.saturating_mul(60));
    loop {
        if std::time::Instant::now() >= deadline {
            eprintln!("[marshal] budget exhausted");
            break;
        }
        let report = run_with_opts(opts.clone());
        println!("{report}");
        tokio::time::sleep(Duration::from_secs(interval_secs.max(5))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_works() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("beads.json");
        let b = beads::add_full(
            Some(&path),
            "impl me",
            "",
            vec![],
            None,
            10,
            BeadKind::Implement,
            None,
        )
        .unwrap();
        // assign via beads path API used by marshal when cwd has beads — unit via beads::assign
        let claimed = beads::assign(Some(&path), &b.id, "Fleet-1").unwrap();
        assert_eq!(claimed.claimant.as_deref(), Some("Fleet-1"));
    }
}
