//! Headless overnight worker — claim ready beads, implement, verify, close.
//!
//! Phase A observability: per-seat status/log, verbose tool lines, heartbeat
//! during long turns, session persistence, honest idle messaging.

use crate::agent::handoff::{self, HandoffNotes};
use crate::agent::{AbortFlag, AgentEvent, AgentLoop, AgentState, SteerQueue};
use crate::ai::provider::Provider;
use crate::beads::{self, Bead};
use crate::fleet::{self, ControlOp, SeatStatus};
use crate::session::{SessionData, SessionStore};
use crate::tools;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub loop_mode: bool,
    pub budget_minutes: u64,
    pub approve: bool,
    pub seat: Option<String>,
    pub fail_fast: bool,
    pub max_rlm_depth: u32,
    pub max_iterations: usize,
    pub thinking_budget: Option<u32>,
    pub sleep_secs: u64,
    pub goal_verify: bool,
    /// Print/log tool + text summaries (default true for overnight sanity).
    pub verbose: bool,
    /// Dual timeout: no heartbeat/response within this many seconds → stuck (retriable).
    pub response_timeout_secs: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            loop_mode: false,
            budget_minutes: 480,
            approve: true,
            seat: None,
            fail_fast: false,
            max_rlm_depth: 2,
            max_iterations: 99999,
            thinking_budget: None,
            sleep_secs: 5,
            goal_verify: true,
            verbose: true,
            response_timeout_secs: 600,
        }
    }
}

impl WorkerConfig {
    /// Conductor-style dual timeout: response silence vs wall budget.
    pub fn dual_timeout(&self) -> crate::orchestration::DualTimeout {
        crate::orchestration::DualTimeout::from_secs(
            self.response_timeout_secs,
            self.budget_minutes.saturating_mul(60),
        )
    }
}

fn claimant_name(cfg: &WorkerConfig) -> String {
    cfg.seat
        .clone()
        .unwrap_or_else(|| format!("worker-{}", std::process::id()))
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

/// Shared control state for a worker process (pause / abort / steer).
struct WorkerControl {
    abort: AbortFlag,
    steer: SteerQueue,
    pause_requested: AtomicBool,
    resume_requested: AtomicBool,
}

impl WorkerControl {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            abort: AbortFlag::new(),
            steer: SteerQueue::new(),
            pause_requested: AtomicBool::new(false),
            resume_requested: AtomicBool::new(false),
        })
    }

    fn apply_commands(&self, cmds: &[fleet::ControlCommand]) {
        for cmd in cmds {
            match cmd.op {
                ControlOp::Pause => {
                    self.pause_requested.store(true, Ordering::SeqCst);
                    self.abort.abort();
                }
                ControlOp::Resume => {
                    self.resume_requested.store(true, Ordering::SeqCst);
                    self.pause_requested.store(false, Ordering::SeqCst);
                }
                ControlOp::Abort => {
                    self.abort.abort();
                }
                ControlOp::Steer => {
                    if let Some(ref text) = cmd.text {
                        if !text.trim().is_empty() {
                            self.steer.push(text.clone());
                        }
                    }
                }
            }
        }
    }

    fn clear_turn_flags(&self) {
        self.abort.clear();
        self.steer.clear();
    }
}

fn find_claimed_by(claimant: &str) -> Option<Bead> {
    beads::list_claimed(None)
        .ok()?
        .into_iter()
        .find(|b| b.claimant.as_deref() == Some(claimant))
}

