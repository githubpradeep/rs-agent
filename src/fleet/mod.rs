//! Per-seat fleet observability — status JSON, rolling logs, live TUI view.
//!
//! Layout:
//! ```text
//! .rs-agent/fleet/<seat-slug>.status.json
//! .rs-agent/fleet/<seat-slug>.log
//! .rs-agent/fleet/<seat-slug>.control.jsonl
//! .rs-agent/fleet/<seat-slug>.control.offset
//! ```

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
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

pub fn control_path(seat: &str) -> PathBuf {
    fleet_dir().join(format!("{}.control.jsonl", seat_slug(seat)))
}

pub fn control_offset_path(seat: &str) -> PathBuf {
    fleet_dir().join(format!("{}.control.offset", seat_slug(seat)))
}

fn unix_now() -> i64 {
    chrono::Local::now().timestamp()
}

/// Append a control command for the worker to consume.
pub fn append_control(seat: &str, op: ControlOp, text: Option<&str>) -> ControlCommand {
    ensure_fleet_dir();
    let cmd = ControlCommand {
        id: uuid::Uuid::new_v4().to_string(),
        op,
        text: text.map(|s| s.to_string()),
        ts: now_str(),
    };
    if let Ok(line) = serde_json::to_string(&cmd) {
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(control_path(seat))
        {
            let _ = writeln!(f, "{line}");
        }
    }
    append_log(
        seat,
        &format!(
            "control {:?}{}",
            cmd.op,
            cmd.text
                .as_ref()
                .map(|t| format!(": {}", t.chars().take(80).collect::<String>()))
                .unwrap_or_default()
        ),
    );
    cmd
}

fn read_control_offset(seat: &str) -> u64 {
    fs::read_to_string(control_offset_path(seat))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

fn write_control_offset(seat: &str, offset: u64) {
    ensure_fleet_dir();
    let _ = fs::write(control_offset_path(seat), format!("{offset}\n"));
}

/// Read and acknowledge new control commands since last offset.
pub fn poll_control(seat: &str) -> Vec<ControlCommand> {
    let path = control_path(seat);
    let Ok(mut f) = fs::OpenOptions::new().read(true).open(&path) else {
        return Vec::new();
    };
    let mut offset = read_control_offset(seat);
    if f.seek(SeekFrom::Start(offset)).is_err() {
        offset = 0;
        let _ = f.seek(SeekFrom::Start(0));
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    let new_offset = offset + buf.len() as u64;
    let mut out = Vec::new();
    for line in buf.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(cmd) = serde_json::from_str::<ControlCommand>(line) {
            out.push(cmd);
        }
    }
    if new_offset != offset {
        write_control_offset(seat, new_offset);
    }
    out
}

/// Mark seat paused (worker side ack).
pub fn set_paused(status: &mut SeatStatus, reason: &str) {
    status.state = "paused".into();
    status.paused_reason = Some(reason.to_string());
    status.paused_at = Some(unix_now());
    heartbeat_touch(status, Some(&format!("paused: {reason}")));
}

/// Mark seat waiting on a human (Conductor HUMAN / headless question).
pub fn set_awaiting_human(status: &mut SeatStatus, prompt: &str) {
    status.awaiting_human = Some(true);
    status.human_prompt = Some(prompt.to_string());
    status.lifecycle = Some("blocked".into());
    status.state = "paused".into();
    heartbeat_touch(status, Some(&format!("awaiting_human: {prompt}")));
}

/// Clear human-wait and resume idle.
pub fn clear_awaiting_human(status: &mut SeatStatus) {
    status.awaiting_human = None;
    status.human_prompt = None;
    if status.lifecycle.as_deref() == Some("blocked") {
        status.lifecycle = Some("idle".into());
    }
    if status.state == "paused" {
        status.state = "idle".into();
    }
    heartbeat_touch(status, Some("human resumed"));
}

/// Headless resume: clear `awaiting_human`, nudge worker via control.jsonl.
pub fn resume_human(seat: &str, answer: &str) -> Result<String, String> {
    let path = status_path(seat);
    let mut status: SeatStatus = if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())?
    } else {
        return Err(format!("no status file for seat `{seat}`"));
    };
    let prompt = status
        .human_prompt
        .clone()
        .unwrap_or_else(|| "(no prompt)".into());
    clear_awaiting_human(&mut status);
    clear_paused(&mut status);
    write_seat_status(&status);
    append_control(seat, ControlOp::Resume, Some(answer));
    if !answer.trim().is_empty() {
        append_control(seat, ControlOp::Steer, Some(answer));
    }
    Ok(format!(
        "resumed seat `{seat}` (was: {prompt}) answer={}",
        answer.chars().take(120).collect::<String>()
    ))
}

