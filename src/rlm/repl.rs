//! Persistent Python REPL with host-mediated llm_query / agent_query.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Minimal JSON-lines REPL. Host calls are emitted as stdout lines during exec.
const REPL_BOOTSTRAP: &str = r#"
import sys, json, traceback, os, io

_NS = {"__builtins__": __builtins__, "os": os, "json": json, "context": ""}
_REAL_OUT = sys.stdout

def _host(method, *args, **kwargs):
    req = {"type": "host_call", "method": method, "args": list(args), "kwargs": kwargs}
    _REAL_OUT.write(json.dumps(req) + "\n")
    _REAL_OUT.flush()
    line = sys.stdin.readline()
    if not line:
        raise RuntimeError("host closed")
    resp = json.loads(line)
    if resp.get("error"):
        raise RuntimeError(str(resp["error"]))
    return resp.get("result")

def llm_query(prompt):
    return _host("llm_query", str(prompt))

def llm_query_batched(prompts):
    return _host("llm_query_batched", list(prompts))

def agent_query(task, tools=None):
    return _host("agent_query", str(task), tools=tools)

def load_file(path):
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        return f.read()

def load_dir(path, max_files=50, max_bytes=200000):
    out = {}
    total = 0
    for root, _dirs, files in os.walk(path):
        for name in files:
            if name.startswith("."):
                continue
            fp = os.path.join(root, name)
            try:
                data = open(fp, "r", encoding="utf-8", errors="replace").read()
            except Exception:
                continue
            if total + len(data) > max_bytes:
                out[fp] = data[:max(0, max_bytes - total)] + "\n...[truncated]"
                return out
            out[fp] = data
            total += len(data)
            if len(out) >= max_files:
                return out
    return out

def FINAL(value):
    _NS["_FINAL"] = value
    return value

_NS.update({
    "llm_query": llm_query,
    "llm_query_batched": llm_query_batched,
    "agent_query": agent_query,
    "load_file": load_file,
    "load_dir": load_dir,
    "FINAL": FINAL,
})

def handle_exec(code):
    stdout_cap = io.StringIO()
    stderr_cap = io.StringIO()
    class StreamOut:
        def __init__(self, stream_name, cap):
            self.stream_name = stream_name
            self.cap = cap
            self._buf = ""
        def write(self, s):
            if not s:
                return 0
            t = s.lstrip()
            if self.stream_name == "stdout" and (
                t.startswith('{"type": "host_call"') or t.startswith('{"type":"host_call"')
            ):
                _REAL_OUT.write(s)
                _REAL_OUT.flush()
                return len(s)
            self.cap.write(s)
            self._buf += s
            while "\n" in self._buf:
                line, self._buf = self._buf.split("\n", 1)
                evt = {"type": self.stream_name, "text": line + "\n"}
                _REAL_OUT.write(json.dumps(evt) + "\n")
                _REAL_OUT.flush()
            return len(s)
        def flush(self):
            if self._buf:
                evt = {"type": self.stream_name, "text": self._buf}
                _REAL_OUT.write(json.dumps(evt) + "\n")
                _REAL_OUT.flush()
                self._buf = ""
            _REAL_OUT.flush()
    old_out, old_err = sys.stdout, sys.stderr
    sys.stdout, sys.stderr = StreamOut("stdout", stdout_cap), StreamOut("stderr", stderr_cap)
    ok = True
    err = ""
    try:
        exec(compile(code, "<repl>", "exec"), _NS, _NS)
    except Exception:
        ok = False
        err = traceback.format_exc()
    finally:
        try:
            sys.stdout.flush()
            sys.stderr.flush()
        except Exception:
            pass
        sys.stdout, sys.stderr = old_out, old_err
    return {
        "type": "result",
        "ok": ok,
        "stdout": stdout_cap.getvalue()[-8000:],
        "stderr": (err or stderr_cap.getvalue())[-8000:],
        "final": _NS.get("_FINAL"),
    }

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception as e:
        _REAL_OUT.write(json.dumps({"type": "result", "ok": False, "error": str(e)}) + "\n")
        _REAL_OUT.flush()
        continue
    typ = msg.get("type")
    if typ == "exec":
        _NS.pop("_FINAL", None)
        out = handle_exec(msg.get("code", ""))
        _REAL_OUT.write(json.dumps(out, default=str) + "\n")
        _REAL_OUT.flush()
    elif typ == "set_context":
        _NS["context"] = msg.get("context", "")
        _REAL_OUT.write(json.dumps({"type": "result", "ok": True}) + "\n")
        _REAL_OUT.flush()
    elif typ == "ping":
        _REAL_OUT.write(json.dumps({"type": "result", "ok": True, "pong": True}) + "\n")
        _REAL_OUT.flush()
    elif typ == "shutdown":
        break
    else:
        _REAL_OUT.write(json.dumps({"type": "result", "ok": False, "error": "unknown type"}) + "\n")
        _REAL_OUT.flush()
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplExecResult {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub final_value: Option<Value>,
}

pub struct ReplSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    write_lock: Mutex<()>,
}

/// Error message shown whenever `python3` can't be found on `PATH`.
pub const PYTHON3_NOT_FOUND: &str =
    "python3 not found on PATH. Install Python 3 and ensure `python3` works.";

/// Quick, non-fatal check for whether `python3` is available on `PATH`.
/// Used for a startup warning banner; the REPL itself still surfaces
/// [`PYTHON3_NOT_FOUND`] if spawning actually fails at the point of use.
pub fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

