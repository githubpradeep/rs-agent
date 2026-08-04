use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Mutex;
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

static NOOP_HISTORY: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn noop_fingerprint(path: &str, hunks: &[EditHunk]) -> String {
    let mut s = path.to_string();
    for h in hunks {
        s.push('|');
        s.push_str(&h.old_string);
        s.push('>');
        s.push_str(&h.new_string);
        s.push_str(&format!(":{}", h.replace_all));
    }
    s
}

fn record_noop_and_check(fp: &str) -> Option<String> {
    let mut g = NOOP_HISTORY.lock().ok()?;
    g.push_back(fp.to_string());
    while g.len() > 8 {
        g.pop_front();
    }
    let count = g.iter().rev().take_while(|x| *x == fp).count();
    if count >= 3 {
        Some(
            "No-op / identical edit payload repeated 3 times (noop loop).\n\
             Re-read the file and change approach — do not retry the same edit."
                .into(),
        )
    } else {
        None
    }
}

fn clear_noop_on_success() {
    if let Ok(mut g) = NOOP_HISTORY.lock() {
        g.clear();
    }
}

#[async_trait]
impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a file using text replacement. Finds old_string and replaces it with new_string. \
         old_string must match uniquely unless replace_all=true. Soft-matches whitespace/indent drift. \
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

        let path = parsed.file_path.clone();
        crate::tools::mutation_queue::with_file_lock(&path, || async {
            let _ = crate::tools::turn_snapshot::track(&path);

            let raw = match fs::read(&path).await {
                Ok(b) => b,
                Err(e) => {
                    return ToolExecuteResult::error(format!("Failed to read {}: {}", path, e));
                }
            };
            let (bom, content_body) = strip_bom(&raw);
            let (newline, content) = normalize_newlines(&content_body);

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

            let fp = noop_fingerprint(&path, &hunks);
            let original = content.clone();
            let mut content = content;
            let mut total_replacements = 0usize;
            let mut strategies_used = Vec::new();

            for (i, hunk) in hunks.iter().enumerate() {
                match apply_hunk_soft(&content, hunk) {
                    Ok((next, n, strat)) => {
                        content = next;
                        total_replacements += n;
                        strategies_used.push(strat);
                    }
                    Err(msg) => {
                        return ToolExecuteResult::error(format!(
                            "edit hunk {}/{} failed in {}:\n{}",
                            i + 1,
                            hunks.len(),
                            path,
                            msg
                        ));
                    }
                }
            }

            if content == original {
                if let Some(err) = record_noop_and_check(&fp) {
                    return ToolExecuteResult::error(err);
                }
                return ToolExecuteResult::error(
                    "Edit applied but file content is unchanged (noop). Re-read and adjust old_string/new_string.",
                );
            }
            clear_noop_on_success();

            let diff = crate::tools::diffutil::unified_diff(&path, &original, &content);
            let out_bytes = encode_with_bom_newline(&content, bom, newline);

            match crate::tools::write::atomic_write_bytes(&path, &out_bytes).await {
                Ok(_) => {
                    let soft_note = if strategies_used.iter().any(|s| *s != "exact") {
                        format!(
                            " (match strategy: {})",
                            strategies_used.join(",")
                        )
                    } else {
                        String::new()
                    };
                    let body = format!(
                        "Successfully edited {} ({} replacement{}, {} hunk{}){soft_note}\n\n{}",
                        path,
                        total_replacements,
                        if total_replacements == 1 { "" } else { "s" },
                        hunks.len(),
                        if hunks.len() == 1 { "" } else { "s" },
                        diff,
                    );
                    let body = crate::tools::post_mutation::after_mutation(&path, body).await;
                    ToolExecuteResult::ok(body)
                }
                Err(e) => ToolExecuteResult::error(format!("Failed to write {}: {}", path, e)),
            }
        })
        .await
    }
}

