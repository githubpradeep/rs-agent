//! Serialize file mutations targeting the same path (pi-style file mutation queue).

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

static QUEUES: once_cell_queues::Lazy<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    once_cell_queues::Lazy::new(|| Mutex::new(HashMap::new()));

/// Minimal once_cell-like lazy without extra dependency.
mod once_cell_queues {
    use std::sync::OnceLock;
    pub struct Lazy<T> {
        cell: OnceLock<T>,
        init: fn() -> T,
    }
    impl<T> Lazy<T> {
        pub const fn new(init: fn() -> T) -> Self {
            Self {
                cell: OnceLock::new(),
                init,
            }
        }
        pub fn get(&self) -> &T {
            self.cell.get_or_init(self.init)
        }
    }
    impl<T> std::ops::Deref for Lazy<T> {
        type Target = T;
        fn deref(&self) -> &T {
            self.get()
        }
    }
}

fn queue_key(file_path: &str) -> String {
    let p = Path::new(file_path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(p)
    };
    abs.canonicalize()
        .unwrap_or(abs)
        .to_string_lossy()
        .to_string()
}

async fn lock_path(file_path: &str) -> OwnedMutexGuard<()> {
    let key = queue_key(file_path);
    let arc = {
        let mut map = QUEUES.lock().await;
        map.entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    arc.lock_owned().await
}

/// Run `f` while holding the per-file mutation lock.
pub async fn with_file_lock<T, F, Fut>(file_path: &str, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let _guard = lock_path(file_path).await;
    f().await
}

/// Extract a file_path-like string from tool args JSON, if present.
pub fn path_from_tool_args(name: &str, args: &serde_json::Value) -> Option<String> {
    if !matches!(name, "edit" | "write" | "apply_patch") {
        return None;
    }
    let normalized = crate::tools::normalize_file_tool_args(args.clone());
    normalized
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            normalized
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}
