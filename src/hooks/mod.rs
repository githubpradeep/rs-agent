//! Minimal hook system — load scripts from `.rs-agent/hooks/` and
//! `~/.rs-agent/hooks/`.
//!
//! Supported filenames (executable, or `.sh` / `.py`):
//! - `before_tool` — argv: tool_name, tool_input_json.
//!   Non-zero exit blocks the tool (stderr becomes the error).
//! - `after_tool` — argv: tool_name, is_error ("0"|"1"); stdin = tool result.
//! - `on_message` — argv: none; stdin = user message text.
//! - `before_goal_continue` — argv: none; stdin = goal condition.
//!   Non-zero exit pauses goal auto-continue.
//! - `on_goal_achieved` — argv: none; stdin = condition + reason (advisory).
//! - `before_handoff` — argv: none; stdin = handoff summary.
//!   Non-zero exit blocks the handoff tool.
//! - `before_bead_close` — argv: none; stdin = bead JSON (id, kind, title, …).
//!   Non-zero exit blocks closing the bead.
//!
//! Hooks are best-effort: missing dirs/scripts are ignored. Timeout: 5s.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub struct HookRegistry {
    before_tool: Option<PathBuf>,
    after_tool: Option<PathBuf>,
    on_message: Option<PathBuf>,
    before_goal_continue: Option<PathBuf>,
    on_goal_achieved: Option<PathBuf>,
    before_handoff: Option<PathBuf>,
    before_bead_close: Option<PathBuf>,
}

impl HookRegistry {
    /// Discover hooks from `~/.rs-agent/hooks/` then `./.rs-agent/hooks/`
    /// (project overrides home).
    pub fn load() -> Self {
        let mut reg = Self::default();
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        let dirs = [
            PathBuf::from(home).join(".rs-agent").join("hooks"),
            std::env::current_dir()
                .unwrap_or_default()
                .join(".rs-agent")
                .join("hooks"),
        ];
        for dir in &dirs {
            if let Some(p) = find_hook(dir, "before_tool") {
                reg.before_tool = Some(p);
            }
            if let Some(p) = find_hook(dir, "after_tool") {
                reg.after_tool = Some(p);
            }
            if let Some(p) = find_hook(dir, "on_message") {
                reg.on_message = Some(p);
            }
            if let Some(p) = find_hook(dir, "before_goal_continue") {
                reg.before_goal_continue = Some(p);
            }
            if let Some(p) = find_hook(dir, "on_goal_achieved") {
                reg.on_goal_achieved = Some(p);
            }
            if let Some(p) = find_hook(dir, "before_handoff") {
                reg.before_handoff = Some(p);
            }
            if let Some(p) = find_hook(dir, "before_bead_close") {
                reg.before_bead_close = Some(p);
            }
        }
        reg
    }

    pub fn has_any(&self) -> bool {
        self.before_tool.is_some()
            || self.after_tool.is_some()
            || self.on_message.is_some()
            || self.before_goal_continue.is_some()
            || self.on_goal_achieved.is_some()
            || self.before_handoff.is_some()
            || self.before_bead_close.is_some()
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(p) = &self.before_tool {
            parts.push(format!("before_tool={}", p.display()));
        }
        if let Some(p) = &self.after_tool {
            parts.push(format!("after_tool={}", p.display()));
        }
        if let Some(p) = &self.on_message {
            parts.push(format!("on_message={}", p.display()));
        }
        if let Some(p) = &self.before_goal_continue {
            parts.push(format!("before_goal_continue={}", p.display()));
        }
        if let Some(p) = &self.on_goal_achieved {
            parts.push(format!("on_goal_achieved={}", p.display()));
        }
        if let Some(p) = &self.before_handoff {
            parts.push(format!("before_handoff={}", p.display()));
        }
        if let Some(p) = &self.before_bead_close {
            parts.push(format!("before_bead_close={}", p.display()));
        }
        if parts.is_empty() {
            "no hooks loaded".into()
        } else {
            parts.join(", ")
        }
    }

    /// Returns `Err(message)` to block the tool call.
    pub fn before_tool(&self, tool_name: &str, tool_input: &str) -> Result<(), String> {
        let Some(script) = &self.before_tool else {
            return Ok(());
        };
        match run_hook(script, &[tool_name, tool_input], None) {
            HookResult::Ok => Ok(()),
            HookResult::Failed { code, stderr } => Err(format!(
                "before_tool hook blocked `{tool_name}` (exit {code}): {}",
                if stderr.trim().is_empty() {
                    "(no stderr)".into()
                } else {
                    stderr.trim().chars().take(500).collect::<String>()
                }
            )),
            HookResult::Error(e) => {
                tracing::warn!(error = %e, "before_tool hook error (allowing tool)");
                Ok(())
            }
        }
    }

    pub fn after_tool(&self, tool_name: &str, is_error: bool, result: &str) {
        let Some(script) = &self.after_tool else {
            return;
        };
        let flag = if is_error { "1" } else { "0" };
        let _ = run_hook(script, &[tool_name, flag], Some(result));
    }

    pub fn on_message(&self, text: &str) {
        let Some(script) = &self.on_message else {
            return;
        };
        let _ = run_hook(script, &[], Some(text));
    }

