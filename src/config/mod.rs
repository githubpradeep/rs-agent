//! Config loading for rs-agent.
//!
//! Merge order (later wins for any field it sets):
//!   1. `Config::default()` (all `None` / empty)
//!   2. user config: `~/.rs-agent/config.toml`
//!   3. project config: `.rs-agent/settings.toml` (relative to cwd)
//!   4. project config: `.rs-agent.toml` (relative to cwd)
//!
//! CLI flags always win over anything loaded here; callers should only use
//! config values to fill in fields the user left at their CLI defaults.

mod secrets;

pub use secrets::{export_secrets_to_env, store_api_key, Secrets};

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const USER_CONFIG_TEMPLATE: &str = r#"# rs-agent user config
#
# Uncomment and edit values to change your defaults. CLI flags always
# override anything set here, and project-level config
# (.rs-agent/settings.toml or .rs-agent.toml in a project directory)
# overrides this file.

# Last /model or /provider selection is written here automatically.
# provider = "anthropic"
# model = "claude-sonnet-4-20250514"
# approve = true
# auto_mode = false
# rlm_depth = 2
# rlm_escalate_chars = 10000
# goal_verify = true
# thinking_budget = 10000
# max_iterations = 99999
# timeout = 300
# base_url = "https://api.anthropic.com/v1"
# disable_mouse = false
# theme = "auto"   # dark | light | forest | auto (COLORFGBG)
# toast = true
# toast_sound = false
# notify = "off"   # off | terminal | system
# allowed_transitions = ["*"]  # routing handoff allow-list

# [model_aliases]
# fast = "claude-haiku-4-20250514"
# smart = "claude-opus-4-20250514"

# [keybindings]
# insert = "i"
# quit = "q"
# toggle_thinking = "t"
# jump_bottom = "G"
# expand_tool = "e"
# toggle_tree = "T"
# perm_once = "a"
# perm_always = "t"
# perm_path = "p"
# perm_deny = "d"

# [[mcp.servers]]
# name = "filesystem"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
"#;

/// User-facing configuration, merged from user + project config files.
///
/// Every field is optional: an absent/`None` value means "not configured",
/// and the caller (CLI) decides what default to fall back to.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub approve: Option<bool>,
    pub auto_mode: Option<bool>,
    pub rlm_depth: Option<u32>,
    /// Char threshold for auto Deep Context escalate hints on huge reads (default 10000).
    pub rlm_escalate_chars: Option<usize>,
    pub thinking_budget: Option<u32>,
    pub max_iterations: Option<usize>,
    /// When true (default), `/goal` runs a tool-using verify subagent after the transcript check.
    pub goal_verify: Option<bool>,
    pub timeout: Option<u64>,
    pub base_url: Option<String>,
    pub model_aliases: HashMap<String, String>,
    pub disable_mouse: Option<bool>,
    pub theme: Option<String>,
    /// In-app attention toasts (blocked/done).
    pub toast: Option<bool>,
    /// Play a short sound with toasts.
    pub toast_sound: Option<bool>,
    /// External notify when unfocused: off | terminal | system.
    pub notify: Option<String>,
    /// Remappable single-key actions (see `tui::keys::default_keybindings`).
    pub keybindings: HashMap<String, String>,
    /// MCP stdio servers (`[[mcp.servers]]`).
    #[serde(default)]
    pub mcp: McpConfig,
    /// Allowed routing handoff transitions (`*` or `from->to`).
    #[serde(default)]
    pub allowed_transitions: Vec<String>,
}

/// MCP client configuration.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct McpConfig {
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// When `Some(false)`, skip connecting this server.
    pub enabled: Option<bool>,
}

impl Config {
    /// Load and merge config from the user config file and any project
    /// config files found relative to the current working directory.
    ///
    /// Never fails: missing files are silently skipped, and files that
    /// fail to parse produce a warning on stderr but don't stop loading.
    pub fn load() -> Config {
        let mut cfg = Config::default();

        cfg.merge_from_file(&config_dir().join("config.toml"));

        let cwd = std::env::current_dir().unwrap_or_default();
        cfg.merge_from_file(&cwd.join(".rs-agent").join("settings.toml"));
        cfg.merge_from_file(&cwd.join(".rs-agent.toml"));

        cfg
    }

