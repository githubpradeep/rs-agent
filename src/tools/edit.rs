use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

#[derive(Deserialize)]
pub struct EditArgs {
    pub file_path: String,
    #[serde(default)]
    pub old_string: Option<String>,
    #[serde(default)]
    pub new_string: Option<String>,
    /// When true, replace every non-overlapping occurrence (skips uniqueness check).
    #[serde(default)]
    pub replace_all: bool,
    /// Multi-hunk edits applied in order (Wave A apply_patch-style).
    #[serde(default)]
    pub edits: Option<Vec<EditHunk>>,
}

#[derive(Deserialize)]
pub struct EditHunk {
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

pub struct EditTool;

#[async_trait]
impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a file using exact text replacement. Finds old_string and replaces it with new_string. \
         old_string must match uniquely unless replace_all=true. \
         For several changes in one file, pass edits=[{old_string,new_string},...] instead."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace (single-hunk mode)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text (single-hunk mode)"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence of old_string (default false)"
                },
                "edits": {
                    "type": "array",
                    "description": "Multi-hunk edits applied in order. Each item: old_string, new_string, optional replace_all.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": {"type": "string"},
                            "new_string": {"type": "string"},
                            "replace_all": {"type": "boolean"}
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["file_path"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let args = crate::tools::normalize_file_tool_args(args);
        let parsed: EditArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid args: {e}. Expected file_path + (old_string/new_string or edits[]). Got keys: {}",
                    args.as_object()
                        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_else(|| "(not an object)".into())
                ))
            }
        };

        let content = match fs::read_to_string(&parsed.file_path).await {
            Ok(c) => c,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Failed to read {}: {}",
                    parsed.file_path, e
                ))
            }
        };

        let hunks: Vec<EditHunk> = if let Some(edits) = parsed.edits {
            if edits.is_empty() {
                return ToolExecuteResult::error(
                    "edits array is empty. Pass at least one {old_string, new_string}.",
                );
            }
            edits
        } else {
            let old = match parsed.old_string {
                Some(s) if !s.is_empty() => s,
                _ => {
                    return ToolExecuteResult::error(
                        "Missing old_string (or provide non-empty edits[]).",
                    )
                }
            };
            let new = match parsed.new_string {
                Some(s) => s,
                None => {
                    return ToolExecuteResult::error("Missing new_string (or provide edits[]).")
                }
            };
            vec![EditHunk {
                old_string: old,
                new_string: new,
                replace_all: parsed.replace_all,
            }]
        };

        let original = content.clone();
        let mut content = content;
        let mut total_replacements = 0usize;
        for (i, hunk) in hunks.iter().enumerate() {
            match apply_hunk(&content, hunk) {
                Ok((next, n)) => {
                    content = next;
                    total_replacements += n;
                }
                Err(msg) => {
                    return ToolExecuteResult::error(format!(
                        "edit hunk {}/{} failed in {}:\n{}",
                        i + 1,
                        hunks.len(),
                        parsed.file_path,
                        msg
                    ));
                }
            }
        }

        let diff = crate::tools::diffutil::unified_diff(&parsed.file_path, &original, &content);

        match fs::write(&parsed.file_path, &content).await {
            Ok(_) => ToolExecuteResult::ok(format!(
                "Successfully edited {} ({} replacement{}, {} hunk{})\n\n{}",
                parsed.file_path,
                total_replacements,
                if total_replacements == 1 { "" } else { "s" },
                hunks.len(),
                if hunks.len() == 1 { "" } else { "s" },
                diff,
            )),
            Err(e) => ToolExecuteResult::error(format!(
                "Failed to write {}: {}",
                parsed.file_path, e
            )),
        }
    }
}