    /// Returns `Err(message)` to pause goal auto-continue.
    pub fn before_goal_continue(&self, condition: &str) -> Result<(), String> {
        let Some(script) = &self.before_goal_continue else {
            return Ok(());
        };
        match run_hook(script, &[], Some(condition)) {
            HookResult::Ok => Ok(()),
            HookResult::Failed { code, stderr } => Err(format!(
                "before_goal_continue hook paused goal (exit {code}): {}",
                if stderr.trim().is_empty() {
                    "(no stderr)".into()
                } else {
                    stderr.trim().chars().take(500).collect::<String>()
                }
            )),
            HookResult::Error(e) => {
                tracing::warn!(error = %e, "before_goal_continue hook error (allowing continue)");
                Ok(())
            }
        }
    }

    pub fn on_goal_achieved(&self, condition: &str, reason: &str) {
        let Some(script) = &self.on_goal_achieved else {
            return;
        };
        let body = format!("{condition}\n---\n{reason}");
        let _ = run_hook(script, &[], Some(&body));
    }

    /// Returns `Err(message)` to block the handoff tool.
    pub fn before_handoff(&self, summary: &str) -> Result<(), String> {
        let Some(script) = &self.before_handoff else {
            return Ok(());
        };
        match run_hook(script, &[], Some(summary)) {
            HookResult::Ok => Ok(()),
            HookResult::Failed { code, stderr } => Err(format!(
                "before_handoff hook blocked handoff (exit {code}): {}",
                if stderr.trim().is_empty() {
                    "(no stderr)".into()
                } else {
                    stderr.trim().chars().take(500).collect::<String>()
                }
            )),
            HookResult::Error(e) => {
                tracing::warn!(error = %e, "before_handoff hook error (allowing handoff)");
                Ok(())
            }
        }
    }

    /// Returns `Err(message)` to block bead close.
    pub fn before_bead_close(&self, bead_json: &str) -> Result<(), String> {
        let Some(script) = &self.before_bead_close else {
            return Ok(());
        };
        match run_hook(script, &[], Some(bead_json)) {
            HookResult::Ok => Ok(()),
            HookResult::Failed { code, stderr } => Err(format!(
                "before_bead_close hook blocked close (exit {code}): {}",
                if stderr.trim().is_empty() {
                    "(no stderr)".into()
                } else {
                    stderr.trim().chars().take(500).collect::<String>()
                }
            )),
            HookResult::Error(e) => {
                tracing::warn!(error = %e, "before_bead_close hook error (allowing close)");
                Ok(())
            }
        }
    }
}

fn find_hook(dir: &Path, name: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    for candidate in [
        dir.join(name),
        dir.join(format!("{name}.sh")),
        dir.join(format!("{name}.py")),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[derive(Debug)]
enum HookResult {
    Ok,
    Failed { code: i32, stderr: String },
    Error(String),
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

fn run_hook(script: &Path, args: &[&str], stdin_data: Option<&str>) -> HookResult {
    let mut cmd = if script.extension().and_then(|e| e.to_str()) == Some("py") {
        let mut c = Command::new("python3");
        c.arg(script);
        c
    } else if script.extension().and_then(|e| e.to_str()) == Some("sh") || !is_executable(script) {
        let mut c = Command::new("bash");
        c.arg(script);
        c
    } else {
        Command::new(script)
    };
    cmd.args(args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return HookResult::Error(format!("spawn {}: {e}", script.display())),
    };

    if let Some(data) = stdin_data {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data.as_bytes());
        }
    } else {
        drop(child.stdin.take());
    }

    let script_disp = script.display().to_string();
    let handle = std::thread::spawn(move || child.wait_with_output());

    let start = std::time::Instant::now();
    loop {
        if handle.is_finished() {
            break;
        }
        if start.elapsed() > TIMEOUT {
            return HookResult::Error(format!(
                "hook {script_disp} timed out after {}s",
                TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    match handle.join() {
        Ok(Ok(output)) => {
            if output.status.success() {
                HookResult::Ok
            } else {
                HookResult::Failed {
                    code: output.status.code().unwrap_or(-1),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                }
            }
        }
        Ok(Err(e)) => HookResult::Error(format!("wait: {e}")),
        Err(_) => HookResult::Error("hook thread panicked".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_does_not_panic() {
        let reg = HookRegistry::load();
        let _ = reg.summary();
        let _ = reg.has_any();
    }

    #[test]
    #[cfg(unix)]
    fn before_tool_blocks_on_nonzero() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("before_tool.sh");
        std::fs::write(&script, "#!/bin/bash\necho blocked >&2\nexit 2\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        let reg = HookRegistry {
            before_tool: Some(script),
            ..Default::default()
        };
        let err = reg.before_tool("bash", "{}").unwrap_err();
        assert!(err.contains("blocked") || err.contains("exit 2"));
    }

    #[test]
    #[cfg(unix)]
    fn before_tool_allows_on_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("before_tool.sh");
        std::fs::write(&script, "#!/bin/bash\nexit 0\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        let reg = HookRegistry {
            before_tool: Some(script),
            ..Default::default()
        };
        assert!(reg.before_tool("read", "{}").is_ok());
    }
}
