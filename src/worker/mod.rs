//! Headless overnight worker — claim ready beads, implement, verify, close.
//!
//! Phase A observability: per-seat status/log, verbose tool lines, heartbeat
//! during long turns, session persistence, honest idle messaging.

use crate::agent::handoff::{self, HandoffNotes};
use crate::agent::{AgentEvent, AgentLoop, AgentState};
use crate::ai::provider::Provider;
use crate::beads::{self, Bead};
use crate::fleet::{self, SeatStatus};
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
        }
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
    let now = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
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
    };
    // Preserve created_at if reloading.
    if let Ok(prev) = store.load(session_id) {
        data.created_at = prev.created_at;
    }
    data.ensure_title();
    let _ = store.save(&data);
}

struct EventSink {
    seat: String,
    verbose: bool,
    status: Arc<Mutex<SeatStatus>>,
    last_status_msg: Arc<Mutex<String>>,
    text_buf: Mutex<String>,
}

impl EventSink {
    fn emit_line(&self, line: &str) {
        eprintln!("[worker:{}] {line}", self.seat);
        fleet::append_log(&self.seat, line);
        if let Ok(mut st) = self.status.lock() {
            fleet::heartbeat_touch(&mut st, Some(line));
            fleet::write_seat_status(&st);
        }
    }

    fn handle(&self, ev: &AgentEvent) {
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

    let session_id = format!(
        "worker_{}_{}",
        fleet::seat_slug(claimant),
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    if let Ok(mut st) = status.lock() {
        st.session_id = Some(session_id.clone());
        st.last_bead = Some(bead.id.clone());
        st.last_title = Some(bead.title.clone());
        st.state = "working".into();
        st.last_error = None;
        fleet::heartbeat_touch(&mut st, Some("starting turn"));
        fleet::write_seat_status(&st);
    }
    fleet::append_log(
        claimant,
        &format!("session {session_id} claimed {} — {}", bead.id, bead.title),
    );

    let mut agent = AgentLoop::new(provider, state)
        .with_max_iterations(cfg.max_iterations)
        .with_rlm_depth(0, cfg.max_rlm_depth)
        .with_goal_verify(cfg.goal_verify);
    tools::register_default_tools_with_rlm(&mut agent, cfg.max_rlm_depth);

    let stop_hb = Arc::new(AtomicBool::new(false));
    let stop_hb2 = stop_hb.clone();
    let seat_hb = claimant.to_string();
    let bead_hb = bead.id.clone();
    let status_hb = status.clone();
    let hb_task = tokio::spawn(async move {
        while !stop_hb2.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(45)).await;
            if stop_hb2.load(Ordering::Relaxed) {
                break;
            }
            let _ = beads::heartbeat(None, &bead_hb, &seat_hb);
            if let Ok(mut st) = status_hb.lock() {
                fleet::heartbeat_touch(&mut st, Some("lease heartbeat"));
                fleet::write_seat_status(&st);
            }
            fleet::append_log(&seat_hb, &format!("heartbeat lease on {bead_hb}"));
        }
    });

    let sink = EventSink {
        seat: claimant.to_string(),
        verbose: cfg.verbose,
        status: status.clone(),
        last_status_msg: Arc::new(Mutex::new(String::new())),
        text_buf: Mutex::new(String::new()),
    };
    let last_status_msg = sink.last_status_msg.clone();

    let prompt = bead_task_prompt(bead, claimant_caste(cfg));
    let result = agent
        .run(&prompt, &mut |ev| {
            sink.handle(&ev);
        })
        .await;

    stop_hb.store(true, Ordering::Relaxed);
    let _ = hb_task.await;
    let _ = beads::heartbeat(None, &bead.id, claimant);

    let store = SessionStore::new();
    persist_session(
        &store,
        &session_id,
        &agent,
        &model,
        &provider_name,
        claimant,
        bead,
    );
    eprintln!(
        "[worker:{claimant}] saved session `{session_id}` (resume with -r {session_id})"
    );
    fleet::append_log(
        claimant,
        &format!("saved session {session_id}"),
    );

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
                last_status_msg.lock().map(|m| m.clone()).unwrap_or_default(),
                format!("Retry bead {}", bead.id),
                vec![bead.id.clone()],
            );
            handoff::store(notes);
            let _ = beads::release(None, &bead.id, Some(claimant));
            Err(e)
        }
    }
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
            cfg.loop_mode, cfg.budget_minutes, cfg.verbose, model, caste.as_str()
        ),
    );
    eprintln!(
        "[worker:{claimant}] caste={} watching: .rs-agent/fleet/{}.status.json + .log",
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
                    tokio::time::sleep(Duration::from_secs(cfg.sleep_secs)).await;
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
        fleet::append_log(
            &claimant,
            &format!("claimed {} — {}", bead.id, bead.title),
        );

        match run_one_bead(
            provider.clone(),
            model.clone(),
            provider_name.clone(),
            system_prompt.clone(),
            &cfg,
            &bead,
            &claimant,
            status.clone(),
        )
        .await
        {
            Ok(()) => {
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
    seats.into_iter().find(|s| s.running).or_else(|| {
        fleet::list_seat_statuses().into_iter().next()
    })
}
