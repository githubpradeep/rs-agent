//! Terminal / system notifications (herdr OSC9 / Kitty OSC99 pattern).

use std::io::{self, Write};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::lifecycle::{self, Lifecycle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyMode {
    Off,
    Terminal,
    System,
}

impl NotifyMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "terminal" | "osc" | "term" => Self::Terminal,
            "system" | "os" | "desktop" => Self::System,
            _ => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Terminal => "terminal",
            Self::System => "system",
        }
    }
}

struct RateLimit {
    last: Option<Instant>,
}

fn rate() -> &'static Mutex<RateLimit> {
    static R: OnceLock<Mutex<RateLimit>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(RateLimit { last: None }))
}

fn allow_notify() -> bool {
    let mut g = rate().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = g.last {
        if t.elapsed() < Duration::from_secs(2) {
            return false;
        }
    }
    g.last = Some(Instant::now());
    true
}

/// Notify on high-signal lifecycle transitions when not focused.
pub fn on_lifecycle(mode: NotifyMode, lifecycle: Lifecycle, detail: &str) {
    if mode == NotifyMode::Off {
        return;
    }
    if lifecycle::is_focused() {
        return;
    }
    if !matches!(lifecycle, Lifecycle::Blocked | Lifecycle::Done) {
        return;
    }
    if !allow_notify() {
        return;
    }
    let title = match lifecycle {
        Lifecycle::Blocked => "rs-agent · needs attention",
        Lifecycle::Done => "rs-agent · done",
        _ => "rs-agent",
    };
    let _ = show(mode, title, Some(detail));
}

pub fn show(mode: NotifyMode, title: &str, body: Option<&str>) -> io::Result<bool> {
    match mode {
        NotifyMode::Off => Ok(false),
        NotifyMode::Terminal => show_terminal(title, body),
        NotifyMode::System => {
            if show_system(title, body)? {
                Ok(true)
            } else {
                show_terminal(title, body)
            }
        }
    }
}

fn show_terminal(title: &str, body: Option<&str>) -> io::Result<bool> {
    let seq = if std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM").unwrap_or_default().contains("kitty")
    {
        build_osc99(title, body)
    } else {
        build_osc9(title, body)
    };
    let seq = if std::env::var_os("TMUX").is_some() {
        wrap_tmux(&seq)
    } else {
        seq
    };
    let mut out = io::stdout();
    out.write_all(&seq)?;
    out.flush()?;
    Ok(true)
}

fn show_system(title: &str, body: Option<&str>) -> io::Result<bool> {
    let body = body.unwrap_or("");
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape_applescript(body),
            escape_applescript(title)
        );
        let status = Command::new("osascript").arg("-e").arg(script).status()?;
        return Ok(status.success());
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("notify-send").arg(title).arg(body).status();
        return Ok(status.map(|s| s.success()).unwrap_or(false));
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (title, body);
        Ok(false)
    }
}

fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn sanitize(text: impl AsRef<str>) -> String {
    text.as_ref()
        .chars()
        .filter(|ch| *ch != '\u{1b}' && *ch != '\u{7}' && *ch != '\u{9c}')
        .map(|ch| match ch {
            '\n' | '\r' | '\t' => ' ',
            _ => ch,
        })
        .collect()
}

fn build_osc9(title: &str, body: Option<&str>) -> Vec<u8> {
    let message = match body {
        Some(b) if !b.is_empty() => sanitize(format!("{title}: {b}")),
        _ => sanitize(title),
    };
    format!("\x1b]9;{message}\x1b\\").into_bytes()
}

fn build_osc99(title: &str, body: Option<&str>) -> Vec<u8> {
    let title = sanitize(title);
    match body {
        Some(body) if !body.is_empty() => {
            let body = sanitize(body);
            format!("\x1b]99;i=1:d=0;{title}\x1b\\\x1b]99;i=1:p=body;{body}\x1b\\").into_bytes()
        }
        _ => format!("\x1b]99;;{title}\x1b\\").into_bytes(),
    }
}

fn wrap_tmux(sequence: &[u8]) -> Vec<u8> {
    let mut wrapped = Vec::with_capacity(sequence.len() + 16);
    wrapped.extend_from_slice(b"\x1bPtmux;");
    for &byte in sequence {
        if byte == 0x1b {
            wrapped.push(0x1b);
        }
        wrapped.push(byte);
    }
    wrapped.extend_from_slice(b"\x1b\\");
    wrapped
}