/// Clear pause fields after resume.
pub fn clear_paused(status: &mut SeatStatus) {
    status.paused_reason = None;
    status.paused_at = None;
    if status.state == "paused" || status.state == "attached" {
        status.state = "idle".into();
    }
    heartbeat_touch(status, Some("resumed"));
}

/// True when pause has exceeded [`PAUSE_TTL_SECS`].
pub fn pause_expired(status: &SeatStatus) -> bool {
    if status.state != "paused" && status.state != "attached" {
        return false;
    }
    let Some(at) = status.paused_at else {
        return false;
    };
    unix_now().saturating_sub(at) as u64 >= PAUSE_TTL_SECS
}

/// Strip `[ts] ` prefix if present; classify the body.
pub fn parse_log_line(raw: &str) -> ParsedLogLine {
    let (timestamp, body) = if let Some(rest) = raw.strip_prefix('[') {
        if let Some(idx) = rest.find(']') {
            let ts = rest[..idx].to_string();
            let body = rest[idx + 1..].trim_start();
            (Some(ts), body)
        } else {
            (None, raw)
        }
    } else {
        (None, raw)
    };
    let lower = body.to_lowercase();
    let kind = if body.starts_with("→ tool ") || body.starts_with("-> tool ") {
        LogKind::Tool
    } else if body.starts_with("← ") || body.starts_with("<- ") {
        LogKind::ToolResult
    } else if lower.starts_with("say:") {
        LogKind::Say
    } else if lower.contains("heartbeat") {
        LogKind::Heartbeat
    } else if lower.starts_with("claimed ") || lower.contains(" claimed ") {
        LogKind::Claimed
    } else if lower.starts_with("closed ") {
        LogKind::Closed
    } else if lower.starts_with("session ") || lower.starts_with("saved session") {
        LogKind::Session
    } else if lower.starts_with("error:") || lower.contains(" failed:") {
        LogKind::Error
    } else if lower.starts_with("control ")
        || lower.starts_with("paused")
        || lower.starts_with("resumed")
        || lower.starts_with("worker ")
    {
        LogKind::Status
    } else {
        LogKind::Raw
    };
    ParsedLogLine {
        timestamp,
        kind,
        body: body.to_string(),
        raw: raw.to_string(),
    }
}

/// Format a parsed line for plain CLI output.
pub fn format_log_line_plain(p: &ParsedLogLine) -> String {
    let tag = match p.kind {
        LogKind::Tool => "TOOL",
        LogKind::ToolResult => "RESULT",
        LogKind::Say => "SAY",
        LogKind::Heartbeat => "HB",
        LogKind::Claimed => "CLAIM",
        LogKind::Closed => "CLOSE",
        LogKind::Session => "SESS",
        LogKind::Error => "ERR",
        LogKind::Status => "STAT",
        LogKind::Raw => "LOG",
    };
    match &p.timestamp {
        Some(ts) => format!("[{ts}] {tag:6} {}", p.body),
        None => format!("{tag:6} {}", p.body),
    }
}