fn bead_task_prompt(bead: &Bead, caste: crate::agent::SeatCaste) -> String {
    let caste_brief = match caste {
        crate::agent::SeatCaste::Fleet => {
            "You are a Fleet implementer. Claim and close implement (and task) beads only. \
             Do not redesign — follow the design notes. Prefer small diffs and cargo check/test."
        }
        crate::agent::SeatCaste::Review => {
            "You are a Review caste seat. Verify the linked implement; close on pass, \
             or bead fail (reopens implement) on fail. Do not rewrite the feature."
        }
        crate::agent::SeatCaste::Crew => {
            "You are Crew. Produce clear design beads or review carefully. \
             Closing design spawns implement for Fleet."
        }
        crate::agent::SeatCaste::Marshal => {
            "You are Marshal-adjacent. Prefer assign/reclaim via tools; only execute beads if needed."
        }
        crate::agent::SeatCaste::Seneschal => {
            "You are Seneschal. Triage mail/wishes into beads; avoid deep implementation."
        }
        crate::agent::SeatCaste::Role => {
            "You are a standing role agent. Follow standing orders; unstick or report, then handoff."
        }
        crate::agent::SeatCaste::Any => {
            "You are an overnight factory worker. Implement this bead and close it when done."
        }
    };
    format!(
        "{caste_brief}\n\n\
         Bead id: {}\n\
         Kind: {}\n\
         Title: {}\n\
         Notes:\n{}\n\n\
         Rules:\n\
         - Use tools to implement and verify as appropriate for your caste.\n\
         - Pipeline: design close → spawns implement; implement close → spawns review; \
           review fail → reopens implement (bead fail). Call bead land only after review passed.\n\
         - When finished successfully: call bead close with id {} and a short notes summary.\n\
         - If blocked (needs human/secrets): call bead block with reason, then escalate.\n\
         - Heartbeat the claim if work runs long (bead heartbeat).\n\
         - At the end call the handoff tool with summary/open_threads/next_steps.",
        bead.id,
        bead.kind.as_str(),
        bead.title,
        bead.notes,
        bead.id
    )
}

fn claimant_caste(cfg: &WorkerConfig) -> crate::agent::SeatCaste {
    let name = claimant_name(cfg);
    crate::agent::seat::resolve_caste(&name)
}

fn persist_session(
    store: &SessionStore,
    session_id: &str,
    agent: &AgentLoop,
    model: &str,
    provider: &str,
    seat: &str,
    bead: &Bead,
) {
    let s = agent.state();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let tree_snapshot = serde_json::to_value(agent.call_tree().snapshot()).ok();
    let mut data = SessionData {
        id: session_id.to_string(),
        title: Some(format!("{}: {}", bead.id, truncate(&bead.title, 40))),
        parent_id: None,
        branch_label: Some(format!("worker:{seat}")),
        created_at: now.clone(),
        updated_at: now,
        model: model.to_string(),
        provider: provider.to_string(),
        system_prompt: s.system_prompt.clone(),
        messages: s.messages.clone(),
        total_input_tokens: s.total_input_tokens,
        total_output_tokens: s.total_output_tokens,
        call_tree: tree_snapshot,
        todos: Some(crate::tools::todowrite::snapshot()),
        goal: s.goal.clone(),
        seat: Some(seat.to_string()),
        handoff: s.handoff.clone().or_else(crate::agent::handoff::snapshot),
        project_root: SessionStore::current_project_root(),
    };
    // Preserve created_at if reloading.
    if let Ok(prev) = store.load(session_id) {
        data.created_at = prev.created_at;
        if data.project_root.is_none() {
            data.project_root = prev.project_root;
        }
    }
    data.ensure_title();
    data.stamp_project();
    if let Err(e) = store.save(&data) {
        eprintln!("[worker:{seat}] session save failed: {e}");
        fleet::append_log(seat, &format!("session save failed: {e}"));
    }
}

/// Write a loadable session file as soon as `session_id` is advertised in status
/// (so `/seat open|attach` works mid-turn, before the first persist_session).
fn seed_session_file(
    store: &SessionStore,
    session_id: &str,
    model: &str,
    provider: &str,
    seat: &str,
    bead: &Bead,
    system_prompt: &str,
    state: &crate::agent::AgentState,
) {
    if store.exists(session_id) {
        return;
    }
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut data = SessionData {
        id: session_id.to_string(),
        title: Some(format!("{}: {}", bead.id, truncate(&bead.title, 40))),
        parent_id: None,
        branch_label: Some(format!("worker:{seat}")),
        created_at: now.clone(),
        updated_at: now,
        model: model.to_string(),
        provider: provider.to_string(),
        system_prompt: system_prompt.to_string(),
        messages: state.messages.clone(),
        total_input_tokens: state.total_input_tokens,
        total_output_tokens: state.total_output_tokens,
        call_tree: None,
        todos: Some(crate::tools::todowrite::snapshot()),
        goal: state.goal.clone(),
        seat: Some(seat.to_string()),
        handoff: state
            .handoff
            .clone()
            .or_else(crate::agent::handoff::snapshot),
        project_root: SessionStore::current_project_root(),
    };
    data.ensure_title();
    if let Err(e) = store.save(&data) {
        eprintln!("[worker:{seat}] session seed failed: {e}");
        fleet::append_log(seat, &format!("session seed failed: {e}"));
    } else {
        fleet::append_log(
            seat,
            &format!("session {session_id} seeded (attach/open ready)"),
        );
    }
}

