//! Project work graph (beads v2) — deps, leases, gates, multi-process lock.
//!
//! Stored at `.rs-agent/beads.json`. No external Beads CLI dependency.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default lease length when claiming (seconds).
pub const DEFAULT_LEASE_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeadStatus {
    Open,
    Claimed,
    Blocked,
    Closed,
    /// Waiting on an external gate (CI, human, hook).
    Gated,
}

impl BeadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Claimed => "claimed",
            Self::Blocked => "blocked",
            Self::Closed => "closed",
            Self::Gated => "gated",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "open" => Some(Self::Open),
            "claimed" | "claim" => Some(Self::Claimed),
            "blocked" | "block" => Some(Self::Blocked),
            "closed" | "close" | "done" => Some(Self::Closed),
            "gated" | "gate" => Some(Self::Gated),
            _ => None,
        }
    }
}

/// Pipeline stage for design → implement → review (or ad-hoc `task`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BeadKind {
    Design,
    Implement,
    Review,
    #[default]
    Task,
}

impl BeadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Implement => "implement",
            Self::Review => "review",
            Self::Task => "task",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "design" | "designs" => Some(Self::Design),
            "implement" | "impl" | "implementation" => Some(Self::Implement),
            "review" | "reviews" => Some(Self::Review),
            "task" | "chore" | "" => Some(Self::Task),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bead {
    pub id: String,
    pub title: String,
    pub status: BeadStatus,
    #[serde(default)]
    pub kind: BeadKind,
    #[serde(default)]
    pub deps: Vec<String>,
    /// Optional epic / parent bead id.
    #[serde(default)]
    pub parent: Option<String>,
    /// Prior pipeline stage this bead was spawned from (design→implement→review).
    #[serde(default)]
    pub linked: Option<String>,
    /// Lower = higher priority (default 100).
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default)]
    pub claimant: Option<String>,
    /// Conductor-style fail classification: retriable | terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_kind: Option<String>,
    /// Unix timestamp when the claim lease expires.
    #[serde(default)]
    pub lease_expires: Option<u64>,
    #[serde(default)]
    pub notes: String,
    pub created_at: String,
    pub updated_at: String,
}

fn default_priority() -> i32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BeadsFile {
    #[serde(default)]
    beads: Vec<Bead>,
    #[serde(default)]
    next_id: u64,
}

fn now_str() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn default_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rs-agent")
        .join("beads.json")
}

fn lock_path(beads_path: &Path) -> PathBuf {
    beads_path.with_extension("json.lock")
}

/// Best-effort exclusive lock for multi-process claim/update.
struct BeadsLock {
    path: PathBuf,
}

impl BeadsLock {
    fn acquire(beads_path: &Path) -> Result<Self, String> {
        let path = lock_path(beads_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir lock: {e}"))?;
        }
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(10);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(f) => {
                    let _ = f.set_len(0);
                    let _ = fs::write(&path, format!("{}\n", std::process::id()));
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Stale lock: if older than 60s, break it.
                    if let Ok(meta) = fs::metadata(&path) {
                        if let Ok(modified) = meta.modified() {
                            if modified.elapsed().unwrap_or_default() > Duration::from_secs(60) {
                                let _ = fs::remove_file(&path);
                                continue;
                            }
                        }
                    }
                    if start.elapsed() > timeout {
                        return Err("beads lock timeout — another process holds beads.json".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("beads lock: {e}")),
            }
        }
    }
}

impl Drop for BeadsLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn load_file(path: &Path) -> Result<BeadsFile, String> {
    if !path.is_file() {
        return Ok(BeadsFile::default());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("read beads: {e}"))?;
    if text.trim().is_empty() {
        return Ok(BeadsFile::default());
    }
    serde_json::from_str(&text).map_err(|e| format!("parse beads: {e}"))
}

fn save_file(path: &Path, file: &BeadsFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let text = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &text).map_err(|e| format!("write beads: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename beads: {e}"))?;
    Ok(())
}

fn with_lock_path<F, T>(path: Option<&Path>, f: F) -> Result<T, String>
where
    F: FnOnce(&Path, &mut BeadsFile) -> Result<T, String>,
{
    let p = path.map(|p| p.to_path_buf()).unwrap_or_else(default_path);
    let _lock = BeadsLock::acquire(&p)?;
    let mut file = load_file(&p)?;
    let out = f(&p, &mut file)?;
    save_file(&p, &file)?;
    Ok(out)
}

fn deps_satisfied(file: &BeadsFile, deps: &[String]) -> bool {
    for d in deps {
        match file.beads.iter().find(|b| b.id == *d) {
            Some(b) if b.status == BeadStatus::Closed => {}
            _ => return false,
        }
    }
    true
}

