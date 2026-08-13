//! Collect durable pins from conversation for compaction (paths + failed edits).

use crate::ai::types::{ContentType, Message, Role};
use crate::permission::extract_tool_path;
use std::collections::BTreeSet;

const PINNED_MARKER: &str = "## Pinned paths (harness)";
const FAILED_MARKER: &str = "## Failed edits (harness)";

#[derive(Debug, Default, Clone)]
pub struct CompactPins {
    pub paths: BTreeSet<String>,
    pub failed_edits: Vec<String>,
}

impl CompactPins {
    pub fn merge(&mut self, other: CompactPins) {
        self.paths.extend(other.paths);
        for f in other.failed_edits {
            if !self.failed_edits.contains(&f) {
                self.failed_edits.push(f);
            }
        }
        // Cap failed notes
        if self.failed_edits.len() > 20 {
            self.failed_edits = self.failed_edits.split_off(self.failed_edits.len() - 20);
        }
        // Cap paths
        while self.paths.len() > 40 {
            if let Some(first) = self.paths.iter().next().cloned() {
                self.paths.remove(&first);
            } else {
                break;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.failed_edits.is_empty()
    }

    pub fn render_block(&self) -> String {
        let mut out = String::new();
        if !self.paths.is_empty() {
            out.push_str(PINNED_MARKER);
            out.push('\n');
            for p in &self.paths {
                out.push_str("- ");
                out.push_str(p);
                out.push('\n');
            }
        }
        if !self.failed_edits.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(FAILED_MARKER);
            out.push('\n');
            for f in &self.failed_edits {
                out.push_str("- ");
                out.push_str(f);
                out.push('\n');
            }
        }
        out
    }

    /// Parse pins previously embedded in a compaction summary.
    pub fn from_summary_text(text: &str) -> CompactPins {
        let mut pins = CompactPins::default();
        let mut section: Option<&str> = None;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with(PINNED_MARKER) || t == "## Relevant Files" {
                section = Some("paths");
                continue;
            }
            if t.starts_with(FAILED_MARKER) {
                section = Some("failed");
                continue;
            }
            if t.starts_with("## ") {
                section = None;
                continue;
            }
            if section == Some("paths") {
                if let Some(p) = t.strip_prefix("- ") {
                    let p = p.trim();
                    if !p.is_empty() && p != "..." {
                        pins.paths.insert(p.to_string());
                    }
                }
            } else if section == Some("failed") {
                if let Some(f) = t.strip_prefix("- ") {
                    let f = f.trim();
                    if !f.is_empty() {
                        pins.failed_edits.push(f.to_string());
                    }
                }
            }
        }
        pins
    }
}

/// Scan messages for file paths and failed edit/write tool results.
pub fn collect_pins_from_messages(messages: &[Message]) -> CompactPins {
    let mut pins = CompactPins::default();
    // Map tool_use_id -> (name, path) for correlating errors
    let mut tool_meta: std::collections::HashMap<String, (String, Option<String>)> =
        std::collections::HashMap::new();

    for msg in messages {
        for c in &msg.content {
            match c.content_type {
                ContentType::ToolUse => {
                    let name = c.name.as_deref().unwrap_or("").to_string();
                    let path = c.input.as_ref().and_then(|v| {
                        extract_tool_path(&v.to_string()).or_else(|| {
                            v.get("file_path")
                                .or_else(|| v.get("path"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                        })
                    });
                    if let Some(ref p) = path {
                        pins.paths.insert(p.clone());
                    }
                    if let Some(id) = c.id.as_ref() {
                        tool_meta.insert(id.clone(), (name, path));
                    }
                }
                ContentType::ToolResult => {
                    let id = c.tool_use_id.as_deref().unwrap_or("");
                    if c.is_error {
                        if let Some((name, path)) = tool_meta.get(id) {
                            if matches!(name.as_str(), "edit" | "write" | "apply_patch") {
                                let snippet = c
                                    .text
                                    .as_deref()
                                    .unwrap_or("error")
                                    .lines()
                                    .next()
                                    .unwrap_or("error");
                                let snippet: String = snippet.chars().take(120).collect();
                                let note = match path {
                                    Some(p) => format!("{name} {p}: {snippet}"),
                                    None => format!("{name}: {snippet}"),
                                };
                                pins.failed_edits.push(note);
                            }
                        }
                    }
                }
                ContentType::Text => {
                    // Recover pins from prior compaction system messages
                    if msg.role == Role::System {
                        if let Some(t) = c.text.as_deref() {
                            if t.contains(PINNED_MARKER) || t.contains(FAILED_MARKER) {
                                pins.merge(CompactPins::from_summary_text(t));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    pins
}

/// Append harness pin blocks to a compaction summary (idempotent-ish).
pub fn append_pins_to_summary(summary: &str, pins: &CompactPins) -> String {
    if pins.is_empty() {
        return summary.to_string();
    }
    // Strip old harness pin sections so we rewrite fresh
    let cleaned = strip_harness_pin_sections(summary);
    let block = pins.render_block();
    if cleaned.trim().is_empty() {
        block
    } else {
        format!("{}\n\n{}", cleaned.trim_end(), block)
    }
}

fn strip_harness_pin_sections(text: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with(PINNED_MARKER) || t.starts_with(FAILED_MARKER) {
            skipping = true;
            continue;
        }
        if skipping && t.starts_with("## ") {
            skipping = false;
        }
        if skipping {
            // still in list under pin section
            if t.starts_with("- ") || t.is_empty() {
                continue;
            }
            skipping = false;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::Content;
    use serde_json::json;

    #[test]
    fn collects_paths_and_failed_edits() {
        let messages = vec![
            Message {
                role: Role::Assistant,
                content: vec![Content {
                    content_type: ContentType::ToolUse,
                    id: Some("1".into()),
                    name: Some("edit".into()),
                    input: Some(
                        json!({"file_path": "src/main.rs", "old_string": "a", "new_string": "b"}),
                    ),
                    ..Default::default()
                }],
            },
            Message {
                role: Role::User,
                content: vec![Content {
                    content_type: ContentType::ToolResult,
                    tool_use_id: Some("1".into()),
                    name: Some("edit".into()),
                    text: Some("Could not find old_string".into()),
                    is_error: true,
                    ..Default::default()
                }],
            },
        ];
        let pins = collect_pins_from_messages(&messages);
        assert!(pins.paths.contains("src/main.rs"));
        assert!(!pins.failed_edits.is_empty());
        assert!(pins.failed_edits[0].contains("edit"));
    }

    #[test]
    fn roundtrip_render_parse() {
        let mut pins = CompactPins::default();
        pins.paths.insert("/tmp/a.rs".into());
        pins.failed_edits.push("edit /tmp/a.rs: miss".into());
        let block = pins.render_block();
        let parsed = CompactPins::from_summary_text(&block);
        assert!(parsed.paths.contains("/tmp/a.rs"));
        assert_eq!(parsed.failed_edits.len(), 1);
    }

    #[test]
    fn append_replaces_old_harness_section() {
        let summary = "## Goal\nok\n\n## Pinned paths (harness)\n- /old\n";
        let mut pins = CompactPins::default();
        pins.paths.insert("/new".into());
        let out = append_pins_to_summary(summary, &pins);
        assert!(out.contains("/new"));
        assert!(!out.contains("/old"));
        assert!(out.contains("## Goal"));
    }
}
