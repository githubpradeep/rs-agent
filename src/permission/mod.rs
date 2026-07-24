use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub tool_input: String,
    /// Set when the tool call was flagged as risky (e.g. a destructive shell
    /// command); holds a human-readable reason to surface in the prompt.
    pub danger_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub enum PermissionReply {
    /// Allow this single tool call, without remembering the decision.
    AllowOnce,
    /// Allow this call and mark the current project as trusted so future
    /// calls are auto-allowed.
    AllowAlways,
    Deny,
}

pub struct PendingPermission {
    pub request: PermissionRequest,
    pub reply_tx: tokio::sync::oneshot::Sender<PermissionReply>,
}

pub struct TrustStore {
    file_path: String,
    data: Mutex<HashMap<String, bool>>,
}

impl TrustStore {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let dir = Path::new(&home).join(".rs-agent");
        let file_path = dir.join("trust.json").to_string_lossy().to_string();
        let _ = fs::create_dir_all(&dir);

        let data = fs::read_to_string(&file_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        Self {
            file_path,
            data: Mutex::new(data),
        }
    }

    pub fn is_trusted(&self, cwd: &str) -> bool {
        let data = self.data.lock().unwrap();
        let normalized = Self::normalize(cwd);
        data.get(&normalized).copied().unwrap_or(false)
    }

    pub fn set_trusted(&self, cwd: &str, trusted: bool) {
        let mut data = self.data.lock().unwrap();
        let normalized = Self::normalize(cwd);
        data.insert(normalized, trusted);
        self.save(&data);
    }

    /// List all known `(path, trusted)` entries, sorted by path.
    pub fn list(&self) -> Vec<(String, bool)> {
        let data = self.data.lock().unwrap();
        let mut entries: Vec<(String, bool)> = data.iter().map(|(k, v)| (k.clone(), *v)).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    /// Remove all trust entries and persist the (now-empty) store to disk.
    pub fn clear(&self) {
        let mut data = self.data.lock().unwrap();
        data.clear();
        self.save(&data);
    }

    /// Alias for [`Self::clear`]: wipes all trust entries from `trust.json`.
    pub fn reset(&self) {
        self.clear();
    }

    fn normalize(path: &str) -> String {
        let p = Path::new(path);
        fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string()
    }

    fn save(&self, data: &HashMap<String, bool>) {
        if let Ok(json) = serde_json::to_string_pretty(data) {
            let _ = fs::write(&self.file_path, &json);
        }
    }

    /// Test-only constructor pointing at an arbitrary trust file path, so
    /// tests don't need to mutate the process-wide `HOME` env var.
    #[cfg(test)]
    fn for_path(file_path: impl Into<String>) -> Self {
        let file_path = file_path.into();
        let data = fs::read_to_string(&file_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            file_path,
            data: Mutex::new(data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_trust_store() -> (tempfile::TempDir, TrustStore) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trust.json").to_string_lossy().to_string();
        let store = TrustStore::for_path(path);
        (tmp, store)
    }

    #[test]
    fn new_store_has_no_trusted_paths() {
        let (_tmp, store) = temp_trust_store();
        assert!(store.list().is_empty());
        assert!(!store.is_trusted("/some/project"));
    }

    #[test]
    fn set_trusted_persists_and_list_reflects_it() {
        let (_tmp, store) = temp_trust_store();
        store.set_trusted("/some/project", true);
        assert!(store.is_trusted("/some/project"));

        let entries = store.list();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].1);
    }

    #[test]
    fn list_is_sorted_by_path() {
        let (_tmp, store) = temp_trust_store();
        store.set_trusted("/z/project", true);
        store.set_trusted("/a/project", false);
        store.set_trusted("/m/project", true);

        let entries = store.list();
        let paths: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn clear_removes_all_entries_and_persists() {
        let (_tmp, store) = temp_trust_store();
        store.set_trusted("/some/project", true);
        store.set_trusted("/other/project", false);
        assert_eq!(store.list().len(), 2);

        store.clear();
        assert!(store.list().is_empty());
        assert!(!store.is_trusted("/some/project"));
    }

    #[test]
    fn reset_is_alias_for_clear() {
        let (_tmp, store) = temp_trust_store();
        store.set_trusted("/some/project", true);
        assert_eq!(store.list().len(), 1);

        store.reset();
        assert!(store.list().is_empty());
    }

    #[test]
    fn clear_reflected_after_reload_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trust.json").to_string_lossy().to_string();

        let store = TrustStore::for_path(path.clone());
        store.set_trusted("/some/project", true);
        store.clear();

        let reloaded = TrustStore::for_path(path);
        assert!(reloaded.list().is_empty());
    }
}