/// Pretty multi-line tail for CLI `fleet logs`.
pub fn format_log_tail_pretty(seat: &str, max_lines: usize) -> String {
    let path = log_path(seat);
    let Ok(text) = fs::read_to_string(&path) else {
        return format!("No log for seat `{seat}` ({})", path.display());
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..]
        .iter()
        .map(|l| format_log_line_plain(&parse_log_line(l)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Byte-offset log follower for live tail.
#[derive(Debug, Clone)]
pub struct LogFollower {
    pub seat: String,
    pub offset: u64,
}

impl LogFollower {
    pub fn new(seat: &str) -> Self {
        let offset = fs::metadata(log_path(seat))
            .map(|m| m.len())
            .unwrap_or(0);
        Self {
            seat: seat.to_string(),
            offset,
        }
    }

    /// Start from the last `max_lines` of existing content (then follow).
    pub fn from_tail(seat: &str, max_lines: usize) -> (Self, Vec<ParsedLogLine>) {
        let path = log_path(seat);
        let text = fs::read_to_string(&path).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        let start = all.len().saturating_sub(max_lines);
        let initial: Vec<ParsedLogLine> = all[start..]
            .iter()
            .map(|l| parse_log_line(l))
            .collect();
        let offset = text.len() as u64;
        (
            Self {
                seat: seat.to_string(),
                offset,
            },
            initial,
        )
    }

    /// Read newly appended bytes; update offset. Returns parsed lines.
    pub fn poll(&mut self) -> Vec<ParsedLogLine> {
        let path = log_path(&self.seat);
        let Ok(meta) = fs::metadata(&path) else {
            return Vec::new();
        };
        let len = meta.len();
        if len < self.offset {
            // Truncated / rotated.
            self.offset = 0;
        }
        if len == self.offset {
            return Vec::new();
        }
        let Ok(mut f) = fs::File::open(&path) else {
            return Vec::new();
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_err() {
            return Vec::new();
        }
        self.offset = len;
        // Incomplete trailing line: hold back until newline.
        if !buf.ends_with('\n') {
            if let Some(last_nl) = buf.rfind('\n') {
                let incomplete = &buf[last_nl + 1..];
                self.offset = len - incomplete.len() as u64;
                buf.truncate(last_nl + 1);
            } else {
                self.offset = len - buf.len() as u64;
                return Vec::new();
            }
        }
        buf.lines()
            .filter(|l| !l.is_empty())
            .map(parse_log_line)
            .collect()
    }
}

/// Legacy aggregate path (still written for `/worker` compatibility).
pub fn legacy_worker_status_path() -> PathBuf {
    project_rs_agent().join("worker-status.json")
}

/// How long a seat may stay paused without a resume before auto-resume (safety).
pub const PAUSE_TTL_SECS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SeatStatus {
    pub seat: String,
    pub pid: u32,
    pub updated_at: String,
    pub heartbeat_at: String,
    /// idle | claiming | working | sleeping | paused | attached | stopped | error
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
    /// Why the seat is paused / who attached (TUI takeover).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_reason: Option<String>,
    /// Unix secs when pause began (for TTL auto-resume).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<i64>,
    /// Herdr-style lifecycle (blocked/working/idle/done).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    /// Headless human-wait (Conductor HUMAN).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awaiting_human: Option<bool>,
    /// Optional schema/prompt for human resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_prompt: Option<String>,
}

/// Control-plane ops from TUI → worker (append-only jsonl).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlOp {
    Pause,
    Resume,
    Abort,
    Steer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlCommand {
    pub id: String,
    pub op: ControlOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub ts: String,
}

/// Parsed fleet log line for pretty rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogKind {
    Tool,
    ToolResult,
    Say,
    Heartbeat,
    Claimed,
    Closed,
    Session,
    Error,
    Status,
    Raw,
}

#[derive(Debug, Clone)]
pub struct ParsedLogLine {
    pub timestamp: Option<String>,
    pub kind: LogKind,
    pub body: String,
    pub raw: String,
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
        // Infer caste from name (Fleet-* → fleet, Crew-* → crew); default fleet.
        let mut caste = crate::agent::SeatCaste::infer_from_name(seat);
        if caste == crate::agent::SeatCaste::Any {
            caste = crate::agent::SeatCaste::Fleet;
        }
        match crate::agent::seat::ensure_with_caste(seat, caste) {
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

/// Delete a seat from the city: stop process, remove fleet files, drop seat profile.
pub fn delete_seat(seat: &str) -> String {
    let mut out = String::new();
    out.push_str(&fleet_down(Some(vec![seat.to_string()])));
    let slug = seat_slug(seat);
    let dir = fleet_dir();
    let mut removed = Vec::new();
    for suffix in [
        ".status.json",
        ".log",
        ".pid",
        ".control.jsonl",
        ".control.offset",
        ".spawn.log",
    ] {
        let path = dir.join(format!("{slug}{suffix}"));
        if path.exists() {
            match fs::remove_file(&path) {
                Ok(()) => removed.push(suffix.trim_start_matches('.').to_string()),
                Err(e) => out.push_str(&format!("  WARN remove {}: {e}\n", path.display())),
            }
        }
    }
    match crate::agent::seat::delete(seat) {
        Ok(()) => out.push_str(&format!("Deleted seat `{seat}` (profile + {:?})\n", removed)),
        Err(e) => out.push_str(&format!("Deleted fleet files for `{seat}`; profile: {e}\n")),
    }
    out
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
        format_log_tail_pretty(seat, lines),
        {
            let p = fleet_dir().join(format!("{}.spawn.log", seat_slug(seat)));
            fs::read_to_string(p)
                .unwrap_or_else(|_| "(no spawn log)".into())
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
    format_city_board(None)
}

/// One row on the city seat board.
#[derive(Debug, Clone)]
pub struct SeatRow {
    pub seat: String,
    pub caste: String,
    pub state: String,
    pub alive: String,
    pub pid: u32,
    pub heartbeat_at: String,
    pub bead_id: String,
    pub bead_title: String,
    pub tool: String,
    pub line: String,
    pub session_id: String,
    pub model: String,
    pub beads_closed: u32,
    pub beads_blocked: u32,
    pub paused_reason: Option<String>,
    pub last_error: Option<String>,
    pub running: bool,
}

impl SeatRow {
    pub fn from_status(s: &SeatStatus) -> Self {
        let managed = read_pid(&s.seat);
        let alive = match managed.or(Some(s.pid)) {
            Some(pid) if pid_alive(pid) => "alive",
            Some(_) if s.running => "stale-pid?",
            _ if s.running => "stale?",
            _ => "stopped",
        }
        .to_string();
        let caste = crate::agent::seat::resolve_caste(&s.seat).as_str().to_string();
        Self {
            seat: s.seat.clone(),
            caste,
            state: s.state.clone(),
            alive,
            pid: s.pid,
            heartbeat_at: s.heartbeat_at.clone(),
            bead_id: s.last_bead.clone().unwrap_or_else(|| "-".into()),
            bead_title: s
                .last_title
                .as_deref()
                .unwrap_or("-")
                .chars()
                .take(48)
                .collect(),
            tool: s.last_tool.clone().unwrap_or_else(|| "-".into()),
            line: s
                .last_line
                .as_deref()
                .unwrap_or("-")
                .chars()
                .take(72)
                .collect(),
            session_id: s.session_id.clone().unwrap_or_else(|| "-".into()),
            model: s.model.clone().unwrap_or_else(|| "-".into()),
            beads_closed: s.beads_closed,
            beads_blocked: s.beads_blocked,
            paused_reason: s.paused_reason.clone(),
            last_error: s.last_error.clone().filter(|e| !e.is_empty()),
            running: s.running,
        }
    }

    /// Compact one-line board row.
    pub fn format_board_line(&self) -> String {
        let mark = match self.state.as_str() {
            "working" => "●",
            "paused" | "attached" => "◐",
            "sleeping" | "idle" => "○",
            "error" => "✗",
            "stopped" => "·",
            _ => "•",
        };
        let mut line = format!(
            "{mark} {:12} [{:<8}] {:>5} {:8}  {:4} {:40}  tool:{:<12}  {}",
            self.seat,
            self.state,
            self.alive,
            self.caste,
            self.bead_id,
            self.bead_title,
            self.tool,
            self.heartbeat_at
        );
        if self.state == "paused" || self.state == "attached" {
            line.push_str(&format!(
                "  ({})",
                self.paused_reason.as_deref().unwrap_or("attach")
            ));
        }
        line
    }
}

/// Collect board rows (status files + pid-only seats).
pub fn collect_seat_rows() -> Vec<SeatRow> {
    let mut rows: Vec<SeatRow> = list_seat_statuses()
        .iter()
        .map(SeatRow::from_status)
        .collect();
    // Pid files without status yet.
    if let Ok(entries) = fs::read_dir(fleet_dir()) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(slug) = name.strip_suffix(".pid") {
                let known = rows.iter().any(|r| seat_slug(&r.seat) == slug);
                if known {
                    continue;
                }
                if let Some(pid) = read_pid(slug) {
                    rows.push(SeatRow {
                        seat: slug.to_string(),
                        caste: crate::agent::seat::resolve_caste(slug).as_str().to_string(),
                        state: "starting".into(),
                        alive: if pid_alive(pid) {
                            "alive".into()
                        } else {
                            "dead".into()
                        },
                        pid,
                        heartbeat_at: "-".into(),
                        bead_id: "-".into(),
                        bead_title: "(no status yet)".into(),
                        tool: "-".into(),
                        line: "-".into(),
                        session_id: "-".into(),
                        model: "-".into(),
                        beads_closed: 0,
                        beads_blocked: 0,
                        paused_reason: None,
                        last_error: None,
                        running: pid_alive(pid),
                    });
                }
            }
        }
    }
    rows.sort_by(|a, b| a.seat.cmp(&b.seat));
    rows
}

/// City cockpit board. `highlight` marks the followed/attached seat.
pub fn format_city_board(highlight: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("── City ──────────────────────────────────────────────────────\n");
    out.push_str(&crate::beads::format_backlog_idle_message());
    out.push('\n');

    let rows = collect_seat_rows();
    if rows.is_empty() {
        out.push_str("No seats yet.\n");
        out.push_str("Start: rs-agent -a fleet up --seats Fleet-1,Fleet-2\n");
        out.push_str("Ops:   /seat follow|attach|steer|abort|open|detach <seat>\n");
    } else {
        out.push_str("Seats:\n");
        for r in &rows {
            let sel = highlight
                .map(|h| seat_slug(h) == seat_slug(&r.seat))
                .unwrap_or(false);
            let prefix = if sel { "→ " } else { "  " };
            out.push_str(prefix);
            out.push_str(&r.format_board_line());
            out.push('\n');
            if sel || r.state == "working" || r.state == "paused" || r.state == "attached" {
                out.push_str(&format!(
                    "     session={}  model={}  closed={} blocked={}\n",
                    r.session_id, r.model, r.beads_closed, r.beads_blocked
                ));
                if !r.line.is_empty() && r.line != "-" {
                    out.push_str(&format!("     line: {}\n", r.line));
                }
            }
            if let Some(err) = &r.last_error {
                out.push_str(&format!(
                    "     error: {}\n",
                    err.chars().take(120).collect::<String>()
                ));
            }
        }
        out.push_str(
            "\nCommands: /city | /seat follow|attach|detach|steer|abort|open <seat>\n",
        );
    }

    out.push('\n');
    out.push_str(&crate::beads::format_fleet_status(None));
    out
}

/// Detail card for one seat (inspect / open).
pub fn format_seat_card(seat: &str) -> String {
    let Some(s) = read_seat_status(seat) else {
        return format!("No status for `{seat}`. Is a worker running?");
    };
    let row = SeatRow::from_status(&s);
    let mut out = format!(
        "Seat `{}`  caste={}  state={}  {}\n\
         pid={}  hb={}  model={}\n\
         bead: {} — {}\n\
         tool: {}\n\
         session: {}\n\
         closed={} blocked={}\n",
        row.seat,
        row.caste,
        row.state,
        row.alive,
        row.pid,
        row.heartbeat_at,
        row.model,
        row.bead_id,
        row.bead_title,
        row.tool,
        row.session_id,
        row.beads_closed,
        row.beads_blocked,
    );
    if let Some(r) = &row.paused_reason {
        out.push_str(&format!("pause: {r}\n"));
    }
    if let Some(e) = &row.last_error {
        out.push_str(&format!("error: {e}\n"));
    }
    out.push_str(&format!("log: {}\n", log_path(seat).display()));
    out.push_str("\n--- log (pretty) ---\n");
    out.push_str(&format_log_tail_pretty(seat, 25));
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

    #[test]
    fn control_mailbox_roundtrip() {
        crate::with_temp_cwd(|_| {
            append_control("Fleet-1", ControlOp::Pause, Some("tui"));
            append_control("Fleet-1", ControlOp::Steer, Some("use edit"));
            let cmds = poll_control("Fleet-1");
            assert_eq!(cmds.len(), 2);
            assert_eq!(cmds[0].op, ControlOp::Pause);
            assert_eq!(cmds[1].op, ControlOp::Steer);
            assert!(poll_control("Fleet-1").is_empty());
        });
    }

    #[test]
    fn parse_log_kinds() {
        let p = parse_log_line("[2026-01-01 12:00:00] → tool bash");
        assert_eq!(p.kind, LogKind::Tool);
        let p = parse_log_line("[t] say: hello world");
        assert_eq!(p.kind, LogKind::Say);
        let p = parse_log_line("[t] claimed b1 — title");
        assert_eq!(p.kind, LogKind::Claimed);
    }

    #[test]
    fn log_follower_polls_new_lines() {
        crate::with_temp_cwd(|_| {
            append_log("Fleet-1", "→ tool ls");
            let (mut f, initial) = LogFollower::from_tail("Fleet-1", 10);
            assert!(!initial.is_empty());
            assert!(f.poll().is_empty());
            append_log("Fleet-1", "say: next");
            let next = f.poll();
            assert_eq!(next.len(), 1);
            assert_eq!(next[0].kind, LogKind::Say);
        });
    }

    #[test]
    fn city_board_lists_seat() {
        crate::with_temp_cwd(|_| {
            let mut s = new_working_status("Fleet-1", Some("m"));
            s.state = "working".into();
            s.last_bead = Some("b1".into());
            s.last_title = Some("do thing".into());
            write_seat_status(&s);
            let board = format_city_board(Some("Fleet-1"));
            assert!(board.contains("Fleet-1"), "{board}");
            assert!(board.contains("→ "), "{board}");
            assert!(board.contains("b1"), "{board}");
        });
    }
}