struct EventSink {
    seat: String,
    verbose: bool,
    status: Arc<Mutex<SeatStatus>>,
    last_status_msg: Arc<Mutex<String>>,
    /// Agent activity clock (excludes lease heartbeats) for dual-timeout silence.
    last_activity: Arc<Mutex<Instant>>,
    text_buf: Mutex<String>,
}

impl EventSink {
    fn touch_activity(&self) {
        if let Ok(mut t) = self.last_activity.lock() {
            *t = Instant::now();
        }
    }

    fn emit_line(&self, line: &str) {
        eprintln!("[worker:{}] {line}", self.seat);
        fleet::append_log(&self.seat, line);
        self.touch_activity();
        if let Ok(mut st) = self.status.lock() {
            fleet::heartbeat_touch(&mut st, Some(line));
            fleet::write_seat_status(&st);
        }
    }

    fn handle(&self, ev: &AgentEvent) {
        self.touch_activity();
        match ev {
            AgentEvent::Status { message } => {
                if let Ok(mut m) = self.last_status_msg.lock() {
                    *m = message.clone();
                }
                self.emit_line(message);
            }
            AgentEvent::Error { message } => {
                self.emit_line(&format!("error: {message}"));
                if let Ok(mut st) = self.status.lock() {
                    st.last_error = Some(message.clone());
                    st.state = "error".into();
                    fleet::write_seat_status(&st);
                }
            }
            AgentEvent::ToolUseStart { name, .. } => {
                if let Ok(mut st) = self.status.lock() {
                    st.last_tool = Some(name.clone());
                    st.state = "working".into();
                    fleet::heartbeat_touch(&mut st, Some(&format!("tool:{name}")));
                    fleet::write_seat_status(&st);
                }
                if self.verbose {
                    self.emit_line(&format!("→ tool {name}"));
                }
            }
            AgentEvent::ToolResult { name, result, .. } => {
                let preview = truncate(&result.content.replace('\n', " "), 120);
                let tag = if result.is_error { "ERR" } else { "ok" };
                if self.verbose {
                    self.emit_line(&format!("← {name} [{tag}] {preview}"));
                }
                if let Ok(mut st) = self.status.lock() {
                    st.last_tool = Some(name.clone());
                    fleet::heartbeat_touch(&mut st, Some(&preview));
                    fleet::write_seat_status(&st);
                }
            }
            AgentEvent::ToolOutput { name, text, .. } => {
                if self.verbose {
                    let line = truncate(&text.replace('\n', " "), 100);
                    if !line.trim().is_empty() {
                        self.emit_line(&format!("  {name}| {line}"));
                    }
                }
            }
            AgentEvent::TextDelta { text } => {
                if !self.verbose {
                    return;
                }
                if let Ok(mut buf) = self.text_buf.lock() {
                    buf.push_str(text);
                    if buf.contains('\n') || buf.len() > 160 {
                        let line = truncate(&buf.replace('\n', " "), 160);
                        buf.clear();
                        if !line.trim().is_empty() {
                            // Avoid spamming emit_line's status rewrite for every text chunk —
                            // log only.
                            eprintln!("[worker:{}] say: {line}", self.seat);
                            fleet::append_log(&self.seat, &format!("say: {line}"));
                        }
                    }
                }
            }
            AgentEvent::Done | AgentEvent::Aborted => {
                if let Ok(mut st) = self.status.lock() {
                    fleet::heartbeat_touch(&mut st, None);
                    fleet::write_seat_status(&st);
                }
            }
            _ => {}
        }
    }
}