fn is_ready(file: &BeadsFile, b: &Bead) -> bool {
    if b.status != BeadStatus::Open {
        return false;
    }
    deps_satisfied(file, &b.deps)
}

fn lease_expired(b: &Bead) -> bool {
    match b.lease_expires {
        Some(exp) => now_unix() >= exp,
        None => b.status == BeadStatus::Claimed, // legacy claimed without lease = reclaimable
    }
}

pub fn list(path: Option<&Path>) -> Result<Vec<Bead>, String> {
    let p = path.map(|p| p.to_path_buf()).unwrap_or_else(default_path);
    Ok(load_file(&p)?.beads)
}

pub fn list_open(path: Option<&Path>) -> Result<Vec<Bead>, String> {
    Ok(list(path)?
        .into_iter()
        .filter(|b| {
            matches!(
                b.status,
                BeadStatus::Open | BeadStatus::Claimed | BeadStatus::Blocked | BeadStatus::Gated
            )
        })
        .collect())
}

/// Beads that are open and whose deps are all closed, sorted by priority then id.
pub fn list_ready(path: Option<&Path>) -> Result<Vec<Bead>, String> {
    let p = path.map(|p| p.to_path_buf()).unwrap_or_else(default_path);
    let file = load_file(&p)?;
    let mut ready: Vec<Bead> = file
        .beads
        .iter()
        .filter(|b| is_ready(&file, b))
        .cloned()
        .collect();
    ready.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(ready)
}

pub fn counts(path: Option<&Path>) -> Result<(usize, usize, usize, usize, usize), String> {
    let beads = list(path)?;
    let p = path.map(|p| p.to_path_buf()).unwrap_or_else(default_path);
    let file = load_file(&p)?;
    let ready = beads.iter().filter(|b| is_ready(&file, b)).count();
    let claimed = beads
        .iter()
        .filter(|b| b.status == BeadStatus::Claimed)
        .count();
    let blocked = beads
        .iter()
        .filter(|b| b.status == BeadStatus::Blocked)
        .count();
    let gated = beads
        .iter()
        .filter(|b| b.status == BeadStatus::Gated)
        .count();
    let open = beads
        .iter()
        .filter(|b| b.status == BeadStatus::Open)
        .count();
    let _ = open;
    Ok((ready, claimed, blocked, gated, beads.len()))
}

pub fn get(path: Option<&Path>, id: &str) -> Result<Option<Bead>, String> {
    Ok(list(path)?.into_iter().find(|b| b.id == id))
}

pub fn add(
    path: Option<&Path>,
    title: &str,
    notes: &str,
    deps: Vec<String>,
) -> Result<Bead, String> {
    add_full(path, title, notes, deps, None, 100, BeadKind::Task, None)
}

pub fn add_full(
    path: Option<&Path>,
    title: &str,
    notes: &str,
    deps: Vec<String>,
    parent: Option<String>,
    priority: i32,
    kind: BeadKind,
    linked: Option<String>,
) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        Ok(push_bead(
            file,
            title,
            notes,
            deps,
            parent,
            priority,
            kind,
            linked,
        ))
    })
}

fn push_bead(
    file: &mut BeadsFile,
    title: &str,
    notes: &str,
    deps: Vec<String>,
    parent: Option<String>,
    priority: i32,
    kind: BeadKind,
    linked: Option<String>,
) -> Bead {
    file.next_id = file.next_id.max(1);
    let id = format!("b{}", file.next_id);
    file.next_id += 1;
    let t = now_str();
    let bead = Bead {
        id: id.clone(),
        title: title.trim().to_string(),
        status: BeadStatus::Open,
        kind,
        deps,
        parent,
        linked,
        priority,
        claimant: None,
        fail_kind: None,
        lease_expires: None,
        notes: notes.to_string(),
        created_at: t.clone(),
        updated_at: t,
    };
    file.beads.push(bead.clone());
    bead
}

/// Start a design→implement→review pipeline (creates an open design bead).
pub fn add_design(
    path: Option<&Path>,
    title: &str,
    notes: &str,
    parent: Option<String>,
    priority: i32,
) -> Result<Bead, String> {
    add_full(
        path,
        title,
        notes,
        vec![],
        parent,
        priority,
        BeadKind::Design,
        None,
    )
}

pub fn claim(path: Option<&Path>, id: &str, claimant: &str) -> Result<Bead, String> {
    claim_with_lease(path, id, claimant, DEFAULT_LEASE_SECS)
}

pub fn claim_with_lease(
    path: Option<&Path>,
    id: &str,
    claimant: &str,
    lease_secs: u64,
) -> Result<Bead, String> {
    claim_with_lease_caste(path, id, claimant, lease_secs, None)
}

