//! Post-mutation formatter + LSP diagnostics appended to tool results.

use crate::lsp::{Diagnostic, DiagnosticSnapshot, SharedDiagnostics};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

static BRIDGE: OnceLock<DiagnosticsBridge> = OnceLock::new();

/// Process-wide bridge so mutating tools can read LSP state without going through the TUI.
#[derive(Clone, Default)]
pub struct DiagnosticsBridge {
    inner: Arc<Mutex<BridgeInner>>,
}

#[derive(Default)]
struct BridgeInner {
    /// Latest snapshot from the LSP client (updated by TUI/LSP task).
    snapshot: DiagnosticSnapshot,
    /// Per-path error fingerprints before the current mutation (delta ledger).
    baseline: HashMap<String, Vec<String>>,
}

impl DiagnosticsBridge {
    pub fn global() -> &'static DiagnosticsBridge {
        BRIDGE.get_or_init(DiagnosticsBridge::default)
    }

    pub fn set_snapshot(&self, snap: DiagnosticSnapshot) {
        if let Ok(mut g) = self.inner.lock() {
            g.snapshot = snap;
        }
    }

    pub fn snapshot(&self) -> DiagnosticSnapshot {
        self.inner
            .lock()
            .map(|g| g.snapshot.clone())
            .unwrap_or_default()
    }

    pub fn capture_baseline(&self, path: &str) {
        let key = normalize_path_key(path);
        let snap = self.snapshot();
        let fps = fingerprints_for(&snap, &key);
        if let Ok(mut g) = self.inner.lock() {
            g.baseline.insert(key, fps);
        }
    }

    /// New error/warning diagnostics since baseline for this path.
    pub fn delta_report(&self, path: &str, limit: usize) -> Option<String> {
        let key = normalize_path_key(path);
        let snap = self.snapshot();
        let baseline = self
            .inner
            .lock()
            .ok()
            .and_then(|g| g.baseline.get(&key).cloned())
            .unwrap_or_default();
        let current = diags_for(&snap, &key);
        let fresh: Vec<&Diagnostic> = current
            .into_iter()
            .filter(|d| d.severity <= 2) // errors + warnings
            .filter(|d| !baseline.contains(&diag_fp(d)))
            .collect();
        if fresh.is_empty() {
            return None;
        }
        let mut lines = Vec::new();
        for (i, d) in fresh.iter().enumerate() {
            if i >= limit {
                lines.push(format!("… ({} more)", fresh.len().saturating_sub(limit)));
                break;
            }
            let sev = match d.severity {
                1 => "error",
                2 => "warn",
                _ => "info",
            };
            lines.push(format!(
                "{sev} [{}:{}] {}",
                d.line + 1,
                d.character + 1,
                d.message
            ));
        }
        Some(format!(
            "<diagnostics file=\"{}\">\n{}\n</diagnostics>",
            path,
            lines.join("\n")
        ))
    }
}

fn normalize_path_key(path: &str) -> String {
    let p = Path::new(path);
    p.canonicalize()
        .unwrap_or_else(|_| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(p)
            }
        })
        .to_string_lossy()
        .to_string()
}

fn diag_fp(d: &Diagnostic) -> String {
    format!("{}:{}:{}:{}", d.line, d.character, d.severity, d.message)
}

fn fingerprints_for(snap: &DiagnosticSnapshot, key: &str) -> Vec<String> {
    diags_for(snap, key)
        .into_iter()
        .map(|d| diag_fp(d))
        .collect()
}

fn diags_for<'a>(snap: &'a DiagnosticSnapshot, key: &str) -> Vec<&'a Diagnostic> {
    let mut out = Vec::new();
    for (path, list) in &snap.by_file {
        let pk = normalize_path_key(path);
        if pk == key || path == key || path.ends_with(key) || key.ends_with(path) {
            out.extend(list.iter());
        }
    }
    out
}

/// Best-effort project formatter. Never fails the tool.
pub fn try_format_file(path: &str) -> bool {
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => run_formatter(&["rustfmt", "--edition", "2021", path]),
        "ts" | "tsx" | "js" | "jsx" | "json" | "css" | "md" => {
            if has_config(p, &["biome.json", "biome.jsonc"]) {
                run_formatter(&["npx", "--yes", "biome", "format", "--write", path])
            } else if has_config(
                p,
                &[
                    ".prettierrc",
                    ".prettierrc.js",
                    ".prettierrc.cjs",
                    ".prettierrc.json",
                    "prettier.config.js",
                ],
            ) {
                run_formatter(&["npx", "--yes", "prettier", "--write", path])
            } else {
                false
            }
        }
        "py" => {
            if run_formatter(&["ruff", "format", path]) {
                true
            } else {
                run_formatter(&["black", "-q", path])
            }
        }
        _ => false,
    }
}

