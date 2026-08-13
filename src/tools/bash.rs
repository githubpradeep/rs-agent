use crate::agent::control::AbortFlag;
use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
pub struct BashArgs {
    pub command: String,
    pub timeout: Option<u64>,
    pub workdir: Option<String>,
}

pub struct BashTool {
    abort: AbortFlag,
}

impl BashTool {
    pub fn new(abort: AbortFlag) -> Self {
        Self { abort }
    }
}

/// Prefer absolute bash so a weird/empty PATH cannot make spawn fail with ENOENT.
fn resolve_bash() -> PathBuf {
    for candidate in ["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"] {
        let p = Path::new(candidate);
        if p.is_file() {
            return p.to_path_buf();
        }
    }
    PathBuf::from("bash")
}

/// Resolve and validate workdir. Returns Ok(None) to use process cwd.
fn resolve_workdir(workdir: Option<&str>) -> Result<Option<PathBuf>, String> {
    let Some(raw) = workdir.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let path = {
        let p = Path::new(raw);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    if !path.is_dir() {
        return Err(format!(
            "workdir does not exist: `{raw}` (resolved: {})\n\
             Omit workdir to use the current directory ({}), or pass a real path under this project.\n\
             Do not invent paths like /Users/james/... — use `.` or an existing relative path.",
            path.display(),
            cwd.display()
        ));
    }
    // Soft guard: absolute paths far outside cwd often mean a hallucinated machine.
    if Path::new(raw).is_absolute() {
        let cwd_canon = cwd.canonicalize().unwrap_or(cwd.clone());
        let path_canon = path.canonicalize().unwrap_or(path.clone());
        if !path_canon.starts_with(&cwd_canon) {
            return Err(format!(
                "workdir `{}` is outside the project cwd ({}).\n\
                 For safety, bash only accepts workdirs under the project. Omit workdir or use a relative path.",
                path_canon.display(),
                cwd_canon.display()
            ));
        }
    }
    Ok(Some(path))
}

fn looks_like_heredoc_file_write(cmd: &str) -> bool {
    let t = cmd.trim_start();
    let lower = t.to_lowercase();
    (lower.starts_with("cat >") || lower.starts_with("cat >>") || lower.contains("| tee "))
        && (lower.contains("<<") || lower.contains("echo ") && lower.contains(">"))
}

/// Detects commands that are likely to be destructive or otherwise dangerous
/// to run without explicit user awareness. Uses simple case-insensitive
/// substring matching rather than full shell parsing, so it may over-match
/// (e.g. `rm -rf /some/dir` also trips the `rm -rf /` heuristic); that's an
/// intentional tradeoff in favor of not missing genuinely dangerous commands.
///
/// Returns `Some(reason)` describing why the command is considered dangerous,
/// or `None` if no heuristic matched.
pub fn is_dangerous_command(cmd: &str) -> Option<&'static str> {
    let lower = cmd.to_lowercase();

    if lower.contains(":(){ :|:& };:") || lower.contains(":(){:|:&};:") {
        return Some("shell fork bomb");
    }
    if lower.contains("rm -rf /*") {
        return Some("recursive force-delete of root wildcard (rm -rf /*)");
    }
    if lower.contains("rm -rf ~") {
        return Some("recursive force-delete of home directory (rm -rf ~)");
    }
    if lower.contains("rm -rf /") {
        return Some("recursive force-delete of root filesystem (rm -rf /)");
    }
    if lower.contains("mkfs") {
        return Some("filesystem format command (mkfs)");
    }
    if lower.contains("dd if=") {
        return Some("low-level disk write command (dd if=)");
    }
    if (lower.contains("curl") || lower.contains("wget"))
        && (lower.contains("| sh")
            || lower.contains("|sh")
            || lower.contains("| bash")
            || lower.contains("|bash"))
    {
        return Some("piping a remote download directly into a shell");
    }
    if lower.contains("sudo ") {
        return Some("elevated privileges via sudo");
    }
    if lower.contains("shutdown") {
        return Some("system shutdown command");
    }
    if lower.contains("reboot") {
        return Some("system reboot command");
    }
    if lower.contains("diskutil erase") {
        return Some("disk erase command");
    }
    None
}

