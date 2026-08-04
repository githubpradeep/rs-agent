//! Per-seat fleet observability — status JSON, rolling logs, live TUI view.
//!
//! Layout:
//! ```text
//! .rs-agent/fleet/<seat-slug>.status.json
//! .rs-agent/fleet/<seat-slug>.log
//! ```

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn project_rs_agent() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
}

pub fn fleet_dir() -> PathBuf {
    project_rs_agent().join("fleet")
}

pub fn seat_slug(seat: &str) -> String {
    crate::agent::seat::slugify(seat)
}

pub fn status_path(seat: &str) -> PathBuf {
    fleet_dir().join(format!("{}.status.json", seat_slug(seat)))
}

pub fn log_path(seat: &str) -> PathBuf {
    fleet_dir().join(format!("{}.log", seat_slug(seat)))
}

/// Legacy aggregate path (still written for `/worker` compatibility).
pub fn legacy_worker_status_path() -> PathBuf {
    project_rs_agent().join("worker-status.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SeatStatus {
    pub seat: String,
    pub pid: u32,
    pub updated_at: String,
    pub heartbeat_at: String,
    /// idle | claiming | working | sleeping | stopped | error
    pub state: String,
    pub last_bead: Option<String>,
    pub last_title: Option<String>,
    pub last_tool: Option<String>,
    pub last_line: Option<String>,
    pub last_error: Option<String>,
    pub session_id: Option<String>,
    pub beads_closed: u32,
    pub beads_blocked: u32,
    pub model: Option<String>,
    pub running: bool,
}

fn now_str() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn ensure_fleet_dir() {
    let _ = fs::create_dir_all(fleet_dir());
}

pub fn write_seat_status(status: &SeatStatus) {
    ensure_fleet_dir();
    let path = status_path(&status.seat);
    if let Ok(text) = serde_json::to_string_pretty(status) {
        let tmp = path.with_extension("json.tmp");
        let _ = fs::write(&tmp, &text);
        let _ = fs::rename(&tmp, &path);
    }
    // Keep legacy aggregate for `/worker`.
    let legacy = serde_json::json!({
        "updated_at": status.updated_at,
        "claimant": status.seat,
        "last_bead": status.last_bead,
        "last_title": status.last_title,
        "last_error": status.last_error,
        "beads_closed": status.beads_closed,
        "beads_blocked": status.beads_blocked,
        "running": status.running,
        "session_id": status.session_id,
        "state": status.state,
        "last_tool": status.last_tool,
        "last_line": status.last_line,
        "pid": status.pid,
    });
    if let Ok(text) = serde_json::to_string_pretty(&legacy) {
        let lp = legacy_worker_status_path();
        let tmp = lp.with_extension("json.tmp");
        let _ = fs::write(&tmp, &text);
        let _ = fs::rename(&tmp, &lp);
    }
}

pub fn read_seat_status(seat: &str) -> Option<SeatStatus> {
    let text = fs::read_to_string(status_path(seat)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn list_seat_statuses() -> Vec<SeatStatus> {
    let dir = fleet_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".status.json"))
            .unwrap_or(false)
        {
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str::<SeatStatus>(&text) {
                    out.push(s);
                }
            }
        }
    }
    out.sort_by(|a, b| a.seat.cmp(&b.seat));
    out
}

pub fn append_log(seat: &str, line: &str) {
    ensure_fleet_dir();
    let path = log_path(seat);
    let ts = now_str();
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "[{ts}] {line}");
    }
    // Cap log size (~2MB) by truncating head when oversized.
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > 2_000_000 {
            if let Ok(text) = fs::read_to_string(&path) {
                let keep = &text[text.len().saturating_sub(1_000_000)..];
                let _ = fs::write(&path, keep);
            }
        }
    }
}