/// Dry-run an edit against the current file contents for permission previews.
pub fn preview_edit_diff(args: &Value) -> Option<String> {
    let args = crate::tools::normalize_file_tool_args(args.clone());
    let parsed: EditArgs = serde_json::from_value(args).ok()?;
    let original = std::fs::read_to_string(&parsed.file_path).ok()?;
    let hunks: Vec<EditHunk> = if let Some(edits) = parsed.edits {
        if edits.is_empty() {
            return None;
        }
        edits
    } else {
        let old = parsed.old_string.filter(|s| !s.is_empty())?;
        let new = parsed.new_string?;
        vec![EditHunk {
            old_string: old,
            new_string: new,
            replace_all: parsed.replace_all,
        }]
    };
    let mut content = original.clone();
    for hunk in &hunks {
        match apply_hunk(&content, hunk) {
            Ok((next, _)) => content = next,
            Err(_) => return None,
        }
    }
    Some(crate::tools::diffutil::unified_diff_capped(
        &parsed.file_path,
        &original,
        &content,
        40,
    ))
}

fn apply_hunk(content: &str, hunk: &EditHunk) -> Result<(String, usize), String> {
    if hunk.old_string.is_empty() {
        return Err("old_string must not be empty".into());
    }
    if hunk.old_string == hunk.new_string {
        return Err("old_string and new_string are identical — nothing to change".into());
    }

    let count = content.matches(&hunk.old_string).count();
    if count == 0 {
        return Err(format_not_found(content, &hunk.old_string));
    }
    if count > 1 && !hunk.replace_all {
        return Err(format!(
            "Found {count} occurrences of old_string. Provide more surrounding context to uniquely \
             identify the match, or set replace_all=true to replace every occurrence."
        ));
    }

    let new_content = if hunk.replace_all {
        content.replace(&hunk.old_string, &hunk.new_string)
    } else {
        content.replacen(&hunk.old_string, &hunk.new_string, 1)
    };
    Ok((new_content, if hunk.replace_all { count } else { 1 }))
}

fn format_not_found(content: &str, old_string: &str) -> String {
    let mut msg = String::from(
        "Could not find old_string (exact match).\n\
         Tips: copy text from a recent read; include more surrounding lines; check whitespace.",
    );

    if let Some(ws) = whitespace_flexible_hint(content, old_string) {
        msg.push_str("\n\n");
        msg.push_str(&ws);
    }

    let suggestions = fuzzy_line_suggestions(content, old_string, 3);
    if !suggestions.is_empty() {
        msg.push_str("\n\nClosest regions in the file (use these as old_string):\n");
        for (rank, s) in suggestions.iter().enumerate() {
            msg.push_str(&format!(
                "--- candidate {} (score {:.0}%) ---\n{}\n",
                rank + 1,
                s.score * 100.0,
                s.snippet
            ));
        }
    } else {
        msg.push_str(&format!(
            "\n\nfile head (first 1500 chars):\n{}",
            content.chars().take(1500).collect::<String>()
        ));
    }
    msg
}

