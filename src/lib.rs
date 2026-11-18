pub mod agent;
pub mod ai;
pub mod cli;
pub mod config;
pub mod context;
pub mod hooks;
pub mod lsp;
pub mod mcp;
pub mod permission;
pub mod prompts;
pub mod rlm;
pub mod session;
pub mod skills;
pub mod tools;
pub mod tui;

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