/// Dry-run an edit against the current file contents for permission previews.
pub fn preview_edit_diff(args: &Value) -> Option<String> {
    let args = crate::tools::normalize_file_tool_args(args.clone());
    let parsed: EditArgs = serde_json::from_value(args).ok()?;
    let raw = std::fs::read(&parsed.file_path).ok()?;
    let (_bom, body) = strip_bom(&raw);
    let (_nl, original) = normalize_newlines(&body);
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
        match apply_hunk_soft(&content, hunk) {
            Ok((next, _, _)) => content = next,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bom {
    None,
    Utf8,
}

fn strip_bom(raw: &[u8]) -> (Bom, String) {
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (
            Bom::Utf8,
            String::from_utf8_lossy(&raw[3..]).into_owned(),
        )
    } else {
        (Bom::None, String::from_utf8_lossy(raw).into_owned())
    }
}

fn normalize_newlines(s: &str) -> (&'static str, String) {
    if s.contains("\r\n") {
        ("\r\n", s.replace("\r\n", "\n"))
    } else {
        ("\n", s.to_string())
    }
}

fn encode_with_bom_newline(content: &str, bom: Bom, newline: &str) -> Vec<u8> {
    let body = if newline == "\r\n" {
        content.replace('\n', "\r\n")
    } else {
        content.to_string()
    };
    let mut out = Vec::new();
    if bom == Bom::Utf8 {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

/// Progressive soft apply: exact → line-trim → whitespace-normalize → indent-flexible.
fn apply_hunk_soft(content: &str, hunk: &EditHunk) -> Result<(String, usize, &'static str), String> {
    if hunk.old_string.is_empty() {
        return Err("old_string must not be empty".into());
    }
    if hunk.old_string == hunk.new_string {
        return Err("old_string and new_string are identical — nothing to change".into());
    }

    // 1. Exact
    if let Ok((next, n)) = apply_exact(content, &hunk.old_string, &hunk.new_string, hunk.replace_all)
    {
        return Ok((next, n, "exact"));
    }

    // 2. Line-trimmed (trim_end per line)
    if let Some((old_m, new_m)) = line_trim_pair(content, &hunk.old_string, &hunk.new_string) {
        if let Ok((next, n)) = apply_exact(content, &old_m, &new_m, hunk.replace_all) {
            return Ok((next, n, "line_trim"));
        }
    }

    // 3. Whitespace-normalized
    if let Some((next, n)) =
        apply_whitespace_normalized(content, &hunk.old_string, &hunk.new_string, hunk.replace_all)
    {
        return Ok((next, n, "whitespace"));
    }

    // 4. Indentation-flexible
    if let Some((next, n)) =
        apply_indent_flexible(content, &hunk.old_string, &hunk.new_string, hunk.replace_all)
    {
        return Ok((next, n, "indent"));
    }

    Err(format_not_found(content, &hunk.old_string))
}

fn apply_exact(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
    let count = content.matches(old).count();
    if count == 0 {
        return Err("no exact match".into());
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "Found {count} occurrences of old_string. Provide more surrounding context to uniquely \
             identify the match, or set replace_all=true to replace every occurrence."
        ));
    }
    let new_content = if replace_all {
        content.replace(old, new)
    } else {
        content.replacen(old, new, 1)
    };
    Ok((new_content, if replace_all { count } else { 1 }))
}

fn line_trim_pair(content: &str, old: &str, new: &str) -> Option<(String, String)> {
    let old_lines: Vec<&str> = old.lines().collect();
    if old_lines.is_empty() {
        return None;
    }
    let file_lines: Vec<&str> = content.lines().collect();
    let window = old_lines.len();
    for start in 0..=file_lines.len().saturating_sub(window) {
        let chunk = &file_lines[start..start + window];
        let matches = chunk
            .iter()
            .zip(old_lines.iter())
            .all(|(a, b)| a.trim_end() == b.trim_end());
        if matches {
            let found = chunk.join("\n");
            // Preserve whether old ended with newline by not forcing it.
            let new_adj = if old.ends_with('\n') && !new.ends_with('\n') {
                format!("{new}\n")
            } else {
                new.to_string()
            };
            // Map new lines onto file indentation of first matched line if needed — keep as-is.
            return Some((found, new_adj));
        }
    }
    None
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn apply_whitespace_normalized(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Option<(String, usize)> {
    let needle = collapse_ws(old);
    if needle.is_empty() {
        return None;
    }
    // Find regions by scanning line windows.
    let old_lines: Vec<&str> = old.lines().collect();
    let window = old_lines.len().max(1);
    let file_lines: Vec<&str> = content.lines().collect();
    let mut starts = Vec::new();
    for start in 0..=file_lines.len().saturating_sub(window) {
        let chunk = file_lines[start..start + window].join("\n");
        if collapse_ws(&chunk) == needle {
            starts.push(start);
        }
    }
    if starts.is_empty() {
        // try larger windows
        for start in 0..file_lines.len() {
            for end in (start + 1)..=file_lines.len().min(start + window + 3) {
                let chunk = file_lines[start..end].join("\n");
                if collapse_ws(&chunk) == needle {
                    starts.push(start);
                    break;
                }
            }
        }
        starts.sort();
        starts.dedup();
    }
    if starts.is_empty() {
        return None;
    }
    if starts.len() > 1 && !replace_all {
        return None;
    }

    let mut result_lines: Vec<String> = file_lines.iter().map(|s| s.to_string()).collect();
    let new_lines: Vec<String> = new.lines().map(|s| s.to_string()).collect();
    // Apply from bottom so indices stay valid
    let mut applied = 0usize;
    for start in starts.into_iter().rev() {
        let end = (start + window).min(result_lines.len());
        // Expand end if collapse matched larger region — recompute
        let mut end2 = end;
        while end2 < result_lines.len()
            && collapse_ws(&result_lines[start..end2].join("\n")) != needle
        {
            end2 += 1;
            if end2 - start > window + 5 {
                break;
            }
        }
        if collapse_ws(&result_lines[start..end2.min(result_lines.len())].join("\n")) != needle {
            // use window
            result_lines.splice(start..end, new_lines.iter().cloned());
        } else {
            result_lines.splice(start..end2, new_lines.iter().cloned());
        }
        applied += 1;
        if !replace_all {
            break;
        }
    }
    let mut out = result_lines.join("\n");
    if content.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    Some((out, applied))
}

fn apply_indent_flexible(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Option<(String, usize)> {
    let strip = |s: &str| -> String {
        s.lines()
            .map(|l| l.trim_start())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let needle = strip(old);
    if needle.is_empty() {
        return None;
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let window = old_lines.len().max(1);
    let file_lines: Vec<&str> = content.lines().collect();
    let mut starts = Vec::new();
    for start in 0..=file_lines.len().saturating_sub(window) {
        let chunk = file_lines[start..start + window].join("\n");
        if strip(&chunk) == needle {
            starts.push(start);
        }
    }
    if starts.is_empty() || (starts.len() > 1 && !replace_all) {
        return None;
    }

    let mut result_lines: Vec<String> = file_lines.iter().map(|s| s.to_string()).collect();
    let mut applied = 0usize;
    for start in starts.into_iter().rev() {
        let end = (start + window).min(result_lines.len());
        let indent = leading_ws(result_lines.get(start).map(|s| s.as_str()).unwrap_or(""));
        let new_block: Vec<String> = new
            .lines()
            .enumerate()
            .map(|(i, l)| {
                if i == 0 {
                    format!("{indent}{}", l.trim_start())
                } else {
                    // preserve relative indent from new, rebased onto file indent of first line
                    let rel = leading_ws(l);
                    let trimmed = l.trim_start();
                    format!("{indent}{rel}{trimmed}")
                }
            })
            .collect();
        result_lines.splice(start..end, new_block);
        applied += 1;
        if !replace_all {
            break;
        }
    }
    let mut out = result_lines.join("\n");
    if content.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    Some((out, applied))
}

fn leading_ws(s: &str) -> String {
    s.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

fn format_not_found(content: &str, old_string: &str) -> String {
    let mut msg = String::from(
        "Could not find old_string (tried exact, line-trim, whitespace, and indent-flexible match).\n\
         Tips: copy text from a recent read; include more surrounding lines.",
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
        "Note: a whitespace-insensitive match EXISTS but soft-apply could not uniquely bind it. \
         Re-read the exact lines and copy them verbatim, or set replace_all=true."
            .into(),
    )
}

struct FuzzyHit {
    score: f64,
    snippet: String,
}

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
    async fn whitespace_drift_applies() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.txt");
        // File has trailing spaces; model old_string does not.
        std::fs::write(&path, "hello world   \nfoo\n").unwrap();
        let tool = EditTool;
        let res = tool
            .execute(
                "1",
                serde_json::json!({
                    "file_path": path.to_str().unwrap(),
                    "old_string": "hello world\n",
                    "new_string": "hi world\n"
                }),
            )
            .await;
        assert!(!res.is_error, "{}", res.content);
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.starts_with("hi world"), "{out}");
    }

    #[tokio::test]
    async fn miss_includes_fuzzy_hint() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.txt");
        std::fs::write(
            &path,
            "fn greet(name: &str) {\n    println!(\"hi {}\", name);\n}\n",
        )
        .unwrap();
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
            res.content.contains("Closest regions")
                || res.content.contains("whitespace-insensitive"),
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