fn has_config(file: &Path, names: &[&str]) -> bool {
    let mut dir = file.parent().map(|p| p.to_path_buf());
    while let Some(d) = dir {
        for name in names {
            if d.join(name).exists() {
                return true;
            }
        }
        if d.join("package.json").exists() {
            // stop at package root even if no prettier config — still allow npx miss
        }
        dir = d.parent().map(|p| p.to_path_buf());
        if d.parent().is_none() {
            break;
        }
        // Limit walk
        if d == Path::new("/") {
            break;
        }
    }
    false
}

fn run_formatter(argv: &[&str]) -> bool {
    if argv.is_empty() {
        return false;
    }
    let status = Command::new(argv[0]).args(&argv[1..]).status();
    matches!(status, Ok(s) if s.success())
}

/// Capture baseline, format, wait for LSP, append delta diagnostics.
pub async fn after_mutation(path: &str, mut content: String) -> String {
    let bridge = DiagnosticsBridge::global();
    bridge.capture_baseline(path);

    let formatted = try_format_file(path);
    if formatted {
        content.push_str("\n(formatter applied)");
    }

    // Give LSP a moment to publish (TUI/LSP task refreshes bridge snapshot).
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Also pull from shared bag if someone registered SharedDiagnostics directly.
    if let Some(shared) = SHARED_DIAGS.get() {
        if let Ok(g) = shared.lock() {
            bridge.set_snapshot(g.clone());
        }
    }

    if let Some(report) = bridge.delta_report(path, 20) {
        content.push('\n');
        content.push('\n');
        content.push_str("LSP errors detected in this file, please fix:\n");
        content.push_str(&report);
    }

    let summary = turn_snapshot_note();
    if !summary.is_empty() {
        content.push('\n');
        content.push_str(&summary);
    }

    content
}

fn turn_snapshot_note() -> String {
    crate::tools::turn_snapshot::current_tracked_summary()
}

static SHARED_DIAGS: OnceLock<SharedDiagnostics> = OnceLock::new();

/// Register the live LSP diagnostics bag (called from TUI when LSP starts).
pub fn register_shared_diagnostics(diags: SharedDiagnostics) {
    let _ = SHARED_DIAGS.set(diags);
}

/// Track file for turn snapshot then run after_mutation.
pub async fn prepare_and_finalize(path: &str, result_content: String) -> String {
    let _ = crate::tools::turn_snapshot::track(path);
    after_mutation(path, result_content).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_ignores_baseline() {
        let bridge = DiagnosticsBridge::default();
        let mut snap = DiagnosticSnapshot::default();
        snap.by_file.insert(
            "/tmp/a.rs".into(),
            vec![Diagnostic {
                path: "/tmp/a.rs".into(),
                line: 1,
                character: 0,
                severity: 1,
                message: "old".into(),
                source: "ra".into(),
            }],
        );
        bridge.set_snapshot(snap.clone());
        bridge.capture_baseline("/tmp/a.rs");

        snap.by_file.insert(
            "/tmp/a.rs".into(),
            vec![
                Diagnostic {
                    path: "/tmp/a.rs".into(),
                    line: 1,
                    character: 0,
                    severity: 1,
                    message: "old".into(),
                    source: "ra".into(),
                },
                Diagnostic {
                    path: "/tmp/a.rs".into(),
                    line: 5,
                    character: 0,
                    severity: 1,
                    message: "new err".into(),
                    source: "ra".into(),
                },
            ],
        );
        bridge.set_snapshot(snap);
        let report = bridge.delta_report("/tmp/a.rs", 10).unwrap();
        assert!(report.contains("new err"));
        assert!(!report.contains(">old<") && report.matches("old").count() <= 1);
        // baseline diag should not appear as fresh — only "new err"
        assert!(report.contains("new err"));
        let old_as_fresh = report.lines().filter(|l| l.contains("] old")).count();
        assert_eq!(old_as_fresh, 0);
    }
}
