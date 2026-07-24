//! Stored API keys in `~/.rs-agent/secrets.toml` (not the main config).
//!
//! Format:
//! ```toml
//! [api_keys]
//! anthropic = "sk-ant-..."
//! openrouter = "sk-or-..."
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::config_dir;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Secrets {
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
}

impl Secrets {
    pub fn path() -> PathBuf {
        config_dir().join("secrets.toml")
    }

    pub fn load() -> Self {
        let path = Self::path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let _ = super::Config::ensure_user_dir();
        let path = Self::path();
        let body = format!(
            "# rs-agent secrets — keep this file private (chmod 600)\n\n{}",
            toml::to_string_pretty(self).unwrap_or_default()
        );
        std::fs::write(&path, body)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn get_key(&self, provider: &str) -> Option<&str> {
        self.api_keys
            .get(&provider.to_lowercase())
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn set_key(&mut self, provider: &str, key: String) {
        self.api_keys.insert(provider.to_lowercase(), key);
    }
}

/// Apply all stored secrets into the process environment (does not overwrite
/// existing env vars).
pub fn export_secrets_to_env() {
    let secrets = Secrets::load();
    for (provider, key) in secrets.api_keys {
        if key.is_empty() {
            continue;
        }
        let env = crate::ai::registry::api_key_env_for(&provider);
        if std::env::var(env).is_err() {
            std::env::set_var(env, &key);
        }
    }
}

/// Save a key for `provider`, write secrets.toml, and export to env.
pub fn store_api_key(provider: &str, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("API key is empty".to_string());
    }
    let mut secrets = Secrets::load();
    secrets.set_key(provider, key.to_string());
    secrets
        .save()
        .map_err(|e| format!("failed to write {}: {}", Secrets::path().display(), e))?;
    let env = crate::ai::registry::api_key_env_for(provider);
    std::env::set_var(env, key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_roundtrip_in_memory() {
        let mut s = Secrets::default();
        s.set_key("openrouter", "sk-or-test".into());
        assert_eq!(s.get_key("openrouter"), Some("sk-or-test"));
        assert_eq!(s.get_key("OPENROUTER"), Some("sk-or-test"));
    }
}