/// If collapsing whitespace makes old_string appear, tell the model.
fn whitespace_flexible_hint(content: &str, old_string: &str) -> Option<String> {
    let needle = collapse_ws(old_string);
    if needle.is_empty() {
        return None;
    }
    let hay = collapse_ws(content);
    if !hay.contains(&needle) {
        return None;
    }
    Some(
        "Note: a whitespace-insensitive match EXISTS. Your old_string likely has different \
         spaces/tabs/newlines than the file. Re-read the exact lines and copy them verbatim."
            .into(),
    )
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct FuzzyHit {
    score: f64,
    snippet: String,
}

/// Find line-windows in `content` most similar to `old_string`.
fn fuzzy_line_suggestions(content: &str, old_string: &str, limit: usize) -> Vec<FuzzyHit> {
    let needle_lines: Vec<&str> = old_string.lines().collect();
    let window = needle_lines.len().max(1);
    let file_lines: Vec<&str> = content.lines().collect();
    if file_lines.is_empty() {
        return Vec::new();
    }

    let needle_norm = normalize_for_score(old_string);
    let mut scored: Vec<(f64, usize)> = Vec::new();

    for start in 0..file_lines.len() {
        let end = (start + window).min(file_lines.len());
        if end <= start {
            continue;
        }
        let chunk = file_lines[start..end].join("\n");
        let score = similarity(&needle_norm, &normalize_for_score(&chunk));
        if score >= 0.45 {
            scored.push((score, start));
        }
        // Also try slightly larger windows for multi-line drift
        if window > 1 && end + 1 <= file_lines.len() {
            let chunk2 = file_lines[start..end + 1].join("\n");
            let score2 = similarity(&needle_norm, &normalize_for_score(&chunk2));
            if score2 >= 0.45 {
                scored.push((score2, start));
            }
        }
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.dedup_by(|a, b| a.1 == b.1);

    let mut out = Vec::new();
    for (score, start) in scored.into_iter().take(limit) {
        let end = (start + window + 1).min(file_lines.len());
        let ctx_start = start.saturating_sub(1);
        let snippet = file_lines[ctx_start..end].join("\n");
        out.push(FuzzyHit { score, snippet });
    }
    out
}

fn normalize_for_score(s: &str) -> String {
    collapse_ws(s).to_lowercase()
}

/// Dice coefficient on character bigrams — cheap fuzzy similarity in [0,1].
fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let bi = |s: &str| -> std::collections::HashMap<(char, char), u32> {
        let chars: Vec<char> = s.chars().collect();
        let mut m = std::collections::HashMap::new();
        for w in chars.windows(2) {
            *m.entry((w[0], w[1])).or_default() += 1;
        }
        m
    };
    let aa = bi(a);
    let bb = bi(b);
    if aa.is_empty() || bb.is_empty() {
        // fall back to char overlap for short strings
        let set_a: std::collections::HashSet<char> = a.chars().collect();
        let set_b: std::collections::HashSet<char> = b.chars().collect();
        let inter = set_a.intersection(&set_b).count() as f64;
        let union = set_a.union(&set_b).count() as f64;
        return if union == 0.0 { 0.0 } else { inter / union };
    }
    let mut inter = 0u32;
    for (k, ca) in &aa {
        if let Some(cb) = bb.get(k) {
            inter += (*ca).min(*cb);
        }
    }
    let total: u32 = aa.values().sum::<u32>() + bb.values().sum::<u32>();
    if total == 0 {
        0.0
    } else {
        (2.0 * inter as f64) / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn replace_all_replaces_every_occurrence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.txt");
        std::fs::write(&path, "aa x aa y aa").unwrap();
        let tool = EditTool;
        let res = tool
            .execute(
                "1",
                serde_json::json!({
                    "file_path": path.to_str().unwrap(),
                    "old_string": "aa",
                    "new_string": "bb",
                    "replace_all": true
                }),
            )
            .await;
        assert!(!res.is_error, "{}", res.content);
        let out = std::fs::read_to_string(&path).unwrap();
        assert_eq!(out, "bb x bb y bb");
    }

    #[tokio::test]
    async fn multi_hunk_edits_apply_in_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.txt");
        std::fs::write(&path, "hello world\nfoo bar\n").unwrap();
        let tool = EditTool;
        let res = tool
            .execute(
                "1",
                serde_json::json!({
                    "file_path": path.to_str().unwrap(),
                    "edits": [
                        {"old_string": "hello", "new_string": "hi"},
                        {"old_string": "foo", "new_string": "baz"}
                    ]
                }),
            )
            .await;
        assert!(!res.is_error, "{}", res.content);
        let out = std::fs::read_to_string(&path).unwrap();
        assert_eq!(out, "hi world\nbaz bar\n");
    }

    #[tokio::test]
    async fn miss_includes_fuzzy_hint() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.txt");
        std::fs::write(&path, "fn greet(name: &str) {\n    println!(\"hi {}\", name);\n}\n").unwrap();
        let tool = EditTool;
        let res = tool
            .execute(
                "1",
                serde_json::json!({
                    "file_path": path.to_str().unwrap(),
                    "old_string": "fn greet(name: String) {",
                    "new_string": "fn greet(name: &str) {"
                }),
            )
            .await;
        assert!(res.is_error);
        assert!(
            res.content.contains("Closest regions") || res.content.contains("whitespace-insensitive"),
            "{}",
            res.content
        );
    }

    #[test]
    fn similarity_ranks_close_strings_higher() {
        let a = similarity("hello world", "hello world");
        let b = similarity("hello world", "hello word");
        let c = similarity("hello world", "zzzzzzzzzz");
        assert!(a > b && b > c);
    }
}