    fn merge_from_file(&mut self, path: &Path) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        match Config::parse_str(&content) {
            Ok(other) => self.merge(other),
            Err(e) => eprintln!("Warning: failed to parse config {}: {}", path.display(), e),
        }
    }

    fn parse_str(content: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(content)
    }

    /// Merge `other` on top of `self`. Any field set (`Some`) in `other`
    /// overrides the corresponding field in `self`. `model_aliases` entries
    /// are merged key-by-key, with `other` winning on conflicts.
    fn merge(&mut self, other: Config) {
        if other.provider.is_some() {
            self.provider = other.provider;
        }
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.approve.is_some() {
            self.approve = other.approve;
        }
        if other.auto_mode.is_some() {
            self.auto_mode = other.auto_mode;
        }
        if other.rlm_depth.is_some() {
            self.rlm_depth = other.rlm_depth;
        }
        if other.rlm_escalate_chars.is_some() {
            self.rlm_escalate_chars = other.rlm_escalate_chars;
        }
        if other.thinking_budget.is_some() {
            self.thinking_budget = other.thinking_budget;
        }
        if other.max_iterations.is_some() {
            self.max_iterations = other.max_iterations;
        }
        if other.goal_verify.is_some() {
            self.goal_verify = other.goal_verify;
        }
        if other.timeout.is_some() {
            self.timeout = other.timeout;
        }
        if other.base_url.is_some() {
            self.base_url = other.base_url;
        }
        if other.disable_mouse.is_some() {
            self.disable_mouse = other.disable_mouse;
        }
        if other.theme.is_some() {
            self.theme = other.theme;
        }
        if other.toast.is_some() {
            self.toast = other.toast;
        }
        if other.toast_sound.is_some() {
            self.toast_sound = other.toast_sound;
        }
        if other.notify.is_some() {
            self.notify = other.notify;
        }
        for (k, v) in other.model_aliases {
            self.model_aliases.insert(k, v);
        }
        for (k, v) in other.keybindings {
            self.keybindings.insert(k, v);
        }
        if !other.mcp.servers.is_empty() {
            // Project/user overlay replaces the server list when present.
            self.mcp = other.mcp;
        }
        if !other.allowed_transitions.is_empty() {
            self.allowed_transitions = other.allowed_transitions;
        }
    }

    /// Create `~/.rs-agent` and its standard subdirectories if missing.
    pub fn ensure_user_dir() -> std::io::Result<PathBuf> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::create_dir_all(dir.join("skills"))?;
        std::fs::create_dir_all(dir.join("prompts"))?;
        std::fs::create_dir_all(dir.join("sessions"))?;
        Ok(dir)
    }

    /// Path to the user config file (`~/.rs-agent/config.toml`).
    pub fn user_config_path() -> PathBuf {
        config_dir().join("config.toml")
    }

    /// True if the user config file is missing or still the commented template
    /// (no real `provider =` assignment).
    pub fn user_config_needs_wizard() -> bool {
        let path = Self::user_config_path();
        match std::fs::read_to_string(&path) {
            Err(_) => true,
            Ok(content) => !content.lines().any(|l| {
                let t = l.trim();
                t.starts_with("provider") && !t.starts_with('#')
            }),
        }
    }

    /// Write a commented example config to `~/.rs-agent/config.toml` if no
    /// config file exists there yet. No-op if it already exists.
    pub fn write_default_user_config_if_missing() -> std::io::Result<()> {
        let dir = Self::ensure_user_dir()?;
        let path = dir.join("config.toml");
        if path.exists() {
            return Ok(());
        }
        std::fs::write(path, USER_CONFIG_TEMPLATE)
    }

    /// Persist this config as real (uncommented) TOML to `~/.rs-agent/config.toml`.
    pub fn save_user_config(&self) -> std::io::Result<()> {
        let dir = Self::ensure_user_dir()?;
        let path = dir.join("config.toml");
        let body = self.to_toml_string();
        std::fs::write(path, body)
    }

    /// Load only `~/.rs-agent/config.toml` (ignores project config overlays).
    pub fn load_user_file() -> Config {
        let mut cfg = Config::default();
        cfg.merge_from_file(&Self::user_config_path());
        cfg
    }

    /// Remember the last mid-session provider/model so the next restart
    /// restores it (unless overridden by `--provider` / `--model`).
    ///
    /// Updates the user config file only — does not pull in project overlays.
    pub fn persist_last_selection(provider: &str, model: &str) -> Result<(), String> {
        let provider = provider.trim();
        let model = model.trim();
        if provider.is_empty() || model.is_empty() {
            return Err("provider and model must be non-empty".into());
        }
        let mut cfg = Self::load_user_file();
        cfg.provider = Some(provider.to_string());
        cfg.model = Some(model.to_string());
        cfg.save_user_config()
            .map_err(|e| format!("failed to save {}: {e}", Self::user_config_path().display()))
    }

    /// Serialize to a user-facing TOML document (skips empty maps / Nones via serde).
    pub fn to_toml_string(&self) -> String {
        let mut out = String::from("# rs-agent user config (written by wizard / save)\n\n");
        if let Ok(s) = toml::to_string_pretty(self) {
            out.push_str(&s);
        }
        out
    }

    /// Resolve a model alias (e.g. "fast" -> "claude-..."). Returns the
    /// input unchanged if it isn't a known alias.
    pub fn resolve_model_alias(&self, model: &str) -> String {
        self.model_aliases
            .get(model)
            .cloned()
            .unwrap_or_else(|| model.to_string())
    }
}

