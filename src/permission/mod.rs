use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub tool_input: String,
    /// Set when the tool call was flagged as risky (e.g. a destructive shell
    /// command); holds a human-readable reason to surface in the prompt.
    pub danger_reason: Option<String>,
    /// Optional unified-diff preview (e.g. for `edit`) shown before allow.
    pub diff_preview: Option<String>,
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

/// Extract a filesystem path from tool JSON args (write/edit/read/…).
pub fn extract_tool_path(tool_input: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(tool_input).ok()?;
    for key in ["file_path", "path", "directory", "dir"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Directory prefix to remember for path-scoped allow (parent of the file).
pub fn path_allow_prefix(file_path: &str) -> String {
    let p = Path::new(file_path);
    let parent = p.parent().unwrap_or(p);
    fs::canonicalize(parent)
        .unwrap_or_else(|_| {
            if parent.is_absolute() {
                parent.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(parent)
            }
        })
        .to_string_lossy()
        .to_string()
}

/// One path-scoped allow rule: tool may run without prompt when target path
/// is under `path_prefix` for this project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathAllowRule {
    pub tool: String,
    pub path_prefix: String,
}

/// Persisted path-scoped permissions (`~/.rs-agent/permissions.json`).
pub struct PathAllowStore {
    file_path: String,
    /// project cwd (normalized) → rules
    data: Mutex<HashMap<String, Vec<PathAllowRule>>>,
}

impl PathAllowStore {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let dir = Path::new(&home).join(".rs-agent");
        let file_path = dir.join("permissions.json").to_string_lossy().to_string();
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

    pub fn allows(&self, project_cwd: &str, tool: &str, target_path: Option<&str>) -> bool {
        let Some(target) = target_path else {
            return false;
        };
        let project = Self::normalize(project_cwd);
        let target_norm = Self::normalize(target);
        let data = self.data.lock().unwrap();
        let Some(rules) = data.get(&project) else {
            return false;
        };
        rules.iter().any(|r| {
            if !(r.tool == "*" || r.tool == tool) {
                return false;
            }
            path_is_under(&target_norm, &r.path_prefix)
        })
    }

    pub fn add_rule(&self, project_cwd: &str, tool: &str, path_prefix: &str) {
        let project = Self::normalize(project_cwd);
        let prefix = Self::normalize(path_prefix);
        let mut data = self.data.lock().unwrap();
        let rules = data.entry(project).or_default();
        let rule = PathAllowRule {
            tool: tool.to_string(),
            path_prefix: prefix,
        };
        if !rules.contains(&rule) {
            rules.push(rule);
            self.save(&data);
        }
    }

    pub fn list_for_project(&self, project_cwd: &str) -> Vec<PathAllowRule> {
        let project = Self::normalize(project_cwd);
        let data = self.data.lock().unwrap();
        data.get(&project).cloned().unwrap_or_default()
    }

    pub fn list_all(&self) -> Vec<(String, Vec<PathAllowRule>)> {
        let data = self.data.lock().unwrap();
        let mut entries: Vec<_> = data.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    pub fn clear_project(&self, project_cwd: &str) {
        let project = Self::normalize(project_cwd);
        let mut data = self.data.lock().unwrap();
        data.remove(&project);
        self.save(&data);
    }

    pub fn clear_all(&self) {
        let mut data = self.data.lock().unwrap();
        data.clear();
        self.save(&data);
    }

    fn normalize(path: &str) -> String {
        let p = Path::new(path);
        fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_string()
    }

    fn save(&self, data: &HashMap<String, Vec<PathAllowRule>>) {
        if let Ok(json) = serde_json::to_string_pretty(data) {
            let _ = fs::write(&self.file_path, &json);
        }
    }

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

impl Default for PathAllowStore {
    fn default() -> Self {
        Self::new()
    }
}

fn path_is_under(target: &str, prefix: &str) -> bool {
    if target == prefix {
        return true;
    }
    let mut p = prefix.to_string();
    if !p.ends_with('/') && !p.ends_with('\\') {
        p.push(std::path::MAIN_SEPARATOR);
    }
    target.starts_with(&p)
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

    #[test]
    fn path_allow_matches_under_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp
            .path()
            .join("permissions.json")
            .to_string_lossy()
            .to_string();
        let store = PathAllowStore::for_path(path);
        let project = tmp.path().to_string_lossy().to_string();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let file = src.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        store.add_rule(&project, "write", src.to_str().unwrap());
        assert!(store.allows(&project, "write", file.to_str()));
        assert!(!store.allows(&project, "bash", file.to_str()));
        assert!(!store.allows(&project, "write", Some("/tmp/other.rs")));
    }

    #[test]
    fn extract_tool_path_reads_aliases() {
        assert_eq!(
            extract_tool_path(r#"{"file_path":"/a/b.rs"}"#).as_deref(),
            Some("/a/b.rs")
        );
        assert_eq!(extract_tool_path(r#"{"path":"/x"}"#).as_deref(), Some("/x"));
    }
}