pub fn claim_with_lease_caste(
    path: Option<&Path>,
    id: &str,
    claimant: &str,
    lease_secs: u64,
    caste: Option<crate::agent::seat::SeatCaste>,
) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        reclaim_stale_unlocked(file);

        let idx = file
            .beads
            .iter()
            .position(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;

        {
            let bead = &file.beads[idx];
            if bead.status == BeadStatus::Closed {
                return Err(format!("bead `{id}` is closed"));
            }
            if bead.status == BeadStatus::Gated {
                return Err(format!("bead `{id}` is gated — ungate first"));
            }
            if bead.status == BeadStatus::Claimed && !lease_expired(bead) {
                if bead.claimant.as_deref() != Some(claimant.trim()) {
                    return Err(format!(
                        "bead `{id}` claimed by {} until {}",
                        bead.claimant.as_deref().unwrap_or("?"),
                        bead.lease_expires.unwrap_or(0)
                    ));
                }
            }
            if !deps_satisfied(file, &bead.deps) {
                return Err(format!(
                    "bead `{id}` deps not satisfied: {}",
                    bead.deps.join(", ")
                ));
            }
            if let Some(c) = caste {
                if !c.allows_kind(bead.kind) {
                    return Err(format!(
                        "caste `{}` cannot claim bead kind `{}` ({id})",
                        c.as_str(),
                        bead.kind.as_str()
                    ));
                }
            }
        }

        let bead = &mut file.beads[idx];
        bead.status = BeadStatus::Claimed;
        bead.claimant = Some(claimant.trim().to_string());
        bead.lease_expires = Some(now_unix().saturating_add(lease_secs));
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

/// Claim the highest-priority ready bead for `claimant`.
pub fn claim_next(path: Option<&Path>, claimant: &str) -> Result<Option<Bead>, String> {
    claim_next_for(path, claimant, crate::agent::seat::SeatCaste::Any)
}

/// Claim next ready bead allowed for this caste.
///
/// Prefers a priority/postpone ready-queue item when present (Wave 9).
pub fn claim_next_for(
    path: Option<&Path>,
    claimant: &str,
    caste: crate::agent::seat::SeatCaste,
) -> Result<Option<Bead>, String> {
    if let Ok(Some(item)) = crate::queue::pop() {
        // Queue payload is bead id (or id\t…).
        let id = item.payload.split('\t').next().unwrap_or(&item.id).trim();
        if !id.is_empty() {
            if let Ok(Some(b)) = get(path, id) {
                if b.status == BeadStatus::Open && caste.allows_kind(b.kind) {
                    return Ok(Some(claim_with_lease_caste(
                        path,
                        id,
                        claimant,
                        DEFAULT_LEASE_SECS,
                        Some(caste),
                    )?));
                }
            }
        }
        // Fall through if queue item was stale.
    }
    let ready = list_ready(path)?;
    let Some(first) = ready.into_iter().find(|b| caste.allows_kind(b.kind)) else {
        return Ok(None);
    };
    Ok(Some(claim_with_lease_caste(
        path,
        &first.id,
        claimant,
        DEFAULT_LEASE_SECS,
        Some(caste),
    )?))
}

/// Marshal assign: claim a specific bead for a seat (caste Any — admin override).
pub fn assign(path: Option<&Path>, id: &str, seat: &str) -> Result<Bead, String> {
    claim_with_lease_caste(path, id, seat, DEFAULT_LEASE_SECS, None)
}

pub fn heartbeat(path: Option<&Path>, id: &str, claimant: &str) -> Result<Bead, String> {
    heartbeat_lease(path, id, claimant, DEFAULT_LEASE_SECS)
}

pub fn heartbeat_lease(
    path: Option<&Path>,
    id: &str,
    claimant: &str,
    lease_secs: u64,
) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        let bead = file
            .beads
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        if bead.status != BeadStatus::Claimed {
            return Err(format!("bead `{id}` is not claimed"));
        }
        if bead.claimant.as_deref() != Some(claimant.trim()) {
            return Err(format!(
                "bead `{id}` claimed by {}, not {claimant}",
                bead.claimant.as_deref().unwrap_or("?")
            ));
        }
        bead.lease_expires = Some(now_unix().saturating_add(lease_secs));
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

fn reclaim_stale_unlocked(file: &mut BeadsFile) -> usize {
    let mut n = 0;
    for b in &mut file.beads {
        if b.status == BeadStatus::Claimed && lease_expired(b) {
            b.status = BeadStatus::Open;
            b.claimant = None;
            b.lease_expires = None;
            b.updated_at = now_str();
            if !b.notes.is_empty() {
                b.notes.push('\n');
            }
            b.notes.push_str("reclaimed: lease expired");
            n += 1;
        }
    }
    n
}

pub fn reclaim_stale(path: Option<&Path>) -> Result<usize, String> {
    with_lock_path(path, |_p, file| Ok(reclaim_stale_unlocked(file)))
}

pub fn release(path: Option<&Path>, id: &str, claimant: Option<&str>) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        let bead = file
            .beads
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        if let Some(c) = claimant {
            if bead.claimant.as_deref() != Some(c) && bead.status == BeadStatus::Claimed {
                return Err(format!("cannot release `{id}` — not claimed by {c}"));
            }
        }
        bead.status = BeadStatus::Open;
        bead.claimant = None;
        bead.lease_expires = None;
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

/// Result of closing a bead (may spawn the next pipeline stage).
#[derive(Debug, Clone)]
pub struct CloseResult {
    pub closed: Bead,
    pub spawned: Option<Bead>,
}

pub fn close(path: Option<&Path>, id: &str, notes: Option<&str>) -> Result<Bead, String> {
    Ok(close_pipeline(path, id, notes)?.closed)
}

/// Close a bead and advance the design→implement→review pipeline when applicable.
pub fn close_pipeline(
    path: Option<&Path>,
    id: &str,
    notes: Option<&str>,
) -> Result<CloseResult, String> {
    with_lock_path(path, |_p, file| close_pipeline_unlocked(file, id, notes))
}

fn close_pipeline_unlocked(
    file: &mut BeadsFile,
    id: &str,
    notes: Option<&str>,
) -> Result<CloseResult, String> {
    let idx = file
        .beads
        .iter()
        .position(|b| b.id == id)
        .ok_or_else(|| format!("bead `{id}` not found"))?;

    let wants_land = notes
        .map(|n| {
            let l = n.to_lowercase();
            l.contains("land") || l.contains("ship")
        })
        .unwrap_or(false);
    let (kind, bead_id) = {
        let bead = file
            .beads
            .get(idx)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        (bead.kind, bead.id.clone())
    };
    if wants_land && kind == BeadKind::Implement && !review_closed_for(file, &bead_id) {
        return Err(format!(
            "cannot land implement `{id}` — linked review not closed (close or fail the review first)"
        ));
    }

    {
        let bead = &mut file.beads[idx];
        bead.status = BeadStatus::Closed;
        bead.claimant = None;
        bead.lease_expires = None;
        if let Some(n) = notes {
            if !n.trim().is_empty() {
                if !bead.notes.is_empty() {
                    bead.notes.push('\n');
                }
                bead.notes.push_str(n.trim());
            }
        }
        bead.updated_at = now_str();
    }

    let closed = file.beads[idx].clone();
    let spawned = spawn_next_stage(file, &closed);
    // Provenance best-effort (outside lock after clone — call after with_lock returns).
    Ok(CloseResult { closed, spawned })
}

/// Close and record ledger/brain provenance.
pub fn close_pipeline_with_memory(
    path: Option<&Path>,
    id: &str,
    notes: Option<&str>,
) -> Result<CloseResult, String> {
    let result = close_pipeline(path, id, notes)?;
    let summary = notes.unwrap_or("").trim();
    let summary = if summary.is_empty() {
        result.closed.title.as_str()
    } else {
        summary
    };
    let _ = crate::brain::record_close(&result.closed.id, result.closed.kind.as_str(), summary);
    Ok(result)
}

fn spawn_next_stage(file: &mut BeadsFile, closed: &Bead) -> Option<Bead> {
    let (next_kind, title_prefix) = match closed.kind {
        BeadKind::Design => (BeadKind::Implement, "Implement"),
        BeadKind::Implement => (BeadKind::Review, "Review"),
        BeadKind::Review | BeadKind::Task => return None,
    };
    // Avoid duplicate open/claimed next-stage beads for the same linked id.
    let already = file.beads.iter().any(|b| {
        b.linked.as_deref() == Some(closed.id.as_str())
            && b.kind == next_kind
            && b.status != BeadStatus::Closed
    });
    if already {
        return None;
    }
    let title = if closed.title.to_lowercase().starts_with(&title_prefix.to_lowercase()) {
        closed.title.clone()
    } else {
        format!("{title_prefix}: {}", closed.title)
    };
    let parent = closed.parent.clone().or_else(|| Some(closed.id.clone()));
    Some(push_bead(
        file,
        &title,
        &format!("auto-spawned from {} ({})", closed.id, closed.kind.as_str()),
        vec![closed.id.clone()],
        parent,
        closed.priority,
        next_kind,
        Some(closed.id.clone()),
    ))
}

fn review_closed_for(file: &BeadsFile, implement_id: &str) -> bool {
    file.beads.iter().any(|b| {
        b.kind == BeadKind::Review
            && b.linked.as_deref() == Some(implement_id)
            && b.status == BeadStatus::Closed
            && !b.notes.to_lowercase().contains("review fail")
    })
}

/// Fail a review bead: mark it closed with fail note and reopen the linked implement.
pub fn fail_review(path: Option<&Path>, id: &str, reason: &str) -> Result<(Bead, Option<Bead>), String> {
    with_lock_path(path, |_p, file| {
        let idx = file
            .beads
            .iter()
            .position(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        if file.beads[idx].kind != BeadKind::Review {
            return Err(format!("bead `{id}` is not a review bead"));
        }
        let linked = file.beads[idx].linked.clone();
        {
            let review = &mut file.beads[idx];
            review.status = BeadStatus::Closed;
            review.claimant = None;
            review.lease_expires = None;
            if !review.notes.is_empty() {
                review.notes.push('\n');
            }
            let r = reason.trim();
            if r.is_empty() {
                review.notes.push_str("review fail");
            } else {
                review.notes.push_str(&format!("review fail: {r}"));
            }
            review.updated_at = now_str();
        }
        let review = file.beads[idx].clone();
        let mut reopened = None;
        if let Some(impl_id) = linked {
            if let Some(imp) = file.beads.iter_mut().find(|b| b.id == impl_id) {
                imp.status = BeadStatus::Open;
                imp.claimant = None;
                imp.lease_expires = None;
                if !imp.notes.is_empty() {
                    imp.notes.push('\n');
                }
                imp.notes.push_str(&format!(
                    "reopened after review {} failed: {}",
                    id,
                    if reason.trim().is_empty() {
                        "(no reason)"
                    } else {
                        reason.trim()
                    }
                ));
                imp.updated_at = now_str();
                reopened = Some(imp.clone());
            }
        }
        Ok((review, reopened))
    })
}

/// Check whether an implement bead's review has passed (closed without fail).
pub fn can_land(path: Option<&Path>, implement_id: &str) -> Result<bool, String> {
    let p = path.map(|p| p.to_path_buf()).unwrap_or_else(default_path);
    let file = load_file(&p)?;
    let Some(b) = file.beads.iter().find(|b| b.id == implement_id) else {
        return Err(format!("bead `{implement_id}` not found"));
    };
    if b.kind != BeadKind::Implement && b.kind != BeadKind::Task {
        return Err(format!(
            "`{implement_id}` is {} — land expects an implement bead",
            b.kind.as_str()
        ));
    }
    Ok(review_closed_for(&file, implement_id))
}

/// Human-readable backlog idle explanation for workers / fleet UI.
pub fn format_backlog_idle_message() -> String {
    let ready = list_ready(None).unwrap_or_default();
    let open = list_open(None).unwrap_or_default();
    let claimed = list_claimed(None).unwrap_or_default();
    let blocked = open
        .iter()
        .filter(|b| matches!(b.status, BeadStatus::Blocked | BeadStatus::Gated))
        .count();
    let waiting_deps = open
        .iter()
        .filter(|b| b.status == BeadStatus::Open)
        .filter(|b| !ready.iter().any(|r| r.id == b.id))
        .count();

    if ready.is_empty() && open.is_empty() {
        return "Backlog: empty (no open beads). Add work before overnight.".into();
    }
    if ready.is_empty() {
        return format!(
            "Backlog: 0 ready · {} open not-ready ({} waiting on deps, {} blocked/gated, {} claimed). \
             Factory is idle until deps clear or leases expire.",
            open.len(),
            waiting_deps,
            blocked,
            claimed.len()
        );
    }
    format!(
        "Backlog: {} ready · {} open · {} claimed",
        ready.len(),
        open.len(),
        claimed.len()
    )
}

/// Active lease holders (claimed beads).
pub fn list_claimed(path: Option<&Path>) -> Result<Vec<Bead>, String> {
    Ok(list(path)?
        .into_iter()
        .filter(|b| b.status == BeadStatus::Claimed)
        .collect())
}

/// Fleet / marshal summary string.
pub fn format_fleet_status(path: Option<&Path>) -> String {
    let _ = reclaim_stale(path);
    let beads = list(path).unwrap_or_default();
    let ready = list_ready(path).unwrap_or_default();
    let claimed = list_claimed(path).unwrap_or_default();
    let mut out = String::new();
    if let Some(c) = format_counts_line(path) {
        out.push_str(&c);
        out.push('\n');
    }
    if claimed.is_empty() {
        out.push_str("Fleet: no active leases.\n");
    } else {
        out.push_str("Fleet leases:\n");
        for b in &claimed {
            out.push_str(&format!(
                "  {} (@{}) [{}] {} lease_until={}\n",
                b.id,
                b.claimant.as_deref().unwrap_or("?"),
                b.kind.as_str(),
                b.title,
                b.lease_expires.unwrap_or(0)
            ));
        }
    }
    if !ready.is_empty() {
        out.push_str("Ready queue:\n");
        for b in ready.iter().take(12) {
            out.push_str(&format!(
                "  {} [{}] (p{}) {}\n",
                b.id,
                b.kind.as_str(),
                b.priority,
                b.title
            ));
        }
    }
    let blocked: Vec<_> = beads
        .iter()
        .filter(|b| matches!(b.status, BeadStatus::Blocked | BeadStatus::Gated))
        .collect();
    if !blocked.is_empty() {
        out.push_str("Blocked/gated:\n");
        for b in blocked.iter().take(8) {
            out.push_str(&format!(
                "  {} [{}] {}\n",
                b.id,
                b.status.as_str(),
                b.title
            ));
        }
    }
    if out.is_empty() {
        "No beads.".into()
    } else {
        out
    }
}

pub fn block(path: Option<&Path>, id: &str, reason: &str) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        let bead = file
            .beads
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        bead.status = BeadStatus::Blocked;
        bead.claimant = None;
        bead.lease_expires = None;
        if !reason.trim().is_empty() {
            if !bead.notes.is_empty() {
                bead.notes.push('\n');
            }
            bead.notes.push_str(&format!("blocked: {}", reason.trim()));
        }
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

/// Fail a bead with retriable vs terminal outcome (Conductor FAILED_WITH_TERMINAL_ERROR).
/// Terminal fails stay blocked and must not be auto-reclaimed for retry.
pub fn fail(
    path: Option<&Path>,
    id: &str,
    reason: &str,
    kind: crate::orchestration::FailKind,
) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        let bead = file
            .beads
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        bead.status = BeadStatus::Blocked;
        bead.claimant = None;
        bead.lease_expires = None;
        bead.fail_kind = Some(kind.as_str().to_string());
        if !bead.notes.is_empty() {
            bead.notes.push('\n');
        }
        bead.notes
            .push_str(&format!("fail({}): {}", kind.as_str(), reason.trim()));
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

/// True when a blocked bead should not be auto-retried.
pub fn is_terminal_fail(bead: &Bead) -> bool {
    bead.fail_kind
        .as_deref()
        .map(|k| crate::orchestration::FailKind::parse(k) == crate::orchestration::FailKind::Terminal)
        .unwrap_or(false)
}

/// Reopen a blocked bead for retry — refused for terminal fails.
pub fn reopen_for_retry(path: Option<&Path>, id: &str) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        let bead = file
            .beads
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        if bead.status != BeadStatus::Blocked {
            return Err(format!("bead `{id}` is not blocked"));
        }
        if is_terminal_fail(bead) {
            return Err(format!(
                "bead `{id}` has terminal fail — will not auto-retry"
            ));
        }
        bead.status = BeadStatus::Open;
        bead.claimant = None;
        bead.lease_expires = None;
        bead.fail_kind = None;
        if !bead.notes.is_empty() {
            bead.notes.push('\n');
        }
        bead.notes.push_str("reopened: retriable fail");
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

/// Ungate beads whose notes contain `wait_until:<rfc3339>` in the past (Conductor WAIT sweeper).
pub fn ungate_due(path: Option<&Path>) -> Result<usize, String> {
    with_lock_path(path, |_p, file| {
        let now = chrono::Local::now();
        let mut n = 0usize;
        for bead in file.beads.iter_mut() {
            if bead.status != BeadStatus::Gated {
                continue;
            }
            let Some(marker) = bead.notes.lines().rev().find_map(|l| {
                l.trim()
                    .strip_prefix("wait_until:")
                    .map(|s| s.trim().to_string())
            }) else {
                continue;
            };
            if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&marker) {
                if ts <= now {
                    bead.status = BeadStatus::Open;
                    bead.updated_at = now_str();
                    bead.notes.push_str("\nungated: wait_until elapsed");
                    n += 1;
                }
            }
        }
        Ok(n)
    })
}

pub fn gate(path: Option<&Path>, id: &str, reason: &str) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        let bead = file
            .beads
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        bead.status = BeadStatus::Gated;
        bead.claimant = None;
        bead.lease_expires = None;
        if !reason.trim().is_empty() {
            if !bead.notes.is_empty() {
                bead.notes.push('\n');
            }
            bead.notes.push_str(&format!("gate: {}", reason.trim()));
        }
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

pub fn ungate(path: Option<&Path>, id: &str) -> Result<Bead, String> {
    with_lock_path(path, |_p, file| {
        let bead = file
            .beads
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| format!("bead `{id}` not found"))?;
        if bead.status != BeadStatus::Gated && bead.status != BeadStatus::Blocked {
            return Err(format!("bead `{id}` is not gated/blocked"));
        }
        bead.status = BeadStatus::Open;
        bead.updated_at = now_str();
        Ok(bead.clone())
    })
}

