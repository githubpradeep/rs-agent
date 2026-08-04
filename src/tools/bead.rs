//! `bead` tool — create/claim/close units of work in the project work graph.

use crate::agent::tool::*;
use crate::beads::{self, BeadKind};
use crate::hooks::HookRegistry;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct BeadArgs {
    /// list | ready | add | claim | claim_next | close | fail | land | block | gate | ungate | show | heartbeat | reclaim | release
    action: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    claimant: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    deps: Option<Vec<String>>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    lease_secs: Option<u64>,
    /// design | implement | review | task
    #[serde(default)]
    kind: Option<String>,
}

pub struct BeadTool;

fn run_before_bead_close(id: &str) -> Result<(), String> {
    let hooks = HookRegistry::load();
    let Some(b) = beads::get(None, id)? else {
        return Err(format!("bead `{id}` not found"));
    };
    // Built-in: landing an implement requires a passed review.
    // (Ordinary close of implement spawns review; notes with land/ship are gated in close_pipeline.)
    let payload = serde_json::to_string(&serde_json::json!({
        "id": b.id,
        "title": b.title,
        "kind": b.kind.as_str(),
        "status": b.status.as_str(),
        "linked": b.linked,
        "deps": b.deps,
    }))
    .unwrap_or_default();
    hooks.before_bead_close(&payload)
}

#[async_trait]
impl AgentTool for BeadTool {
    fn name(&self) -> &str {
        "bead"
    }