async fn run_one_bead(
    provider: Arc<dyn Provider>,
    model: String,
    provider_name: String,
    system_prompt: String,
    cfg: &WorkerConfig,
    bead: &Bead,
    claimant: &str,
    status: Arc<Mutex<SeatStatus>>,
    control: Arc<WorkerControl>,
    resume_session_id: Option<String>,
) -> Result<(), String> {
    tools::handoff::set_active_seat(Some(claimant.to_string()));
    let mut state = AgentState::new(model.clone(), provider_name.clone())
        .with_system_prompt(system_prompt)
        .with_thinking_budget(cfg.thinking_budget);
    state.seat = Some(claimant.to_string());
    state.pending_wake = true;
    state.goal = Some(crate::agent::goal::GoalState::new(
        format!("bead:{} closed", bead.id),
        0,
        0,
    ));

    let store = SessionStore::new();
    let session_id = resume_session_id.unwrap_or_else(|| {
        format!(
            "worker_{}_{}",
            fleet::seat_slug(claimant),
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        )
    });
    if let Ok(prev) = store.load(&session_id) {
        state.messages = prev.messages;
        state.goal = prev.goal.or(state.goal);
        state.handoff = prev.handoff;
        if !prev.system_prompt.trim().is_empty() {
            state.system_prompt = prev.system_prompt.clone();
        }
        crate::agent::handoff::restore(state.handoff.clone());
        if let Some(todos) = prev.todos {
            crate::tools::todowrite::restore(todos);
        }
        fleet::append_log(
            claimant,
            &format!(
                "reloaded session {session_id} ({} msgs)",
                state.messages.len()
            ),
        );
    }

    // Advertise session_id only after a loadable file exists on disk.
    seed_session_file(
        &store,
        &session_id,
        &model,
        &provider_name,
        claimant,
        bead,
        &state.system_prompt,
        &state,
    );

    if let Ok(mut st) = status.lock() {
        st.session_id = Some(session_id.clone());
        st.last_bead = Some(bead.id.clone());
        st.last_title = Some(bead.title.clone());
        st.state = "working".into();
        st.last_error = None;
        fleet::clear_paused(&mut st);
        st.state = "working".into();
        fleet::heartbeat_touch(&mut st, Some("starting turn"));
        fleet::write_seat_status(&st);
    }
    fleet::append_log(
        claimant,
        &format!("session {session_id} claimed {} — {}", bead.id, bead.title),
    );

    control.clear_turn_flags();
    // Re-apply any pending pause before we start.
    let pending = fleet::poll_control(claimant);
    control.apply_commands(&pending);
    if control.pause_requested.load(Ordering::SeqCst) {
        // Pause before starting — stub already on disk.
        if let Ok(mut st) = status.lock() {
            st.session_id = Some(session_id.clone());
            fleet::set_paused(&mut st, "tui attach");
            fleet::write_seat_status(&st);
        }
        return Ok(());
    }

    let mut agent = AgentLoop::new(provider, state)
        .with_max_iterations(cfg.max_iterations)
        .with_rlm_depth(0, cfg.max_rlm_depth)
        .with_goal_verify(cfg.goal_verify)
        .with_abort(control.abort.clone())
        .with_steer(control.steer.clone());
    tools::register_default_tools_with_rlm(&mut agent, cfg.max_rlm_depth);

    // Refresh file with live agent state so open/attach mid-turn has context.
    persist_session(
        &store,
        &session_id,
        &agent,
        &model,
        &provider_name,
        claimant,
        bead,
    );

    let stop_ctrl = Arc::new(AtomicBool::new(false));
    let stop_ctrl2 = stop_ctrl.clone();
    let seat_ctrl = claimant.to_string();
    let control_poll = control.clone();
    let ctrl_task = tokio::spawn(async move {
        while !stop_ctrl2.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if stop_ctrl2.load(Ordering::Relaxed) {
                break;
            }
            let cmds = fleet::poll_control(&seat_ctrl);
            control_poll.apply_commands(&cmds);
        }
    });

    // Lease heartbeat every 45s + Conductor dual timeout (silence vs wall).
    let stop_lease = Arc::new(AtomicBool::new(false));
    let stop_lease2 = stop_lease.clone();
    let seat_lease = claimant.to_string();
    let bead_lease = bead.id.clone();
    let status_lease = status.clone();
    let abort_lease = control.abort.clone();
    let dual = cfg.dual_timeout();
    let turn_started = Instant::now();
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let last_activity_lease = last_activity.clone();
    let lease_task = tokio::spawn(async move {
        let mut ticks: u32 = 0;
        while !stop_lease2.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(15)).await;
            if stop_lease2.load(Ordering::Relaxed) {
                break;
            }
            ticks = ticks.saturating_add(1);
            if dual.wall_exceeded(turn_started.elapsed()) {
                fleet::append_log(
                    &seat_lease,
                    &format!("wall timeout on {bead_lease} — aborting turn"),
                );
                abort_lease.abort();
                break;
            }
            let silent = last_activity_lease
                .lock()
                .map(|t| t.elapsed())
                .unwrap_or_default();
            if dual.response_exceeded(silent) {
                fleet::append_log(
                    &seat_lease,
                    &format!(
                        "response silence {}s on {bead_lease} — aborting (retriable)",
                        silent.as_secs()
                    ),
                );
                if let Ok(mut st) = status_lease.lock() {
                    st.last_error = Some(format!("response_timeout:{}s", silent.as_secs()));
                    st.lifecycle = Some("blocked".into());
                    fleet::write_seat_status(&st);
                }
                abort_lease.abort();
                break;
            }
            // Renew bead lease every ~45s (every 3rd tick).
            if ticks % 3 == 0 {
                let _ = beads::heartbeat(None, &bead_lease, &seat_lease);
                if let Ok(mut st) = status_lease.lock() {
                    // Lease renew only — does not reset agent activity clock.
                    fleet::heartbeat_touch(&mut st, Some("lease heartbeat"));
                    fleet::write_seat_status(&st);
                }
                fleet::append_log(&seat_lease, &format!("heartbeat lease on {bead_lease}"));
            }
        }
    });

    let sink = EventSink {
        seat: claimant.to_string(),
        verbose: cfg.verbose,
        status: status.clone(),
        last_status_msg: Arc::new(Mutex::new(String::new())),
        last_activity: last_activity.clone(),
        text_buf: Mutex::new(String::new()),
    };
    let last_status_msg = sink.last_status_msg.clone();

    let had_history = !agent.state().messages.is_empty();
    let prompt = if had_history {
        format!(
            "Continue working on bead {} ({}). \
             Prior conversation is in context. Finish and close the bead when done, \
             or follow any new [steer] instructions.",
            bead.id, bead.title
        )
    } else {
        bead_task_prompt(bead, claimant_caste(cfg))
    };

    let result = agent
        .run(&prompt, &mut |ev| {
            sink.handle(&ev);
        })
        .await;

    stop_ctrl.store(true, Ordering::Relaxed);
    stop_lease.store(true, Ordering::Relaxed);
    let _ = ctrl_task.await;
    let _ = lease_task.await;
    let _ = beads::heartbeat(None, &bead.id, claimant);

    persist_session(
        &store,
        &session_id,
        &agent,
        &model,
        &provider_name,
        claimant,
        bead,
    );
    eprintln!("[worker:{claimant}] saved session `{session_id}` (resume with -r {session_id})");
    fleet::append_log(claimant, &format!("saved session {session_id}"));

    let pausing = control.pause_requested.load(Ordering::SeqCst);
    if pausing {
        if let Ok(mut st) = status.lock() {
            st.session_id = Some(session_id.clone());
            fleet::set_paused(&mut st, "tui attach");
            fleet::write_seat_status(&st);
        }
        fleet::append_log(claimant, "paused for TUI attach — keeping lease");
        // Keep claim; do not release.
        return Ok(());
    }

    match result {
        Ok(()) => {
            if let Ok(Some(b)) = beads::get(None, &bead.id) {
                if b.status == beads::BeadStatus::Claimed {
                    let _ = beads::release(None, &bead.id, Some(claimant));
                    fleet::append_log(
                        claimant,
                        &format!("released {} (turn ended without close)", bead.id),
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            let notes = HandoffNotes::new(
                format!("interrupted: {e}"),
                last_status_msg
                    .lock()
                    .map(|m| m.clone())
                    .unwrap_or_default(),
                format!("Retry bead {}", bead.id),
                vec![bead.id.clone()],
            );
            handoff::store(notes);
            if !control.pause_requested.load(Ordering::SeqCst) {
                let _ = beads::release(None, &bead.id, Some(claimant));
            }
            Err(e)
        }
    }
}

/// Block until resume control, pause TTL, or budget deadline.
async fn wait_while_paused(
    claimant: &str,
    _cfg: &WorkerConfig,
    status: Arc<Mutex<SeatStatus>>,
    control: Arc<WorkerControl>,
    deadline: Instant,
) {
    control.resume_requested.store(false, Ordering::SeqCst);
    eprintln!("[worker:{claimant}] paused — waiting for TUI detach/resume");
    fleet::append_log(claimant, "paused — waiting for resume");

    loop {
        if Instant::now() >= deadline {
            fleet::append_log(claimant, "pause ended: budget exhausted");
            break;
        }
        // Drain control.
        let cmds = fleet::poll_control(claimant);
        control.apply_commands(&cmds);
        if control.resume_requested.load(Ordering::SeqCst) {
            fleet::append_log(claimant, "resume received");
            break;
        }
        let expired = status
            .lock()
            .map(|st| fleet::pause_expired(&st))
            .unwrap_or(false);
        if expired {
            fleet::append_log(
                claimant,
                &format!(
                    "pause TTL ({}s) expired — auto-resume",
                    fleet::PAUSE_TTL_SECS
                ),
            );
            break;
        }
        // Heartbeat lease so other seats do not steal.
        if let Some(b) = find_claimed_by(claimant) {
            let _ = beads::heartbeat(None, &b.id, claimant);
        }
        if let Ok(mut st) = status.lock() {
            if st.state != "paused" && st.state != "attached" {
                let reason = st
                    .paused_reason
                    .clone()
                    .unwrap_or_else(|| "tui attach".into());
                fleet::set_paused(&mut st, &reason);
            }
            fleet::heartbeat_touch(&mut st, Some("paused (awaiting detach)"));
            fleet::write_seat_status(&st);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    control.pause_requested.store(false, Ordering::SeqCst);
    control.resume_requested.store(false, Ordering::SeqCst);
    control.abort.clear();
    if let Ok(mut st) = status.lock() {
        fleet::clear_paused(&mut st);
        st.state = "idle".into();
        fleet::write_seat_status(&st);
    }
    eprintln!("[worker:{claimant}] resumed");
}

/// Run worker once or until budget / empty ready queue.
pub async fn run_worker(
    provider: Arc<dyn Provider>,
    model: String,
    provider_name: String,
    system_prompt: String,
    cfg: WorkerConfig,
) -> Result<(), String> {
    let claimant = claimant_name(&cfg);
    let caste = claimant_caste(&cfg);
    let deadline = Instant::now() + Duration::from_secs(cfg.budget_minutes.saturating_mul(60));
    let status = Arc::new(Mutex::new(fleet::new_working_status(
        &claimant,
        Some(&model),
    )));
    let control = WorkerControl::new();
    {
        let mut st = status.lock().map_err(|e| e.to_string())?;
        st.state = "idle".into();
        fleet::write_seat_status(&st);
    }
    // Worker records its own pid — keep launcher pid file in sync when possible.
    fleet::write_pid(&claimant, std::process::id());
    fleet::append_log(
        &claimant,
        &format!(
            "worker start loop={} budget={}m verbose={} model={} caste={}",
            cfg.loop_mode,
            cfg.budget_minutes,
            cfg.verbose,
            model,
            caste.as_str()
        ),
    );
    eprintln!(
        "[worker:{claimant}] caste={} watching: .rs-agent/fleet/{}.status.json + .log + .control.jsonl",
        caste.as_str(),
        fleet::seat_slug(&claimant)
    );

    let n = beads::reclaim_stale(None).unwrap_or(0);
    if n > 0 {
        eprintln!("[worker:{claimant}] reclaimed {n} stale lease(s)");
        fleet::append_log(&claimant, &format!("reclaimed {n} stale lease(s)"));
    }

    loop {
        if Instant::now() >= deadline {
            eprintln!("[worker:{claimant}] budget exhausted — writing handoff");
            fleet::append_log(&claimant, "budget exhausted — handoff");
            let notes = handoff::HandoffNotes::new(
                format!("Worker `{claimant}` budget exhausted"),
                "Budget wall-clock reached; resume with same seat".into(),
                "Reclaim/assign via marshal; continue ready beads".into(),
                vec![],
            );
            handoff::store(notes.clone());
            if let Ok(mut seat) = crate::agent::seat::load_or_create(&claimant) {
                seat.append_handoff(notes);
                let _ = crate::agent::seat::save(&seat);
            }
            break;
        }

        // Honor pause before claiming (and after turns).
        {
            let cmds = fleet::poll_control(&claimant);
            control.apply_commands(&cmds);
        }
        if control.pause_requested.load(Ordering::SeqCst) {
            if let Ok(mut st) = status.lock() {
                fleet::set_paused(&mut st, "tui attach");
                fleet::write_seat_status(&st);
            }
            wait_while_paused(&claimant, &cfg, status.clone(), control.clone(), deadline).await;
            continue;
        }

        // If we still hold a claimed bead (paused mid-work / TUI detach), continue it.
        let resume_session = status.lock().ok().and_then(|st| st.session_id.clone());
        if let Some(existing) = find_claimed_by(&claimant) {
            eprintln!(
                "[worker:{claimant}] continuing claimed {} — {}",
                existing.id, existing.title
            );
            fleet::append_log(
                &claimant,
                &format!("continuing {} — {}", existing.id, existing.title),
            );
            match run_one_bead(
                provider.clone(),
                model.clone(),
                provider_name.clone(),
                system_prompt.clone(),
                &cfg,
                &existing,
                &claimant,
                status.clone(),
                control.clone(),
                resume_session.clone(),
            )
            .await
            {
                Ok(()) => {
                    if control.pause_requested.load(Ordering::SeqCst) {
                        wait_while_paused(
                            &claimant,
                            &cfg,
                            status.clone(),
                            control.clone(),
                            deadline,
                        )
                        .await;
                        continue;
                    }
                    if let Ok(Some(b)) = beads::get(None, &existing.id) {
                        if let Ok(mut st) = status.lock() {
                            if b.status == beads::BeadStatus::Closed {
                                st.beads_closed += 1;
                                fleet::append_log(&claimant, &format!("closed {}", existing.id));
                            } else if b.status == beads::BeadStatus::Blocked {
                                st.beads_blocked += 1;
                            }
                            st.state = "idle".into();
                            fleet::write_seat_status(&st);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[worker:{claimant}] bead {} failed: {e}", existing.id);
                    fleet::append_log(&claimant, &format!("bead {} failed: {e}", existing.id));
                    if control.pause_requested.load(Ordering::SeqCst) {
                        wait_while_paused(
                            &claimant,
                            &cfg,
                            status.clone(),
                            control.clone(),
                            deadline,
                        )
                        .await;
                        continue;
                    }
                    if let Ok(mut st) = status.lock() {
                        st.last_error = Some(e.clone());
                        st.state = "error".into();
                        fleet::write_seat_status(&st);
                    }
                    if !cfg.fail_fast {
                        tokio::time::sleep(Duration::from_secs(cfg.sleep_secs.max(3))).await;
                        continue;
                    }
                    return Err(e);
                }
            }
            if !cfg.loop_mode {
                break;
            }
            continue;
        }

        let idle_msg = beads::format_backlog_idle_message();
        let bead = match beads::claim_next_for(None, &claimant, caste) {
            Ok(Some(b)) => b,
            Ok(None) => {
                if cfg.loop_mode {
                    let caste_idle = format!(
                        "{idle_msg} (caste `{}` — no allowed ready kinds)",
                        caste.as_str()
                    );
                    eprintln!("[worker:{claimant}] {caste_idle}");
                    eprintln!(
                        "[worker:{claimant}] sleeping {}s (not working — queue not ready)",
                        cfg.sleep_secs
                    );
                    fleet::append_log(&claimant, &caste_idle);
                    if let Ok(mut st) = status.lock() {
                        st.state = "sleeping".into();
                        st.last_error = None;
                        fleet::heartbeat_touch(&mut st, Some(&caste_idle));
                        fleet::write_seat_status(&st);
                    }
                    // Sleep in slices so pause can interrupt.
                    let mut slept = 0u64;
                    while slept < cfg.sleep_secs {
                        if Instant::now() >= deadline {
                            break;
                        }
                        let cmds = fleet::poll_control(&claimant);
                        control.apply_commands(&cmds);
                        if control.pause_requested.load(Ordering::SeqCst) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        slept += 1;
                    }
                    let _ = beads::reclaim_stale(None);
                    continue;
                }
                eprintln!("[worker:{claimant}] {idle_msg} — exiting");
                fleet::append_log(&claimant, &format!("{idle_msg} — exiting"));
                break;
            }
            Err(e) => {
                eprintln!("[worker:{claimant}] claim error: {e}");
                fleet::append_log(&claimant, &format!("claim error: {e}"));
                if cfg.fail_fast {
                    if let Ok(mut st) = status.lock() {
                        st.last_error = Some(e.clone());
                        st.running = false;
                        st.state = "error".into();
                        fleet::write_seat_status(&st);
                    }
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_secs(cfg.sleep_secs)).await;
                continue;
            }
        };

        eprintln!(
            "[worker:{claimant}] claimed {} [{}] — {}",
            bead.id,
            bead.kind.as_str(),
            bead.title
        );
        fleet::append_log(&claimant, &format!("claimed {} — {}", bead.id, bead.title));

        match run_one_bead(
            provider.clone(),
            model.clone(),
            provider_name.clone(),
            system_prompt.clone(),
            &cfg,
            &bead,
            &claimant,
            status.clone(),
            control.clone(),
            None,
        )
        .await
        {
            Ok(()) => {
                if control.pause_requested.load(Ordering::SeqCst) {
                    wait_while_paused(&claimant, &cfg, status.clone(), control.clone(), deadline)
                        .await;
                    continue;
                }
                if let Ok(Some(b)) = beads::get(None, &bead.id) {
                    if let Ok(mut st) = status.lock() {
                        if b.status == beads::BeadStatus::Closed {
                            st.beads_closed += 1;
                            fleet::append_log(&claimant, &format!("closed {}", bead.id));
                        } else if b.status == beads::BeadStatus::Blocked {
                            st.beads_blocked += 1;
                        }
                        st.state = "idle".into();
                        fleet::write_seat_status(&st);
                    }
                }
            }
            Err(e) => {
                eprintln!("[worker:{claimant}] bead {} failed: {e}", bead.id);
                fleet::append_log(&claimant, &format!("bead {} failed: {e}", bead.id));
                if control.pause_requested.load(Ordering::SeqCst) {
                    wait_while_paused(&claimant, &cfg, status.clone(), control.clone(), deadline)
                        .await;
                    continue;
                }
                if let Ok(mut st) = status.lock() {
                    st.last_error = Some(e.clone());
                    st.state = "error".into();
                }
                if e.contains("stream error")
                    || e.contains("error sending request")
                    || crate::agent::AgentLoop::is_transport_failure_msg(&e)
                {
                    let _ = beads::release(None, &bead.id, Some(&claimant));
                } else if cfg.fail_fast {
                    if let Ok(mut st) = status.lock() {
                        st.running = false;
                        fleet::write_seat_status(&st);
                    }
                    return Err(e);
                } else {
                    let _ = beads::block(None, &bead.id, &format!("worker error: {e}"));
                    if let Ok(mut st) = status.lock() {
                        st.beads_blocked += 1;
                    }
                }
                if let Ok(st) = status.lock() {
                    fleet::write_seat_status(&st);
                }
                tokio::time::sleep(Duration::from_secs(cfg.sleep_secs.max(3))).await;
            }
        }

        if !cfg.loop_mode {
            break;
        }
    }

    if let Ok(mut st) = status.lock() {
        st.running = false;
        st.state = "stopped".into();
        fleet::heartbeat_touch(&mut st, Some("stopped"));
        fleet::write_seat_status(&st);
    }
    fleet::append_log(&claimant, "worker stopped");
    Ok(())
}

// Re-exports for TUI compatibility.
pub fn format_status_for_tui() -> String {
    fleet::format_worker_help(None)
}

pub fn read_status() -> Option<crate::fleet::SeatStatus> {
    let seats = fleet::list_seat_statuses();
    seats
        .into_iter()
        .find(|s| s.running)
        .or_else(|| fleet::list_seat_statuses().into_iter().next())
}
