use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub tool_input: String,
}

#[derive(Debug, Clone)]
pub enum PermissionReply {
    Allow,
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
}