impl ReplSession {
    pub async fn spawn() -> Result<Self, String> {
        let mut child = Command::new("python3")
            .arg("-u")
            .arg("-c")
            .arg(REPL_BOOTSTRAP)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    PYTHON3_NOT_FOUND.to_string()
                } else {
                    format!("failed to spawn python3 REPL: {}", e)
                }
            })?;

        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;

        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            write_lock: Mutex::new(()),
        };

        let _ = session
            .roundtrip(&serde_json::json!({"type": "ping"}))
            .await?;
        Ok(session)
    }

    pub async fn set_context(&mut self, context: &str) -> Result<(), String> {
        let resp = self
            .roundtrip(&serde_json::json!({"type": "set_context", "context": context}))
            .await?;
        if resp.get("ok") == Some(&Value::Bool(true)) {
            Ok(())
        } else {
            Err(resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("set_context failed")
                .to_string())
        }
    }

    pub async fn exec_with_host<F, Fut>(
        &mut self,
        code: &str,
        host_handler: F,
    ) -> Result<ReplExecResult, String>
    where
        F: FnMut(String, Vec<Value>, Value) -> Fut,
        Fut: std::future::Future<Output = Result<Value, String>>,
    {
        self.exec_with_host_and_output(code, host_handler, |_, _| {}).await
    }

    /// Like [`exec_with_host`], but also forwards live `stdout`/`stderr` JSON
    /// lines to `on_output(stream, text)` as they arrive.
    pub async fn exec_with_host_and_output<F, Fut, O>(
        &mut self,
        code: &str,
        mut host_handler: F,
        mut on_output: O,
    ) -> Result<ReplExecResult, String>
    where
        F: FnMut(String, Vec<Value>, Value) -> Fut,
        Fut: std::future::Future<Output = Result<Value, String>>,
        O: FnMut(&str, &str),
    {
        let _guard = self.write_lock.lock().await;
        let msg = serde_json::json!({"type": "exec", "code": code});
        let line = serde_json::to_string(&msg).map_err(|e| e.to_string())? + "\n";
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        self.stdin.flush().await.map_err(|e| e.to_string())?;

        loop {
            let mut buf = String::new();
            let n = self
                .stdout
                .read_line(&mut buf)
                .await
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("REPL exited unexpectedly".to_string());
            }
            let v: Value = serde_json::from_str(buf.trim())
                .map_err(|e| format!("bad REPL line: {} ({})", e, buf.trim()))?;
            match v.get("type").and_then(|t| t.as_str()) {
                Some("host_call") => {
                    let method = v
                        .get("method")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = v
                        .get("args")
                        .and_then(|a| a.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let kwargs = v.get("kwargs").cloned().unwrap_or(serde_json::json!({}));
                    let reply = match host_handler(method, args, kwargs).await {
                        Ok(result) => serde_json::json!({"result": result}),
                        Err(error) => serde_json::json!({"error": error}),
                    };
                    let reply_line = serde_json::to_string(&reply).unwrap_or_default() + "\n";
                    self.stdin
                        .write_all(reply_line.as_bytes())
                        .await
                        .map_err(|e| e.to_string())?;
                    self.stdin.flush().await.map_err(|e| e.to_string())?;
                }
                Some("stdout") | Some("stderr") => {
                    let stream = v.get("type").and_then(|t| t.as_str()).unwrap_or("stdout");
                    let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    on_output(stream, text);
                }
                Some("result") => {
                    return Ok(ReplExecResult {
                        ok: v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false),
                        stdout: v
                            .get("stdout")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                        stderr: v
                            .get("stderr")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                        final_value: v.get("final").cloned().filter(|x| !x.is_null()),
                    });
                }
                _ => {}
            }
        }
    }

    async fn roundtrip(&mut self, msg: &Value) -> Result<Value, String> {
        let _guard = self.write_lock.lock().await;
        let line = serde_json::to_string(msg).map_err(|e| e.to_string())? + "\n";
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        self.stdin.flush().await.map_err(|e| e.to_string())?;
        let mut buf = String::new();
        self.stdout
            .read_line(&mut buf)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::from_str(buf.trim()).map_err(|e| format!("bad reply: {} ({})", e, buf.trim()))
    }

    pub async fn shutdown(mut self) {
        let _ = self.stdin.write_all(b"{\"type\":\"shutdown\"}\n").await;
        let _ = self.child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python3_availability_matches_direct_check() {
        let expected = std::process::Command::new("python3")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert_eq!(python3_available(), expected);
    }

    #[test]
    fn python3_not_found_message_is_actionable() {
        assert!(PYTHON3_NOT_FOUND.contains("python3"));
        assert!(PYTHON3_NOT_FOUND.contains("PATH"));
        assert!(PYTHON3_NOT_FOUND.to_lowercase().contains("install"));
    }

    #[tokio::test]
    async fn spawn_reports_clear_error_when_python3_missing() {
        let orig_path = std::env::var_os("PATH");
        // SAFETY: no other test in this process depends on PATH concurrently
        // touching python3 spawn behavior; restored immediately after.
        std::env::set_var("PATH", "/definitely/does/not/exist");
        let result = ReplSession::spawn().await;
        if let Some(p) = orig_path {
            std::env::set_var("PATH", p);
        } else {
            std::env::remove_var("PATH");
        }
        match result {
            Err(e) => assert_eq!(e, PYTHON3_NOT_FOUND),
            Ok(_) => panic!("expected spawn to fail with empty PATH"),
        }
    }
}