    fn description(&self) -> &str {
        "Manage the project work graph (.rs-agent/beads.json). \
         Actions: list, ready, add (title, kind?, deps?, parent?, priority?), claim, claim_next, \
         close (design→implement→review pipeline), fail (fail review + reopen implement), land, \
         block, gate, ungate, show, heartbeat, reclaim, release. \
         Prefer beads for overnight multi-session goals."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "list | ready | add | claim | claim_next | close | fail | land | block | gate | ungate | show | heartbeat | reclaim | release"
                },
                "id": {"type": "string", "description": "Bead id (e.g. b3)"},
                "title": {"type": "string", "description": "Title for add"},
                "notes": {"type": "string"},
                "claimant": {"type": "string", "description": "Who claims (seat name)"},
                "reason": {"type": "string", "description": "Block/gate/fail reason"},
                "deps": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Dependency bead ids"
                },
                "parent": {"type": "string", "description": "Parent/epic bead id"},
                "priority": {"type": "integer", "description": "Lower = higher priority (default 100)"},
                "lease_secs": {"type": "integer", "description": "Claim lease length in seconds"},
                "kind": {
                    "type": "string",
                    "description": "design | implement | review | task (default task). Closing design spawns implement; closing implement spawns review."
                }
            },
            "required": ["action"]
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: BeadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid bead args: {e}. Expected {{action, ...}}."
                ))
            }
        };
        let action = parsed.action.trim().to_lowercase();
        let who = || {
            parsed
                .claimant
                .clone()
                .or_else(crate::tools::handoff::active_seat)
                .unwrap_or_else(|| "agent".into())
        };
        match action.as_str() {
            "list" | "ls" => match beads::list(None) {
                Ok(items) => {
                    let mut out = String::new();
                    if let Some(c) = beads::format_counts_line(None) {
                        out.push_str(&c);
                        out.push('\n');
                    }
                    out.push_str(&beads::format_summary(&items));
                    ToolExecuteResult::ok(out)
                }
                Err(e) => ToolExecuteResult::error(e),
            },
            "ready" => match beads::list_ready(None) {
                Ok(items) => ToolExecuteResult::ok(if items.is_empty() {
                    "No ready beads.".to_string()
                } else {
                    format!("Ready beads:\n{}", beads::format_summary(&items))
                }),
                Err(e) => ToolExecuteResult::error(e),
            },
            "add" | "create" => {
                let title = parsed.title.unwrap_or_default();
                if title.trim().is_empty() {
                    return ToolExecuteResult::error("add requires title");
                }
                let kind = parsed
                    .kind
                    .as_deref()
                    .and_then(BeadKind::parse)
                    .unwrap_or(BeadKind::Task);
                match beads::add_full(
                    None,
                    &title,
                    parsed.notes.as_deref().unwrap_or(""),
                    parsed.deps.unwrap_or_default(),
                    parsed.parent,
                    parsed.priority.unwrap_or(100),
                    kind,
                    None,
                ) {
                    Ok(b) => ToolExecuteResult::ok(format!(
                        "Created {} [{}] — {}",
                        b.id,
                        b.kind.as_str(),
                        b.title
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "claim" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("claim requires id");
                }
                let lease = parsed.lease_secs.unwrap_or(beads::DEFAULT_LEASE_SECS);
                let who = who();
                let caste = crate::agent::seat::resolve_caste(&who);
                match beads::claim_with_lease_caste(None, &id, &who, lease, Some(caste)) {
                    Ok(b) => ToolExecuteResult::ok(format!(
                        "Claimed {} (@{}, caste {}) — {} lease until {}",
                        b.id,
                        b.claimant.as_deref().unwrap_or("?"),
                        caste.as_str(),
                        b.title,
                        b.lease_expires.unwrap_or(0)
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "claim_next" | "next" => {
                let who = who();
                let caste = crate::agent::seat::resolve_caste(&who);
                match beads::claim_next_for(None, &who, caste) {
                    Ok(Some(b)) => ToolExecuteResult::ok(format!(
                        "Claimed next {} (@{}, caste {}) — {}",
                        b.id,
                        b.claimant.as_deref().unwrap_or("?"),
                        caste.as_str(),
                        b.title
                    )),
                    Ok(None) => ToolExecuteResult::ok(format!(
                        "No ready beads for caste `{}`.",
                        caste.as_str()
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "heartbeat" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("heartbeat requires id");
                }
                let lease = parsed.lease_secs.unwrap_or(beads::DEFAULT_LEASE_SECS);
                match beads::heartbeat_lease(None, &id, &who(), lease) {
                    Ok(b) => ToolExecuteResult::ok(format!(
                        "Heartbeat {} lease until {}",
                        b.id,
                        b.lease_expires.unwrap_or(0)
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "reclaim" => match beads::reclaim_stale(None) {
                Ok(n) => ToolExecuteResult::ok(format!("Reclaimed {n} stale lease(s)")),
                Err(e) => ToolExecuteResult::error(e),
            },
            "release" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("release requires id");
                }
                match beads::release(None, &id, Some(&who())) {
                    Ok(b) => ToolExecuteResult::ok(format!("Released {} — {}", b.id, b.title)),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "close" | "done" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("close requires id");
                }
                if let Err(e) = run_before_bead_close(&id) {
                    return ToolExecuteResult::error(e);
                }
                // Agent-side: closing implement for land requires passed review.
                if let Ok(Some(b)) = beads::get(None, &id) {
                    if b.kind == BeadKind::Implement {
                        let notes = parsed.notes.as_deref().unwrap_or("");
                        let landish = {
                            let l = notes.to_lowercase();
                            l.contains("land") || l.contains("ship")
                        };
                        if landish && !beads::can_land(None, &id).unwrap_or(false) {
                            return ToolExecuteResult::error(
                                "implement land blocked: linked review not closed. \
                                 Close the review bead first, or omit land/ship from notes to \
                                 close implement and spawn a review.",
                            );
                        }
                    }
                }
                match beads::close_pipeline_with_memory(None, &id, parsed.notes.as_deref()) {
                    Ok(r) => {
                        let mut msg = format!(
                            "Closed {} [{}] — {}",
                            r.closed.id,
                            r.closed.kind.as_str(),
                            r.closed.title
                        );
                        if let Some(s) = r.spawned {
                            msg.push_str(&format!(
                                "\nSpawned {} [{}] — {}",
                                s.id,
                                s.kind.as_str(),
                                s.title
                            ));
                        }
                        ToolExecuteResult::ok(msg)
                    }
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "fail" | "fail_review" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("fail requires review bead id");
                }
                match beads::fail_review(
                    None,
                    &id,
                    parsed.reason.as_deref().or(parsed.notes.as_deref()).unwrap_or(""),
                ) {
                    Ok((rev, reopened)) => {
                        let mut msg = format!("Failed review {} — {}", rev.id, rev.title);
                        if let Some(imp) = reopened {
                            msg.push_str(&format!(
                                "\nReopened implement {} — {}",
                                imp.id, imp.title
                            ));
                        }
                        ToolExecuteResult::ok(msg)
                    }
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "land" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("land requires implement bead id");
                }
                match beads::can_land(None, &id) {
                    Ok(true) => ToolExecuteResult::ok(format!(
                        "OK to land `{id}` — linked review is closed."
                    )),
                    Ok(false) => ToolExecuteResult::error(format!(
                        "Cannot land `{id}` — linked review not closed (or failed)."
                    )),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "block" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("block requires id");
                }
                match beads::block(None, &id, parsed.reason.as_deref().unwrap_or("")) {
                    Ok(b) => ToolExecuteResult::ok(format!("Blocked {} — {}", b.id, b.title)),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "gate" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("gate requires id");
                }
                match beads::gate(None, &id, parsed.reason.as_deref().unwrap_or("")) {
                    Ok(b) => ToolExecuteResult::ok(format!("Gated {} — {}", b.id, b.title)),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "ungate" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("ungate requires id");
                }
                match beads::ungate(None, &id) {
                    Ok(b) => ToolExecuteResult::ok(format!("Ungated {} — {}", b.id, b.title)),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            "show" | "get" => {
                let id = parsed.id.unwrap_or_default();
                if id.is_empty() {
                    return ToolExecuteResult::error("show requires id");
                }
                match beads::get(None, &id) {
                    Ok(Some(b)) => ToolExecuteResult::ok(
                        serde_json::to_string_pretty(&b).unwrap_or_else(|_| format!("{b:?}")),
                    ),
                    Ok(None) => ToolExecuteResult::error(format!("bead `{id}` not found")),
                    Err(e) => ToolExecuteResult::error(e),
                }
            }
            _ => ToolExecuteResult::error(format!(
                "Unknown action `{action}`. Use list|ready|add|claim|claim_next|close|fail|land|block|gate|ungate|show|heartbeat|reclaim|release."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn list_empty_ok() {
        let tool = BeadTool;
        let r = tool.execute("1", json!({"action": "list"})).await;
        assert!(!r.is_error || r.content.contains("beads") || r.content.contains("Beads"));
    }
}