pub fn format_summary(beads: &[Bead]) -> String {
    if beads.is_empty() {
        return "No beads.".into();
    }
    let mut out = String::from("Beads:\n");
    for b in beads {
        let who = b
            .claimant
            .as_deref()
            .map(|c| format!(" @{c}"))
            .unwrap_or_default();
        let parent = b
            .parent
            .as_deref()
            .map(|p| format!(" parent:{p}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  [{}] {} [{}] — {}{}{} (p{})\n",
            b.status.as_str(),
            b.id,
            b.kind.as_str(),
            b.title,
            who,
            parent,
            b.priority
        ));
    }
    out
}

pub fn format_counts_line(path: Option<&Path>) -> Option<String> {
    let (ready, claimed, blocked, gated, _total) = counts(path).ok()?;
    if ready == 0 && claimed == 0 && blocked == 0 && gated == 0 {
        return None;
    }
    Some(format!(
        "beads: {ready} ready, {claimed} claimed, {blocked} blocked, {gated} gated"
    ))
}

/// Wake-packet listing of ready + open work (capped).
pub fn format_wake_block(limit: usize) -> Option<String> {
    let ready = list_ready(None).unwrap_or_default();
    let open = list_open(None).unwrap_or_default();
    if ready.is_empty() && open.is_empty() {
        return None;
    }
    let mut out = String::from("## Beads\n");
    if !ready.is_empty() {
        out.push_str("### Ready\n");
        for b in ready.iter().take(limit) {
            out.push_str(&format!(
                "- {} [{}] (p{}) {}\n",
                b.id,
                b.kind.as_str(),
                b.priority,
                b.title
            ));
        }
    }
    let non_ready: Vec<_> = open
        .iter()
        .filter(|b| b.status != BeadStatus::Open || !ready.iter().any(|r| r.id == b.id))
        .collect();
    if !non_ready.is_empty() {
        out.push_str("### In flight / blocked\n");
        for b in non_ready.iter().take(limit) {
            out.push_str(&format!(
                "- {} [{}] {}\n",
                b.id,
                b.status.as_str(),
                b.title
            ));
        }
    }
    Some(out)
}