/// Soft interrupt (SIGINT to process group), brief grace, then KILL.
/// Kept short so Esc feels instant (~50–100ms), not half a second+.
async fn cancel_child(child: &mut tokio::process::Child, soft: bool) {
    let pid = child.id();
    #[cfg(unix)]
    if let Some(pid) = pid {
        let pgid = format!("-{pid}");
        if soft {
            let _ = tokio::process::Command::new("kill")
                .args(["-INT", &pgid])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            for _ in 0..5 {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        let _ = tokio::process::Command::new("kill")
            .args(["-KILL", &pgid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_millis(150), child.wait()).await;
}

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a shell command. Prefer specialized tools for file ops. \
         Long-running commands stream to the TUI; Esc cancels (SIGINT then kill)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to run" },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 30000). Values < 1000 are treated as seconds."
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory (must exist under project cwd)"
                }
            },
            "required": ["command"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let _ = tool_call_id;
        let parsed: BashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid args: {e}. Expected {{\"command\":\"...\"}} (optional timeout ms, workdir)."
                ))
            }
        };

        if parsed.command.trim().is_empty() {
            return ToolExecuteResult::error("command must not be empty");
        }

        if looks_like_heredoc_file_write(&parsed.command) {
            return ToolExecuteResult::error(
                "Do not create/overwrite files via bash heredoc/cat. \
                 Use the write tool with file_path + content, then bash only to run builds/tests.",
            );
        }

        let workdir = match resolve_workdir(parsed.workdir.as_deref()) {
            Ok(w) => w,
            Err(e) => return ToolExecuteResult::error(e),
        };

        // Values < 1000 are likely seconds mistakenly passed as ms
        let timeout_ms = match parsed.timeout {
            Some(t) if (1..1000).contains(&t) => t.saturating_mul(1000),
            Some(t) if t > 0 => t,
            _ => 30_000,
        };
        let timeout = Duration::from_millis(timeout_ms.min(3_600_000)); // cap 1h
        let danger = is_dangerous_command(&parsed.command);
        let warning_prefix = danger
            .map(|reason| format!("⚠️ DANGEROUS COMMAND DETECTED: {}\n\n", reason))
            .unwrap_or_default();

        let bash = resolve_bash();
        let mut cmd = tokio::process::Command::new(&bash);
        cmd.arg("-c").arg(&parsed.command);

        if let Some(dir) = &workdir {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        // Own process group so Esc can SIGINT/TERM the whole tree (herdr soft-cancel pattern).
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        cmd.kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Failed to spawn `{}`: {e}\n\
                     command: {}\n\
                     workdir: {}",
                    bash.display(),
                    parsed.command.chars().take(200).collect::<String>(),
                    workdir
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| {
                            std::env::current_dir()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| ".".into())
                        }),
                ))
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let out_task = tokio::spawn(async move {
            let mut buf = String::new();
            if let Some(stdout) = stdout {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    crate::tools::output_sink::emit_tool_output(
                        "bash",
                        "stdout",
                        &format!("{line}\n"),
                    );
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
            buf
        });
        let err_task = tokio::spawn(async move {
            let mut buf = String::new();
            if let Some(stderr) = stderr {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    crate::tools::output_sink::emit_tool_output(
                        "bash",
                        "stderr",
                        &format!("{line}\n"),
                    );
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
            buf
        });

        let finish = |warning_prefix: String, exit_code: i32, mut text: String| {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&format!("Exit code: {}", exit_code));
            // Prefer tail for errors (stack traces live at the end); head for success.
            let direction = if exit_code != 0 { "tail" } else { "head" };
            let capped = crate::tools::truncate_store::truncate_or_spill_with(
                &text,
                crate::tools::truncate_store::DEFAULT_MAX_LINES,
                crate::tools::truncate_store::DEFAULT_MAX_BYTES,
                direction,
            );
            let body = format!("{warning_prefix}{}", capped.content);
            if exit_code != 0 {
                ToolExecuteResult::error(body)
            } else {
                ToolExecuteResult::ok(body)
            }
        };

        let started = Instant::now();
        let abort = self.abort.clone();
        let result = tokio::select! {
            biased;
            _ = abort.wait() => {
                cancel_child(&mut child, true).await;
                out_task.abort();
                err_task.abort();
                ToolExecuteResult::error(format!(
                    "{warning_prefix}Command cancelled (Esc): {}",
                    parsed.command.chars().take(120).collect::<String>()
                ))
            }
            status = child.wait() => {
                match status {
                    Ok(status) => {
                        let stdout_text = out_task.await.unwrap_or_default();
                        let stderr_text = err_task.await.unwrap_or_default();
                        let mut text = stdout_text;
                        if !stderr_text.is_empty() {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&stderr_text);
                        }
                        finish(warning_prefix, status.code().unwrap_or(-1), text)
                    }
                    Err(e) => {
                        out_task.abort();
                        err_task.abort();
                        ToolExecuteResult::error(format!(
                            "{warning_prefix}Command failed: {e}"
                        ))
                    }
                }
            }
            _ = tokio::time::sleep(timeout.saturating_sub(started.elapsed())) => {
                cancel_child(&mut child, false).await;
                out_task.abort();
                err_task.abort();
                ToolExecuteResult::error(format!(
                    "{}Command timed out after {}ms: {}",
                    warning_prefix,
                    timeout.as_millis(),
                    parsed.command
                ))
            }
        };
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rm_rf_root() {
        assert!(is_dangerous_command("rm -rf /").is_some());
        assert!(is_dangerous_command("sudo rm -rf /").is_some());
        assert!(is_dangerous_command("RM -RF /").is_some());
    }

    #[test]
    fn detects_rm_rf_home() {
        assert!(is_dangerous_command("rm -rf ~").is_some());
        assert!(is_dangerous_command("rm -rf ~/Documents").is_some());
    }

    #[test]
    fn detects_rm_rf_root_wildcard() {
        assert!(is_dangerous_command("rm -rf /*").is_some());
    }

    #[test]
    fn detects_sudo() {
        assert!(is_dangerous_command("sudo apt install foo").is_some());
    }

    #[test]
    fn detects_shutdown_and_reboot() {
        assert!(is_dangerous_command("shutdown -h now").is_some());
        assert!(is_dangerous_command("reboot").is_some());
    }

    #[test]
    fn flags_heredoc_file_writes() {
        assert!(looks_like_heredoc_file_write("cat > foo <<EOF\nhi\nEOF"));
        assert!(looks_like_heredoc_file_write("cat >> bar <<'E'\nx\nE"));
        assert!(!looks_like_heredoc_file_write("echo hello"));
        assert!(!looks_like_heredoc_file_write("cat file.txt"));
    }

    #[test]
    fn rejects_missing_workdir() {
        let err = resolve_workdir(Some("/definitely/not/a/real/path/xyz")).unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[tokio::test]
    async fn abort_cancels_long_sleep() {
        let abort = AbortFlag::new();
        let tool = BashTool::new(abort.clone());
        let start = Instant::now();
        let handle = tokio::spawn(async move {
            tool.execute(
                "test",
                serde_json::json!({
                    "command": "sleep 30",
                    "timeout": 60000
                }),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        abort.abort();
        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("join timed out")
            .expect("task panicked");
        assert!(
            result.is_error,
            "expected cancelled error, got: {}",
            result.content
        );
        assert!(
            result.content.to_lowercase().contains("cancel"),
            "unexpected: {}",
            result.content
        );
        assert!(
            start.elapsed() < Duration::from_millis(800),
            "abort too slow: {:?}",
            start.elapsed()
        );
    }
}
