//! Standing role runners — Beadle, Gargoyle, and friends.

use crate::agent::SeatCaste;
use crate::beads::{self, BeadStatus};
use crate::fleet;
use crate::mail;
use crate::marshal;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    Beadle,
    Gargoyle,
    Drawbridge,
    Scryer,
    Marshal,
}

impl RoleKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "beadle" => Some(Self::Beadle),
            "gargoyle" => Some(Self::Gargoyle),
            "drawbridge" => Some(Self::Drawbridge),
            "scryer" => Some(Self::Scryer),
            "marshal" => Some(Self::Marshal),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Beadle => "Beadle",
            Self::Gargoyle => "Gargoyle",
            Self::Drawbridge => "Drawbridge",
            Self::Scryer => "Scryer",
            Self::Marshal => "Marshal",
        }
    }

    pub fn seat_name(self) -> &'static str {
        self.as_str()
    }
}

fn roles_brain_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("brain")
        .join("roles")
}

/// Ensure standing-orders markdown exists for a role.
pub fn ensure_role_orders(kind: RoleKind) -> Result<PathBuf, String> {
    let dir = roles_brain_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir brain/roles: {e}"))?;
    let path = dir.join(format!("{}.md", kind.as_str().to_lowercase()));
    if !path.exists() {
        let body = match kind {
            RoleKind::Beadle => {
                "# Beadle\n\nFind stuck leases, blocked beads without reason, idle fleet with ready work.\n\
                 Prefer marshal reclaim/auto-assign; mail Seneschal when human judgment needed.\n"
            }
            RoleKind::Gargoyle => {
                "# Gargoyle\n\nRepo health: run `cargo test` (or project check). On red, gate/block \
                 related beads and mail Seneschal.\n"
            }
            RoleKind::Drawbridge => {
                "# Drawbridge\n\nWatch CI/deploy hooks when present; mail on red.\n"
            }
            RoleKind::Scryer => {
                "# Scryer\n\nOptional: ingest paths/URLs into wishes (`rs-agent wish`).\n"
            }
            RoleKind::Marshal => {
                "# Marshal\n\nReclaim stale leases, release dead pids, auto-assign implement beads.\n"
            }
        };
        fs::write(&path, body).map_err(|e| format!("write role orders: {e}"))?;
    }
    let _ = crate::agent::seat::ensure_with_caste(kind.seat_name(), SeatCaste::Role);
    Ok(path)
}

fn run_beadle() -> String {
    let mut out = String::from("Beadle pass:\n");
    // Reuse marshal reclaim + dead pid + auto-assign.
    let report = marshal::run_with_opts(marshal::MarshalOpts {
        auto_assign: true,
        max_assign: 4,
        stuck_mins: 30,
        mail_stuck: true,
    });
    out.push_str(&report);
    // Flag blocked without notes.
    for b in beads::list(None).unwrap_or_default() {
        if b.status == BeadStatus::Blocked && b.notes.trim().is_empty() {
            let _ = mail::send(
                "Beadle",
                "Seneschal",
                &format!("Blocked bead {} has empty reason: {}", b.id, b.title),
                vec![b.id.clone()],
            );
            out.push_str(&format!("  warned empty block reason on {}\n", b.id));
        }
    }
    // Idle fleet + ready work nudge is already in marshal auto-assign.
    let ready_impl = beads::list_ready(None)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| {
            matches!(
                b.kind,
                crate::beads::BeadKind::Implement | crate::beads::BeadKind::Task
            )
        })
        .count();
    let idle = fleet::list_seat_statuses()
        .into_iter()
        .filter(|s| s.running && (s.state == "idle" || s.state == "sleeping"))
        .count();
    out.push_str(&format!(
        "Beadle note: ready_impl/task={ready_impl} idle_running_seats={idle}\n"
    ));
    out
}

