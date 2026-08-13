//! Thin headless runtime + JSON control socket (herdr-inspired).

pub mod socket;

pub use socket::{call, default_socket_path, handle_request, serve_socket, PROTOCOL_VERSION};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    pub protocol: u32,
}

impl ApiResponse {
    pub fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            ok: true,
            result: Some(result),
            error: None,
            id,
            protocol: PROTOCOL_VERSION,
        }
    }

    pub fn err(id: Option<serde_json::Value>, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            result: None,
            error: Some(error.into()),
            id,
            protocol: PROTOCOL_VERSION,
        }
    }
}

/// Write a pid file for the daemon.
pub fn write_pid(path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", std::process::id()))
}

pub fn pid_path() -> PathBuf {
    crate::config::Config::user_config_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("runtime.pid")
}

pub fn is_runtime_env() -> bool {
    std::env::var_os("RS_AGENT_RUNTIME").is_some() || std::env::var_os("RS_AGENT_SOCKET").is_some()
}