pub fn tail_log(seat: &str, max_lines: usize) -> String {
    let path = log_path(seat);
    let Ok(text) = fs::read_to_string(&path) else {
        return format!("No log for seat `{seat}` ({})", path.display());
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

pub fn pid_path(seat: &str) -> PathBuf {
    fleet_dir().join(format!("{}.pid", seat_slug(seat)))
}

pub fn read_pid(seat: &str) -> Option<u32> {
    let text = fs::read_to_string(pid_path(seat)).ok()?;
    text.trim().parse().ok()
}

pub fn write_pid(seat: &str, pid: u32) {
    ensure_fleet_dir();
    let _ = fs::write(pid_path(seat), format!("{pid}\n"));
}

pub fn clear_pid(seat: &str) {
    let _ = fs::remove_file(pid_path(seat));
}

fn kill_pid(pid: u32) -> Result<(), String> {
    if !pid_alive(pid) {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let status = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|e| format!("kill: {e}"))?;
        if !status.success() {
            return Err(format!("kill -TERM {pid} failed"));
        }
        // Brief wait; escalate to KILL if needed.
        std::thread::sleep(std::time::Duration::from_millis(400));
        if pid_alive(pid) {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(format!("fleet down not supported on this OS (pid {pid})"))
    }
}

/// Options for `fleet up`.
#[derive(Debug, Clone)]
pub struct FleetUpOpts {
    pub seats: Vec<String>,
    pub budget_minutes: u64,
    pub sleep_secs: u64,
    pub quiet: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub approve: bool,
    pub fail_fast: bool,
}

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("current_exe: {e}"))
}

fn parse_seats(raw: &str) -> Vec<String> {
    raw.split(|c| c == ',' || c == ' ')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Spawn one worker process for `seat`. Returns child pid.
pub fn spawn_worker(seat: &str, opts: &FleetUpOpts) -> Result<u32, String> {
    ensure_fleet_dir();
    if let Some(pid) = read_pid(seat) {
        if pid_alive(pid) {
            return Err(format!(
                "seat `{seat}` already running (pid {pid}). `fleet down` first or pick another seat."
            ));
        }
        clear_pid(seat);
    }

    let exe = current_exe()?;
    let spawn_log = fleet_dir().join(format!("{}.spawn.log", seat_slug(seat)));
    let spawn_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&spawn_log)
        .map_err(|e| format!("spawn log: {e}"))?;
    let spawn_err = spawn_file
        .try_clone()
        .map_err(|e| format!("spawn log clone: {e}"))?;

    let mut cmd = std::process::Command::new("nohup");
    cmd.arg(&exe);
    if opts.approve {
        cmd.arg("-a");
    }
    if let Some(ref p) = opts.provider {
        cmd.args(["--provider", p]);
    }
    if let Some(ref m) = opts.model {
        cmd.args(["--model", m]);
    }
    cmd.args([
        "worker",
        "--loop",
        "--budget-minutes",
        &opts.budget_minutes.to_string(),
        "--sleep-secs",
        &opts.sleep_secs.to_string(),
        "--seat",
        seat,
    ]);
    if opts.quiet {
        cmd.arg("--quiet");
    }
    if opts.fail_fast {
        cmd.arg("--fail-fast");
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::from(spawn_file));
    cmd.stderr(std::process::Stdio::from(spawn_err));
    // nohup ignores SIGHUP when the launching terminal closes.

    let child = cmd.spawn().map_err(|e| format!("spawn worker `{seat}`: {e}"))?;
    let pid = child.id();
    // Note: nohup's pid is the nohup wrapper on some systems; worker is child.
    // Prefer reading status file later; also try to find worker via pgrep is overkill.
    write_pid(seat, pid);
    append_log(seat, &format!("fleet up spawned nohup pid={pid}"));
    std::mem::forget(child);
    Ok(pid)
}

/// Start fleet seats. Returns a human report.
pub fn fleet_up(opts: FleetUpOpts) -> Result<String, String> {
    if opts.seats.is_empty() {
        return Err("fleet up requires --seats Fleet-1,Fleet-2".into());
    }
    let mut out = String::from("Fleet up:\n");
    for seat in &opts.seats {
        // Seal fleet caste so claim routing only picks implement/task.
        match crate::agent::seat::ensure_with_caste(seat, crate::agent::SeatCaste::Fleet) {
            Ok(s) => out.push_str(&format!(
                "  seat {} caste={}\n",
                s.name,
                s.effective_caste().as_str()
            )),
            Err(e) => out.push_str(&format!("  WARN seat `{seat}` profile: {e}\n")),
        }
        match spawn_worker(seat, &opts) {
            Ok(pid) => out.push_str(&format!("  started {seat} pid={pid}\n")),
            Err(e) => out.push_str(&format!("  FAIL {seat}: {e}\n")),
        }
    }
    out.push('\n');
    out.push_str(&format_live_fleet());
    Ok(out)
}

