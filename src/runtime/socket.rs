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
        "agent.steer" => {
            let text = req
                .params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let seat = req.params.get("seat").and_then(|v| v.as_str());
            if let Some(seat) = seat {
                let _ = crate::fleet::append_control(seat, crate::fleet::ControlOp::Steer, Some(text));
                ApiResponse::ok(id, serde_json::json!({ "steered": seat, "text": text }))
            } else {
                ApiResponse::err(id, "params.seat required for agent.steer")
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
