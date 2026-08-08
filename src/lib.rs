pub mod ai;
pub mod agent;
pub mod beads;
pub mod brain;
pub mod cli;
pub mod config;
pub mod context;
pub mod hooks;
pub mod lifecycle;
pub mod lsp;
pub mod mail;
pub mod mcp;
pub mod moot;
pub mod notify;
pub mod orchestration;
pub mod permission;
pub mod prompts;
pub mod queue;
pub mod rlm;
pub mod roles;
pub mod runtime;
pub mod schedule;
pub mod session;
pub mod skills;
pub mod tools;
pub mod tui;
pub mod wish;
pub mod worker;
pub mod marshal;
pub mod fleet;

/// Serialize tests that mutate process CWD (tempdir races otherwise).
#[cfg(test)]
pub fn with_temp_cwd<R>(f: impl FnOnce(&std::path::Path) -> R) -> R {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let fallback = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let prev = std::env::current_dir().unwrap_or_else(|_| fallback.clone());
    std::env::set_current_dir(tmp.path()).expect("set cwd");
    let out = f(tmp.path());
    let _ = std::env::set_current_dir(&prev).or_else(|_| std::env::set_current_dir(&fallback));
    out
}
