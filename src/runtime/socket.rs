//! Unix JSON-lines control socket.

use super::{ApiRequest, ApiResponse};
use crate::lifecycle::{self, Lifecycle};
use crate::notify::{self, NotifyMode};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 1;

pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("RS_AGENT_SOCKET") {
        return PathBuf::from(p);
    }
    crate::config::Config::user_config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("rs-agent.sock")
}

/// Prepare socket path: remove stale socket if no live peer.
pub fn prepare_socket_path(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        // Best-effort: if connect fails, reclaim.
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            if UnixStream::connect(path).is_err() {
                let _ = std::fs::remove_file(path);
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("socket already live: {}", path.display()),
                ));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(path);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn handle_request(req: &ApiRequest) -> ApiResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        "ping" => ApiResponse::ok(
            id,
            serde_json::json!({ "pong": true, "protocol": PROTOCOL_VERSION }),
        ),
        "session.snapshot" | "agent.status" => {
            let snap = lifecycle::snapshot();
            ApiResponse::ok(id, serde_json::to_value(snap).unwrap_or_default())
        }
        "agent.wait" => {
            let until = req
                .params
                .get("until")
                .and_then(|v| v.as_str())
                .map(Lifecycle::parse)
                .unwrap_or(Lifecycle::Blocked);
            let timeout_ms = req
                .params
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(30_000);
            match lifecycle::wait_until(until, Duration::from_millis(timeout_ms)) {
                Ok(snap) => ApiResponse::ok(id, serde_json::to_value(snap).unwrap_or_default()),
                Err(e) => ApiResponse::err(id, e),
            }
        }
        "notification.show" => {
            let title = req
                .params
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("rs-agent");
            let body = req.params.get("body").and_then(|v| v.as_str());
            let mode = req
                .params
                .get("mode")
                .and_then(|v| v.as_str())
                .map(NotifyMode::parse)
                .unwrap_or(NotifyMode::Terminal);
            match notify::show(mode, title, body) {
                Ok(sent) => ApiResponse::ok(id, serde_json::json!({ "sent": sent })),
                Err(e) => ApiResponse::err(id, e.to_string()),
            }
        }
        "config.reload" => {
            let _ = crate::config::Config::load();
            ApiResponse::ok(id, serde_json::json!({ "reloaded": true }))
        }
        "server.stop" => {
            // Client-side honor; server loop checks a flag file.
            let flag = default_socket_path().with_extension("stop");
            let _ = std::fs::write(&flag, "1\n");
            ApiResponse::ok(id, serde_json::json!({ "stopping": true }))
        }
        "agent.steer" | "seat.steer" => {
            let text = req
                .params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let seat = req.params.get("seat").and_then(|v| v.as_str());
            if let Some(seat) = seat {
                let _ =
                    crate::fleet::append_control(seat, crate::fleet::ControlOp::Steer, Some(text));
                ApiResponse::ok(id, serde_json::json!({ "steered": seat, "text": text }))
            } else {
                ApiResponse::err(id, "params.seat required for seat.steer")
            }
        }
        "seat.abort" => {
            let Some(seat) = req.params.get("seat").and_then(|v| v.as_str()) else {
                return ApiResponse::err(id, "params.seat required");
            };
            let _ = crate::fleet::append_control(seat, crate::fleet::ControlOp::Abort, None);
            ApiResponse::ok(id, serde_json::json!({ "aborted": seat }))
        }
        "seat.pause" => {
            let Some(seat) = req.params.get("seat").and_then(|v| v.as_str()) else {
                return ApiResponse::err(id, "params.seat required");
            };
            let _ = crate::fleet::append_control(seat, crate::fleet::ControlOp::Pause, None);
            ApiResponse::ok(id, serde_json::json!({ "paused": seat }))
        }
        "seat.resume" => {
            let Some(seat) = req.params.get("seat").and_then(|v| v.as_str()) else {
                return ApiResponse::err(id, "params.seat required");
            };
            let _ = crate::fleet::append_control(seat, crate::fleet::ControlOp::Resume, None);
            ApiResponse::ok(id, serde_json::json!({ "resumed": seat }))
        }
        "city.board" => ApiResponse::ok(
            id,
            crate::tui::fleet_panel::CityPanelState::board_snapshot_json(),
        ),
        "wish.create" => {
            let text = req
                .params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let as_task = req
                .params
                .get("as_task")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let auto_ready = req
                .params
                .get("auto_ready")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            match crate::wish::create_wish(text, as_task, auto_ready) {
                Ok(b) => ApiResponse::ok(
                    id,
                    serde_json::json!({
                        "id": b.id,
                        "title": b.title,
                        "kind": b.kind.as_str(),
                    }),
                ),
                Err(e) => ApiResponse::err(id, e),
            }
        }
        "fleet.up" => {
            let seats = if let Some(arr) = req.params.get("seats").and_then(|v| v.as_array()) {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            } else {
                let fleet_n = req
                    .params
                    .get("fleet_n")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2) as usize;
                let crew_n = req
                    .params
                    .get("crew_n")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let mut seats = Vec::new();
                for i in 1..=fleet_n.min(16) {
                    seats.push(format!("Fleet-{i}"));
                }
                for i in 1..=crew_n.min(8) {
                    seats.push(format!("Crew-{i}"));
                }
                seats
            };
            if seats.is_empty() {
                return ApiResponse::err(id, "fleet.up needs seats or fleet_n/crew_n");
            }
            let opts = crate::fleet::FleetUpOpts {
                seats,
                budget_minutes: 480,
                sleep_secs: 5,
                quiet: false,
                provider: req
                    .params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                model: req
                    .params
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                approve: true,
                fail_fast: false,
                shared_worktree: false,
            };
            match crate::fleet::fleet_up(opts) {
                Ok(msg) => ApiResponse::ok(id, serde_json::json!({ "report": msg })),
                Err(e) => ApiResponse::err(id, e),
            }
        }
        "fleet.down" => {
            let seats = req
                .params
                .get("seats")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                });
            let msg = crate::fleet::fleet_down(seats);
            ApiResponse::ok(id, serde_json::json!({ "report": msg }))
        }
        "fleet.delete" | "seat.delete" => {
            let Some(seat) = req.params.get("seat").and_then(|v| v.as_str()) else {
                return ApiResponse::err(id, "params.seat required");
            };
            let report = crate::fleet::delete_seat(seat);
            ApiResponse::ok(id, serde_json::json!({ "report": report }))
        }
        "bead.delete" | "wish.delete" => {
            let Some(bead_id) = req.params.get("id").and_then(|v| v.as_str()) else {
                return ApiResponse::err(id, "params.id required");
            };
            match crate::beads::delete(None, bead_id) {
                Ok(b) => ApiResponse::ok(
                    id,
                    serde_json::json!({ "id": b.id, "title": b.title, "deleted": true }),
                ),
                Err(e) => ApiResponse::err(id, e),
            }
        }
        other => ApiResponse::err(id, format!("unknown method: {other}")),
    }
}