/// Stop fleet workers (by pid files + running status).
pub fn fleet_down(seats: Option<Vec<String>>) -> String {
    ensure_fleet_dir();
    let targets: Vec<String> = if let Some(list) = seats {
        list
    } else {
        // All pid files + running statuses.
        let mut names = Vec::new();
        if let Ok(entries) = fs::read_dir(fleet_dir()) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if let Some(slug) = name.strip_suffix(".pid") {
                    if let Some(s) = list_seat_statuses()
                        .into_iter()
                        .find(|s| seat_slug(&s.seat) == slug)
                    {
                        names.push(s.seat);
                    } else {
                        names.push(slug.to_string());
                    }
                }
            }
        }
        for s in list_seat_statuses() {
            if s.running && !names.iter().any(|n| seat_slug(n) == seat_slug(&s.seat)) {
                names.push(s.seat);
            }
        }
        names.sort();
        names.dedup();
        names
    };

    let mut out = String::from("Fleet down:\n");
    if targets.is_empty() {
        out.push_str("  (no pid files / running seats found)\n");
        return out;
    }
    for seat in targets {
        // Prefer live status pid (actual worker) over launcher pid file.
        let status_pid = read_seat_status(&seat).map(|s| s.pid).filter(|p| *p > 0);
        let file_pid = read_pid(&seat);
        let pid = match (status_pid, file_pid) {
            (Some(sp), _) if pid_alive(sp) => Some(sp),
            (_, Some(fp)) if pid_alive(fp) => Some(fp),
            (Some(sp), _) => Some(sp),
            (_, fp) => fp,
        };
        match pid {
            Some(pid) if pid > 0 => match kill_pid(pid) {
                Ok(()) => {
                    clear_pid(&seat);
                    if let Some(mut st) = read_seat_status(&seat) {
                        st.running = false;
                        st.state = "stopped".into();
                        write_seat_status(&st);
                    }
                    append_log(&seat, &format!("fleet down killed pid={pid}"));
                    out.push_str(&format!("  stopped {seat} pid={pid}\n"));
                }
                Err(e) => out.push_str(&format!("  FAIL {seat} pid={pid}: {e}\n")),
            },
            _ => {
                clear_pid(&seat);
                out.push_str(&format!("  {seat}: no live pid\n"));
            }
        }
    }
    out.push('\n');
    out.push_str(&format_live_fleet());
    out
}

pub fn fleet_status() -> String {
    format_live_fleet()
}

pub fn fleet_logs(seat: &str, lines: usize) -> String {
    format!(
        "Log tail for `{seat}` ({})\n{}\n\n--- spawn log ---\n{}",
        log_path(seat).display(),
        tail_log(seat, lines),
        {
            let p = fleet_dir().join(format!("{}.spawn.log", seat_slug(seat)));
            fs::read_to_string(p).unwrap_or_else(|_| "(no spawn log)".into())
                .lines()
                .rev()
                .take(30)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        }
    )
}

/// Parse `/fleet` or CLI seat lists.
pub fn parse_seat_list(raw: &str) -> Vec<String> {
    parse_seats(raw)
}