fn run_gargoyle() -> String {
    let mut out = String::from("Gargoyle pass:\n");
    let status = std::process::Command::new("cargo")
        .args(["test", "--lib", "--", "--quiet"])
        .output();
    match status {
        Ok(o) if o.status.success() => {
            out.push_str("  cargo test --lib: green\n");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let snippet: String = stderr.chars().take(400).collect();
            out.push_str("  cargo test --lib: RED\n");
            out.push_str(&format!("  {snippet}\n"));
            let _ = mail::send(
                "Gargoyle",
                "Seneschal",
                &format!("cargo test red:\n{snippet}"),
                vec![],
            );
            // Soft-gate: mark highest-priority open implement as gated with reason.
            if let Ok(ready) = beads::list_ready(None) {
                if let Some(b) = ready
                    .into_iter()
                    .find(|b| b.kind == crate::beads::BeadKind::Implement)
                {
                    let _ = beads::gate(None, &b.id, "gargoyle: cargo test red");
                    out.push_str(&format!("  gated {}\n", b.id));
                }
            }
        }
        Err(e) => out.push_str(&format!("  could not run cargo: {e}\n")),
    }
    out
}

fn run_drawbridge() -> String {
    let mut out = String::from("Drawbridge pass:\n");
    // Lightweight: if .github exists and gh is available, note checks — else noop message.
    if std::path::Path::new(".github").is_dir() {
        let gh = std::process::Command::new("gh")
            .args(["pr", "checks"])
            .output();
        match gh {
            Ok(o) if o.status.success() => {
                out.push_str("  gh pr checks: ok (or no PR)\n");
            }
            Ok(o) => {
                let body = String::from_utf8_lossy(&o.stdout);
                let err = String::from_utf8_lossy(&o.stderr);
                let snippet = if body.trim().is_empty() {
                    err.chars().take(300).collect::<String>()
                } else {
                    body.chars().take(300).collect::<String>()
                };
                out.push_str(&format!("  checks attention:\n{snippet}\n"));
                let _ = mail::send(
                    "Drawbridge",
                    "Seneschal",
                    &format!("CI/checks:\n{snippet}"),
                    vec![],
                );
            }
            Err(_) => out.push_str("  gh not available — skip\n"),
        }
    } else {
        out.push_str("  no .github/ — skip\n");
    }
    out
}

fn run_scryer(path_or_url: Option<&str>) -> String {
    let mut out = String::from("Scryer pass:\n");
    let Some(src) = path_or_url else {
        out.push_str("  nothing to ingest (pass --source)\n");
        return out;
    };
    let text = if src.starts_with("http://") || src.starts_with("https://") {
        format!("Ingest URL wish: {src}")
    } else if let Ok(body) = fs::read_to_string(src) {
        format!(
            "Ingest file {src}:\n{}",
            body.chars().take(1500).collect::<String>()
        )
    } else {
        format!("Ingest note: {src}")
    };
    match crate::wish::create_wish(&text, false, false) {
        Ok(b) => out.push_str(&format!("  created wish bead {}\n", b.id)),
        Err(e) => out.push_str(&format!("  wish fail: {e}\n")),
    }
    out
}

pub fn run_once(kind: RoleKind, source: Option<&str>) -> Result<String, String> {
    let _ = ensure_role_orders(kind)?;
    let out = match kind {
        RoleKind::Beadle => run_beadle(),
        RoleKind::Gargoyle => run_gargoyle(),
        RoleKind::Drawbridge => run_drawbridge(),
        RoleKind::Scryer => run_scryer(source),
        RoleKind::Marshal => marshal::run_once(),
    };
    Ok(out)
}

pub async fn run_loop(
    kind: RoleKind,
    source: Option<String>,
    interval_secs: u64,
    budget_minutes: u64,
) -> Result<(), String> {
    let deadline =
        std::time::Instant::now() + Duration::from_secs(budget_minutes.saturating_mul(60));
    loop {
        if std::time::Instant::now() >= deadline {
            eprintln!("[role:{}] budget exhausted", kind.as_str());
            break;
        }
        match run_once(kind, source.as_deref()) {
            Ok(msg) => println!("{msg}"),
            Err(e) => eprintln!("[role:{}] {e}", kind.as_str()),
        }
        tokio::time::sleep(Duration::from_secs(interval_secs.max(30))).await;
    }
    Ok(())
}
