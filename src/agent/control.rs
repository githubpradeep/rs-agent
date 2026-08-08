use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Shared abort flag for cancelling an in-flight agent turn (and RLM subtrees).
///
/// `abort()` is synchronous and wakes any `wait()` futures immediately so
/// bash / streaming do not sit on a poll interval.
#[derive(Clone)]
pub struct AbortFlag {
    inner: Arc<AbortInner>,
}

struct AbortInner {
    aborted: AtomicBool,
    notify: Notify,
}

impl Default for AbortFlag {
    fn default() -> Self {
        Self::new()
    }
}

impl AbortFlag {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AbortInner {
                aborted: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn abort(&self) {
        self.inner.aborted.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn clear(&self) {
        self.inner.aborted.store(false, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.inner.aborted.load(Ordering::SeqCst)
    }

    /// Resolves as soon as [`Self::abort`] is called (or if already aborted).
    pub async fn wait(&self) {
        loop {
            if self.is_aborted() {
                return;
            }
            // Subscribe before re-check so we cannot miss a notify.
            let notified = self.inner.notify.notified();
            if self.is_aborted() {
                return;
            }
            notified.await;
        }
    }
}

/// Queue of user steer messages injected between agent turns while running.
#[derive(Clone, Default)]
pub struct SteerQueue {
    inner: Arc<Mutex<VecDeque<String>>>,
}

impl SteerQueue {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn push(&self, text: String) {
        if let Ok(mut q) = self.inner.lock() {
            q.push_back(text);
        }
    }

    pub fn drain(&self) -> Vec<String> {
        if let Ok(mut q) = self.inner.lock() {
            q.drain(..).collect()
        } else {
            Vec::new()
        }
    }

    pub fn clear(&self) {
        if let Ok(mut q) = self.inner.lock() {
            q.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn wait_wakes_immediately_on_abort() {
        let flag = AbortFlag::new();
        let f = flag.clone();
        let start = Instant::now();
        let h = tokio::spawn(async move {
            f.wait().await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        flag.abort();
        tokio::time::timeout(Duration::from_millis(200), h)
            .await
            .expect("wait timed out")
            .expect("task panicked");
        assert!(start.elapsed() < Duration::from_millis(150));
    }
}
