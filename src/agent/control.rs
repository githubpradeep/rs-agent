use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Shared abort flag for cancelling an in-flight agent turn (and RLM subtrees).
#[derive(Clone, Default)]
pub struct AbortFlag {
    inner: Arc<AtomicBool>,
}

impl AbortFlag {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn abort(&self) {
        self.inner.store(true, Ordering::SeqCst);
    }

    pub fn clear(&self) {
        self.inner.store(false, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
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