/// Directory used for all rs-agent user state: `~/.rs-agent`.
pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".rs-agent")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_empty() {
        let cfg = Config::default();
        assert!(cfg.provider.is_none());
        assert!(cfg.model.is_none());
        assert!(cfg.model_aliases.is_empty());
    }

    #[test]
    fn parses_snake_case_toml() {
        let toml_str = r#"
            provider = "anthropic"
            model = "claude-sonnet-4-20250514"
            approve = true
            auto_mode = true
            rlm_depth = 3
            thinking_budget = 5000
            max_iterations = 50
            timeout = 120
            base_url = "https://example.com"
            disable_mouse = true
            theme = "light"

            [model_aliases]
            fast = "claude-haiku-4-20250514"
        "#;
        let cfg = Config::parse_str(toml_str).expect("should parse");
        assert_eq!(cfg.provider.as_deref(), Some("anthropic"));
        assert_eq!(cfg.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(cfg.approve, Some(true));
        assert_eq!(cfg.auto_mode, Some(true));
        assert_eq!(cfg.rlm_depth, Some(3));
        assert_eq!(cfg.thinking_budget, Some(5000));
        assert_eq!(cfg.max_iterations, Some(50));
        assert_eq!(cfg.timeout, Some(120));
        assert_eq!(cfg.base_url.as_deref(), Some("https://example.com"));
        assert_eq!(cfg.disable_mouse, Some(true));
        assert_eq!(cfg.theme.as_deref(), Some("light"));
        assert_eq!(
            cfg.model_aliases.get("fast").map(|s| s.as_str()),
            Some("claude-haiku-4-20250514")
        );
    }

    #[test]
    fn parses_partial_toml_leaving_rest_none() {
        let cfg = Config::parse_str(r#"provider = "openai""#).expect("should parse");
        assert_eq!(cfg.provider.as_deref(), Some("openai"));
        assert!(cfg.model.is_none());
        assert!(cfg.timeout.is_none());
    }

    #[test]
    fn parses_empty_toml() {
        let cfg = Config::parse_str("").expect("should parse");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn merge_overrides_only_set_fields() {
        let mut base = Config {
            provider: Some("anthropic".to_string()),
            model: Some("claude-sonnet-4-20250514".to_string()),
            timeout: Some(300),
            ..Default::default()
        };
        let override_cfg = Config {
            model: Some("claude-opus-4-20250514".to_string()),
            approve: Some(true),
            ..Default::default()
        };
        base.merge(override_cfg);

        // Overridden.
        assert_eq!(base.model.as_deref(), Some("claude-opus-4-20250514"));
        assert_eq!(base.approve, Some(true));
        // Untouched fields survive.
        assert_eq!(base.provider.as_deref(), Some("anthropic"));
        assert_eq!(base.timeout, Some(300));
    }

    #[test]
    fn merge_combines_model_aliases_with_later_winning_on_conflict() {
        let mut base = Config::default();
        base.model_aliases
            .insert("fast".to_string(), "model-a".to_string());
        base.model_aliases
            .insert("smart".to_string(), "model-b".to_string());

        let mut override_cfg = Config::default();
        override_cfg
            .model_aliases
            .insert("fast".to_string(), "model-a2".to_string());
        override_cfg
            .model_aliases
            .insert("cheap".to_string(), "model-c".to_string());

        base.merge(override_cfg);

        assert_eq!(
            base.model_aliases.get("fast").map(|s| s.as_str()),
            Some("model-a2")
        );
        assert_eq!(
            base.model_aliases.get("smart").map(|s| s.as_str()),
            Some("model-b")
        );
        assert_eq!(
            base.model_aliases.get("cheap").map(|s| s.as_str()),
            Some("model-c")
        );
    }

    #[test]
    fn merge_precedence_user_then_project_then_local_matches_load_order() {
        // Simulates the three layers `load()` merges, in order.
        let user = Config::parse_str(
            r#"
                provider = "anthropic"
                model = "user-model"
                timeout = 300
            "#,
        )
        .unwrap();
        let project_settings = Config::parse_str(
            r#"
                model = "project-model"
            "#,
        )
        .unwrap();
        let local = Config::parse_str(
            r#"
                model = "local-model"
                approve = true
            "#,
        )
        .unwrap();

        let mut merged = Config::default();
        merged.merge(user);
        merged.merge(project_settings);
        merged.merge(local);

        assert_eq!(merged.provider.as_deref(), Some("anthropic"));
        assert_eq!(merged.model.as_deref(), Some("local-model"));
        assert_eq!(merged.timeout, Some(300));
        assert_eq!(merged.approve, Some(true));
    }

    #[test]
    fn resolve_model_alias_returns_mapped_value() {
        let mut cfg = Config::default();
        cfg.model_aliases
            .insert("fast".to_string(), "claude-haiku-4-20250514".to_string());

        assert_eq!(cfg.resolve_model_alias("fast"), "claude-haiku-4-20250514");
    }

    #[test]
    fn resolve_model_alias_passes_through_unknown_names() {
        let cfg = Config::default();
        assert_eq!(
            cfg.resolve_model_alias("claude-sonnet-4-20250514"),
            "claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn parses_keybindings_table() {
        let cfg = Config::parse_str(
            r#"
            [keybindings]
            insert = "j"
            quit = "x"
            "#,
        )
        .expect("parse");
        assert_eq!(cfg.keybindings.get("insert").map(|s| s.as_str()), Some("j"));
        assert_eq!(cfg.keybindings.get("quit").map(|s| s.as_str()), Some("x"));
    }

    #[test]
    fn to_toml_string_roundtrips_provider() {
        let cfg = Config {
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet-4-20250514".into()),
            theme: Some("forest".into()),
            ..Default::default()
        };
        let s = cfg.to_toml_string();
        let parsed = Config::parse_str(&s).expect("parse saved");
        assert_eq!(parsed.provider.as_deref(), Some("anthropic"));
        assert_eq!(parsed.theme.as_deref(), Some("forest"));
    }

    #[test]
    fn persist_last_selection_updates_user_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
provider = "anthropic"
model = "old-model"
theme = "dark"

[model_aliases]
fast = "haiku"
"#,
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.merge_from_file(&path);
        cfg.provider = Some("openrouter".into());
        cfg.model = Some("openrouter/auto".into());
        let body = cfg.to_toml_string();
        std::fs::write(&path, body).unwrap();

        let mut reloaded = Config::default();
        reloaded.merge_from_file(&path);
        assert_eq!(reloaded.provider.as_deref(), Some("openrouter"));
        assert_eq!(reloaded.model.as_deref(), Some("openrouter/auto"));
        assert_eq!(reloaded.theme.as_deref(), Some("dark"));
        assert_eq!(
            reloaded.model_aliases.get("fast").map(|s| s.as_str()),
            Some("haiku")
        );
    }

    #[test]
    fn config_dir_ends_with_dot_rs_agent() {
        let dir = config_dir();
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some(".rs-agent"));
    }

    #[test]
    fn load_from_missing_files_returns_default() {
        // merge_from_file on a nonexistent path is a no-op.
        let mut cfg = Config::default();
        cfg.merge_from_file(Path::new("/definitely/does/not/exist/config.toml"));
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn parses_mcp_servers() {
        let cfg = Config::parse_str(
            r#"
[[mcp.servers]]
name = "fs"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.mcp.servers.len(), 1);
        assert_eq!(cfg.mcp.servers[0].name, "fs");
        assert_eq!(cfg.mcp.servers[0].command, "npx");
        assert_eq!(cfg.mcp.servers[0].args.len(), 3);
    }
}
