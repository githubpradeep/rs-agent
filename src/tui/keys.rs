//! Configurable action keybindings with hardcoded fallbacks.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

/// Default action → single-character (or named) bindings.
pub fn default_keybindings() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("insert".into(), "i".into());
    m.insert("quit".into(), "q".into());
    m.insert("toggle_thinking".into(), "t".into());
    m.insert("jump_bottom".into(), "G".into());
    m.insert("expand_tool".into(), "e".into());
    m.insert("toggle_tree".into(), "T".into());
    m.insert("toggle_sessions".into(), "s".into());
    m.insert("perm_once".into(), "a".into());
    // Keep off `t` — that toggles thinking in normal mode.
    m.insert("perm_always".into(), "A".into());
    m.insert("perm_path".into(), "p".into());
    m.insert("perm_deny".into(), "d".into());
    m
}

/// Merge user overrides on top of defaults.
pub fn merge_keybindings(user: &HashMap<String, String>) -> HashMap<String, String> {
    let mut m = default_keybindings();
    for (k, v) in user {
        if !v.is_empty() {
            m.insert(k.clone(), v.clone());
        }
    }
    m
}

pub struct KeyMap {
    map: HashMap<String, String>,
}

impl KeyMap {
    pub fn new(map: HashMap<String, String>) -> Self {
        Self { map }
    }

    pub fn binding(&self, action: &str) -> &str {
        self.map
            .get(action)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// True if this key event matches the configured binding for `action`
    /// (char keys only; modifiers must be empty except Shift for uppercase).
    pub fn matches(&self, action: &str, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return false;
        }
        let Some(binding) = self.map.get(action) else {
            return false;
        };
        match key.code {
            KeyCode::Char(c) => {
                if binding.len() == 1 {
                    binding.chars().next() == Some(c)
                } else {
                    false
                }
            }
            KeyCode::Enter => binding.eq_ignore_ascii_case("enter"),
            KeyCode::Esc => binding.eq_ignore_ascii_case("esc"),
            KeyCode::Tab => binding.eq_ignore_ascii_case("tab"),
            _ => false,
        }
    }

    pub fn hint_line(&self) -> String {
        format!(
            "[{}]=insert [{}]=quit [{}]=think [{}]=tool [{}]=tree [{}]=sessions",
            self.binding("insert"),
            self.binding("quit"),
            self.binding("toggle_thinking"),
            self.binding("expand_tool"),
            self.binding("toggle_tree"),
            self.binding("toggle_sessions"),
        )
    }
}