/// Live fleet view for TUI / CLI (seats + bead leases).
pub fn format_live_fleet() -> String {
    let mut out = String::new();
    out.push_str(&crate::beads::format_backlog_idle_message());
    out.push('\n');

    let seats = list_seat_statuses();
    let mut pid_seats: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(fleet_dir()) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(slug) = name.strip_suffix(".pid") {
                if let Some(pid) = read_pid(slug) {
                    pid_seats.push(format!("{slug} pid={pid} alive={}", pid_alive(pid)));
                }
            }
        }
    }

    if seats.is_empty() && pid_seats.is_empty() {
        out.push_str("No fleet seats yet.\n");
        out.push_str("Start: rs-agent -a fleet up --seats Fleet-1,Fleet-2\n");
    } else {
        if !pid_seats.is_empty() {
            out.push_str("Pid files:\n");
            for line in &pid_seats {
                out.push_str(&format!("  {line}\n"));
            }
        }
        if seats.is_empty() {
            out.push_str("No seat status files yet (workers still starting?).\n");
        } else {
            out.push_str("Seats:\n");
            for s in &seats {
                let managed = read_pid(&s.seat);
                let alive = match managed.or(Some(s.pid)) {
                    Some(pid) if pid_alive(pid) => "alive",
                    Some(_) if s.running => "stale-pid?",
                    _ if s.running => "stale?",
                    _ => "stopped",
                };
                out.push_str(&format!(
                    "  {} [{}] pid={} {} hb={}\n    bead: {} — {}\n    tool: {}\n    line: {}\n    session: {}  closed={} blocked={}\n",
                    s.seat,
                    s.state,
                    s.pid,
                    alive,
                    s.heartbeat_at,
                    s.last_bead.as_deref().unwrap_or("-"),
                    s.last_title.as_deref().unwrap_or("-"),
                    s.last_tool.as_deref().unwrap_or("-"),
                    s.last_line
                        .as_deref()
                        .unwrap_or("-")
                        .chars()
                        .take(100)
                        .collect::<String>(),
                    s.session_id.as_deref().unwrap_or("-"),
                    s.beads_closed,
                    s.beads_blocked,
                ));
                if let Some(err) = &s.last_error {
                    if !err.is_empty() {
                        out.push_str(&format!(
                            "    error: {}\n",
                            err.chars().take(120).collect::<String>()
                        ));
                    }
                }
            }
        }
    }

    out.push('\n');
    out.push_str(&crate::beads::format_fleet_status(None));
    out
}

pub fn format_worker_help(seat: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(seat) = seat {
        if let Some(s) = read_seat_status(seat) {
            out.push_str(&format!(
                "Seat `{}` ({})\nstate={} running={} pid={}\nbead: {} — {}\ntool: {}\nline: {}\nsession: {}\nlog: {}\n",
                s.seat,
                s.updated_at,
                s.state,
                s.running,
                s.pid,
                s.last_bead.as_deref().unwrap_or("-"),
                s.last_title.as_deref().unwrap_or("-"),
                s.last_tool.as_deref().unwrap_or("-"),
                s.last_line.as_deref().unwrap_or("-"),
                s.session_id.as_deref().unwrap_or("-"),
                log_path(seat).display(),
            ));
            out.push_str("\n--- log tail ---\n");
            out.push_str(&tail_log(seat, 40));
            return out;
        }
    }
    // Fall back to all seats + legacy.
    out.push_str(&format_live_fleet());
    if let Ok(text) = fs::read_to_string(legacy_worker_status_path()) {
        out.push_str("\n--- legacy worker-status.json ---\n");
        out.push_str(&text);
    }
    out
}

/// Touch heartbeat fields on an existing status.
pub fn heartbeat_touch(status: &mut SeatStatus, line: Option<&str>) {
    let t = now_str();
    status.heartbeat_at = t.clone();
    status.updated_at = t;
    if let Some(l) = line {
        status.last_line = Some(l.chars().take(200).collect());
    }
}

pub fn new_working_status(seat: &str, model: Option<&str>) -> SeatStatus {
    let t = now_str();
    SeatStatus {
        seat: seat.to_string(),
        pid: std::process::id(),
        updated_at: t.clone(),
        heartbeat_at: t,
        state: "idle".into(),
        model: model.map(|s| s.to_string()),
        running: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_roundtrip_in_temp_cwd() {
        crate::with_temp_cwd(|_| {
            let mut s = new_working_status("Fleet-1", Some("m"));
            s.state = "working".into();
            s.last_bead = Some("b1".into());
            write_seat_status(&s);
            let loaded = read_seat_status("Fleet-1").unwrap();
            assert_eq!(loaded.last_bead.as_deref(), Some("b1"));
            append_log("Fleet-1", "hello");
            let tail = tail_log("Fleet-1", 5);
            assert!(tail.contains("hello"), "tail={tail:?}");
        });
    }
}
