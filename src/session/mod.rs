use crate::ai::types::Message;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub provider: String,
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
}

pub struct SessionStore {
    dir: String,
}

impl SessionStore {
    fn home_dir() -> String {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string())
    }

    pub fn new() -> Self {
        let dir = Path::new(&Self::home_dir())
            .join(".rs-agent")
            .join("sessions");
        let _ = fs::create_dir_all(&dir);
        Self {
            dir: dir.to_string_lossy().to_string(),
        }
    }

    pub fn generate_id() -> String {
        Local::now().format("session_%Y%m%d_%H%M%S").to_string()
    }

    pub fn session_path(&self, id: &str) -> String {
        Path::new(&self.dir).join(format!("{}.json", id))
            .to_string_lossy()
            .to_string()
    }

    pub fn exists(&self, id: &str) -> bool {
        Path::new(&self.session_path(id)).exists()
    }

    pub fn save(&self, data: &SessionData) -> Result<(), String> {
        let path = self.session_path(&data.id);
        let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        fs::write(&path, &json).map_err(|e| format!("Failed to save session: {}", e))
    }

    pub fn load(&self, id: &str) -> Result<SessionData, String> {
        let path = self.session_path(id);
        let json = fs::read_to_string(&path).map_err(|e| format!("Session '{}' not found: {}", id, e))?;
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse session '{}': {}", id, e))
    }

    pub fn list(&self) -> Result<Vec<String>, String> {
        let dir = Path::new(&self.dir);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions: Vec<String> = fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect();
        sessions.sort_by(|a, b| b.cmp(a));
        Ok(sessions)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let path = self.session_path(id);
        fs::remove_file(&path).map_err(|e| format!("Failed to delete session '{}': {}", id, e))
    }
}