pub fn goal_mentions_beads(condition: &str) -> bool {
    let lower = condition.to_lowercase();
    lower.contains("bead:")
        || lower.contains("beads clear")
        || lower.contains("no open beads")
        || lower.contains("no open ready")
        || lower.contains("all beads closed")
        || lower.contains("backlog empty")
}

pub fn evaluate_bead_condition(condition: &str) -> Option<(bool, String)> {
    let lower = condition.to_lowercase();
    if lower.contains("no open ready")
        || lower.contains("ready beads empty")
        || lower.contains("no ready beads")
    {
        let ready = list_ready(None).unwrap_or_default();
        if ready.is_empty() {
            return Some((true, "no ready beads".into()));
        }
        return Some((false, format!("{} ready bead(s) remain", ready.len())));
    }
    if lower.contains("no open beads")
        || lower.contains("all beads closed")
        || lower.contains("beads clear")
        || lower.contains("backlog empty")
    {
        let open = list_open(None).unwrap_or_default();
        if open.is_empty() {
            return Some((true, "no open beads".into()));
        }
        return Some((
            false,
            format!("{} open bead(s) remain", open.len()),
        ));
    }
    if let Some(rest) = lower.strip_prefix("bead:") {
        let id = rest.split_whitespace().next().unwrap_or("").trim();
        if id.is_empty() {
            return None;
        }
        match get(None, id) {
            Ok(Some(b)) if b.status == BeadStatus::Closed => {
                Some((true, format!("{id} closed")))
            }
            Ok(Some(b)) => Some((
                false,
                format!("{id} is {}", b.status.as_str()),
            )),
            Ok(None) => Some((false, format!("bead `{id}` not found"))),
            Err(e) => Some((false, e)),
        }
    } else {
        None
    }
}