/// Serve JSON-lines requests on a Unix socket until stop flag or fatal error.
#[cfg(unix)]
pub fn serve_socket(path: &Path) -> std::io::Result<()> {
    use std::os::unix::net::UnixListener;
    prepare_socket_path(path)?;
    let listener = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    std::env::set_var("RS_AGENT_RUNTIME", "1");
    std::env::set_var("RS_AGENT_SOCKET", path);
    crate::lifecycle::publish(Lifecycle::Idle, "runtime listening");
    let stop_flag = path.with_extension("stop");
    let _ = std::fs::remove_file(&stop_flag);

    listener.set_nonblocking(true)?;
    loop {
        if stop_flag.exists() {
            let _ = std::fs::remove_file(&stop_flag);
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                    let resp = match serde_json::from_str::<ApiRequest>(line.trim()) {
                        Ok(req) => handle_request(&req),
                        Err(e) => ApiResponse::err(None, format!("bad request: {e}")),
                    };
                    let mut stream = stream;
                    if let Ok(bytes) = serde_json::to_vec(&resp) {
                        let _ = stream.write_all(&bytes);
                        let _ = stream.write_all(b"\n");
                        let _ = stream.flush();
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[cfg(not(unix))]
pub fn serve_socket(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "runtime socket is Unix-only",
    ))
}

/// One-shot client call.
#[cfg(unix)]
pub fn call(path: &Path, req: &ApiRequest) -> std::io::Result<ApiResponse> {
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(path)?;
    let bytes = serde_json::to_vec(req)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(&bytes)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(line.trim())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(not(unix))]
pub fn call(_path: &Path, _req: &ApiRequest) -> std::io::Result<ApiResponse> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "runtime socket is Unix-only",
    ))
}
