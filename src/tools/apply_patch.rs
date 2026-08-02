//! Apply a unified diff patch to a file on disk.

use crate::agent::tool::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;

#[derive(Deserialize)]
struct ApplyPatchArgs {
    /// Unified diff text (--- / +++ / @@ hunks).
    #[serde(alias = "diff", alias = "patch_text", alias = "content")]
    patch: String,
    /// Optional explicit path; otherwise taken from +++ header.
    #[serde(default)]
    file_path: Option<String>,
}

pub struct ApplyPatchTool;

#[async_trait]
impl AgentTool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff patch to an existing file. Pass patch=\"\" with ---/+++ headers \
         and @@ hunks. Optional file_path overrides the path from the +++ line. \
         Prefer edit for small surgical replacements; use apply_patch for multi-hunk model-generated diffs."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Unified diff (--- a/path, +++ b/path, @@ hunks)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Optional path override if the diff header is relative/missing"
                }
            },
            "required": ["patch"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let args = normalize_patch_args(args);
        let parsed: ApplyPatchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid apply_patch args: {e}. Expected {{patch: \"...\", file_path?: \"...\"}}."
                ))
            }
        };
        if parsed.patch.trim().is_empty() {
            return ToolExecuteResult::error("patch must not be empty");
        }

        let (header_path, hunks) = match parse_unified_diff(&parsed.patch) {
            Ok(v) => v,
            Err(e) => return ToolExecuteResult::error(e),
        };
        let path = parsed
            .file_path
            .filter(|s| !s.trim().is_empty())
            .or(header_path)
            .unwrap_or_default();
        if path.is_empty() {
            return ToolExecuteResult::error(
                "Could not determine file path. Include +++ b/path in the diff or pass file_path.",
            );
        }

        crate::tools::mutation_queue::with_file_lock(&path, || async {
            let _ = crate::tools::turn_snapshot::track(&path);

            let original = match fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) => {
                    return ToolExecuteResult::error(format!("Failed to read {path}: {e}"))
                }
            };

            let updated = match apply_hunks(&original, &hunks) {
                Ok(c) => c,
                Err(e) => {
                    return ToolExecuteResult::error(format!("Patch failed for {path}:\n{e}"))
                }
            };

            let diff_preview = crate::tools::diffutil::unified_diff(&path, &original, &updated);
            match fs::write(&path, &updated).await {
                Ok(_) => {
                    let body = format!(
                        "Successfully applied patch to {} ({} hunk{})\n\n{}",
                        path,
                        hunks.len(),
                        if hunks.len() == 1 { "" } else { "s" },
                        diff_preview
                    );
                    let body = crate::tools::post_mutation::after_mutation(&path, body).await;
                    ToolExecuteResult::ok(body)
                }
                Err(e) => ToolExecuteResult::error(format!("Failed to write {path}: {e}")),
            }
        })
        .await
    }
}

fn normalize_patch_args(args: Value) -> Value {
    let Value::Object(mut map) = args else {
        return args;
    };
    if !map.contains_key("patch") {
        for alias in ["diff", "patch_text", "content", "text"] {
            if let Some(v) = map.remove(alias) {
                map.insert("patch".into(), v);
                break;
            }
        }
    }
    if !map.contains_key("file_path") {
        for alias in ["path", "file", "filename"] {
            if let Some(v) = map.remove(alias) {
                map.insert("file_path".into(), v);
                break;
            }
        }
    }
    Value::Object(map)
}

#[derive(Debug, Clone)]
struct Hunk {
    /// 1-based old start line (from @@ header); 0 means not used.
    old_start: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Delete(String),
    Insert(String),
}

fn parse_unified_diff(patch: &str) -> Result<(Option<String>, Vec<Hunk>), String> {
    let mut path: Option<String> = None;
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut cur: Option<Hunk> = None;

    for raw in patch.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix("+++ ") {
            let p = rest
                .trim()
                .trim_start_matches("b/")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if p != "/dev/null" && !p.is_empty() {
                path = Some(p);
            }
            continue;
        }
        if line.starts_with("--- ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@") {
            if let Some(h) = cur.take() {
                hunks.push(h);
            }
            // @@ -old_start,old_count +new_start,new_count @@
            let body = rest.trim_start();
            let old_part = body
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('-');
            let old_start = old_part
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            cur = Some(Hunk {
                old_start,
                lines: Vec::new(),
            });
            continue;
        }
        if let Some(h) = cur.as_mut() {
            if let Some(text) = line.strip_prefix('+') {
                if !line.starts_with("+++") {
                    h.lines.push(HunkLine::Insert(text.to_string()));
                }
            } else if let Some(text) = line.strip_prefix('-') {
                if !line.starts_with("---") {
                    h.lines.push(HunkLine::Delete(text.to_string()));
                }
            } else if let Some(text) = line.strip_prefix(' ') {
                h.lines.push(HunkLine::Context(text.to_string()));
            } else if line == "\\ No newline at end of file" {
                continue;
            } else if !line.is_empty() {
                // tolerate missing leading space on context
                h.lines.push(HunkLine::Context(line.to_string()));
            }
        }
    }
    if let Some(h) = cur {
        hunks.push(h);
    }
    if hunks.is_empty() {
        return Err("No @@ hunks found in patch".into());
    }
    Ok((path, hunks))
}

fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, String> {
    let mut file_lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
    let had_trailing_nl = original.ends_with('\n');

    // Apply from bottom to top so line numbers stay valid.
    let mut ordered: Vec<&Hunk> = hunks.iter().collect();
    ordered.sort_by_key(|h| std::cmp::Reverse(h.old_start));

    for hunk in ordered {
        apply_one_hunk(&mut file_lines, hunk)?;
    }

    let mut out = file_lines.join("\n");
    if had_trailing_nl || original.is_empty() {
        if !out.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
    }
    Ok(out)
}

fn apply_one_hunk(file_lines: &mut Vec<String>, hunk: &Hunk) -> Result<(), String> {
    // Build expected old slice and new slice from hunk lines.
    let mut old_slice: Vec<&str> = Vec::new();
    let mut new_slice: Vec<String> = Vec::new();
    for line in &hunk.lines {
        match line {
            HunkLine::Context(s) => {
                old_slice.push(s);
                new_slice.push(s.clone());
            }
            HunkLine::Delete(s) => old_slice.push(s),
            HunkLine::Insert(s) => new_slice.push(s.clone()),
        }
    }

    if old_slice.is_empty() && new_slice.is_empty() {
        return Ok(());
    }

    // Prefer header line number (1-based); fall back to search.
    let mut start = hunk.old_start.saturating_sub(1);
    if !old_slice.is_empty() {
        if !slice_matches(file_lines, start, &old_slice) {
            if let Some(found) = find_slice(file_lines, &old_slice) {
                start = found;
            } else {
                return Err(format!(
                    "hunk at line {} context not found. Expected:\n{}",
                    hunk.old_start,
                    old_slice
                        .iter()
                        .take(8)
                        .map(|l| format!("  {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        file_lines.drain(start..start + old_slice.len());
        for (i, line) in new_slice.into_iter().enumerate() {
            file_lines.insert(start + i, line);
        }
    } else {
        // Pure insert: place at old_start
        for (i, line) in new_slice.into_iter().enumerate() {
            file_lines.insert(start + i, line);
        }
    }
    Ok(())
}

fn slice_matches(file: &[String], start: usize, needle: &[&str]) -> bool {
    if start + needle.len() > file.len() {
        return false;
    }
    file[start..start + needle.len()]
        .iter()
        .zip(needle.iter())
        .all(|(a, b)| a == *b)
}

fn find_slice(file: &[String], needle: &[&str]) -> Option<usize> {
    if needle.is_empty() || needle.len() > file.len() {
        return None;
    }
    for i in 0..=file.len() - needle.len() {
        if slice_matches(file, i, needle) {
            return Some(i);
        }
    }
    None
}

/// Preview path + would-be diff for permission cards (best-effort).
pub fn preview_apply_patch(args: &Value) -> Option<String> {
    let args = normalize_patch_args(args.clone());
    let parsed: ApplyPatchArgs = serde_json::from_value(args).ok()?;
    let (header_path, hunks) = parse_unified_diff(&parsed.patch).ok()?;
    let path = parsed
        .file_path
        .filter(|s| !s.is_empty())
        .or(header_path)?;
    let original = std::fs::read_to_string(&path).ok()?;
    let updated = apply_hunks(&original, &hunks).ok()?;
    Some(crate::tools::diffutil::unified_diff_capped(
        &path, &original, &updated, 40,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_simple_hunk() {
        let original = "a\nb\nc\n";
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";
        let (path, hunks) = parse_unified_diff(patch).unwrap();
        assert_eq!(path.as_deref(), Some("x.txt"));
        let out = apply_hunks(original, &hunks).unwrap();
        assert_eq!(out, "a\nB\nc\n");
    }

    #[test]
    fn finds_context_when_line_number_wrong() {
        let original = "one\ntwo\nthree\n";
        let patch = "\
--- a/f
+++ b/f
@@ -99,3 +99,3 @@
 one
-two
+TWO
 three
";
        let (_, hunks) = parse_unified_diff(patch).unwrap();
        let out = apply_hunks(original, &hunks).unwrap();
        assert_eq!(out, "one\nTWO\nthree\n");
    }
}