/// Soft / vague goals should not declare victory while ready work remains.
pub fn is_soft_goal(condition: &str) -> bool {
    let lower = condition.to_lowercase();
    lower.contains("keep implement")
        || lower.contains("keep working")
        || lower.contains("continue implement")
        || lower.contains("implement operators")
        || lower.contains("implement more")
        || (lower.contains("keep") && lower.contains("implement"))
}

pub fn soft_goal_blocked_by_backlog(condition: &str) -> bool {
    if !is_soft_goal(condition) {
        return false;
    }
    !list_ready(None).unwrap_or_default().is_empty()
        || !list_open(None).unwrap_or_default().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crud_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("beads.json");
        let b = add(Some(&path), "fix auth", "", vec![]).unwrap();
        assert_eq!(b.id, "b1");
        claim(Some(&path), "b1", "fox").unwrap();
        let g = get(Some(&path), "b1").unwrap().unwrap();
        assert_eq!(g.status, BeadStatus::Claimed);
        assert!(g.lease_expires.is_some());
        close(Some(&path), "b1", Some("done")).unwrap();
        let open = list_open(Some(&path)).unwrap();
        assert!(open.is_empty());
    }

    #[test]
    fn deps_gate_ready_and_claim() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("beads.json");
        let a = add(Some(&path), "first", "", vec![]).unwrap();
        let b = add(Some(&path), "second", "", vec![a.id.clone()]).unwrap();
        let ready = list_ready(Some(&path)).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, a.id);
        assert!(claim(Some(&path), &b.id, "w").is_err());
        close(Some(&path), &a.id, None).unwrap();
        claim(Some(&path), &b.id, "w").unwrap();
    }

    #[test]
    fn reclaim_expired_lease() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("beads.json");
        let b = add(Some(&path), "x", "", vec![]).unwrap();
        claim_with_lease(Some(&path), &b.id, "w", 0).unwrap();
        // lease_expires = now, so immediately expired
        std::thread::sleep(Duration::from_millis(20));
        let n = reclaim_stale(Some(&path)).unwrap();
        assert!(n >= 1);
        let g = get(Some(&path), &b.id).unwrap().unwrap();
        assert_eq!(g.status, BeadStatus::Open);
    }

    #[test]
    fn goal_bead_mentions() {
        assert!(goal_mentions_beads("no open beads"));
        assert!(goal_mentions_beads("bead:b3 closed"));
        assert!(!goal_mentions_beads("all tests pass"));
    }

    #[test]
    fn soft_goal_detect() {
        assert!(is_soft_goal("keep implementing operators"));
        assert!(!is_soft_goal("cargo test passes"));
    }

    #[test]
    fn pipeline_design_to_review() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("beads.json");
        let d = add_design(Some(&path), "auth", "", None, 10).unwrap();
        assert_eq!(d.kind, BeadKind::Design);
        let r1 = close_pipeline(Some(&path), &d.id, None).unwrap();
        let imp = r1.spawned.expect("implement spawned");
        assert_eq!(imp.kind, BeadKind::Implement);
        assert_eq!(imp.deps, vec![d.id.clone()]);
        let r2 = close_pipeline(Some(&path), &imp.id, None).unwrap();
        let rev = r2.spawned.expect("review spawned");
        assert_eq!(rev.kind, BeadKind::Review);
        assert!(!can_land(Some(&path), &imp.id).unwrap());

        let (_failed, reopened) = fail_review(Some(&path), &rev.id, "needs tests").unwrap();
        let reopened = reopened.expect("implement reopened");
        assert_eq!(reopened.status, BeadStatus::Open);

        let r3 = close_pipeline(Some(&path), &imp.id, None).unwrap();
        let rev2 = r3.spawned.expect("second review spawned");
        close(Some(&path), &rev2.id, Some("lgtm")).unwrap();
        assert!(can_land(Some(&path), &imp.id).unwrap());
    }

    #[test]
    fn fleet_caste_cannot_claim_design() {
        use crate::agent::SeatCaste;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("beads.json");
        let d = add_design(Some(&path), "design only", "", None, 10).unwrap();
        let err = claim_with_lease_caste(
            Some(&path),
            &d.id,
            "Fleet-1",
            DEFAULT_LEASE_SECS,
            Some(SeatCaste::Fleet),
        )
        .unwrap_err();
        assert!(err.contains("cannot claim"), "{err}");
        // Ready queue has design, but claim_next_for fleet skips it.
        assert!(claim_next_for(Some(&path), "Fleet-1", SeatCaste::Fleet)
            .unwrap()
            .is_none());
        // Crew can claim design.
        claim_with_lease_caste(
            Some(&path),
            &d.id,
            "Crew-1",
            DEFAULT_LEASE_SECS,
            Some(SeatCaste::Crew),
        )
        .unwrap();
    }

    #[test]
    fn marshal_assign_bypasses_caste() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("beads.json");
        let d = add_design(Some(&path), "assigned design", "", None, 10).unwrap();
        let b = assign(Some(&path), &d.id, "Fleet-1").unwrap();
        assert_eq!(b.claimant.as_deref(), Some("Fleet-1"));
    }
}
