//! Spill oversized tool outputs to disk so the model can re-read them.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_MAX_LINES: usize = 2000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn tool_output_dir() -> PathBuf {
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".rs-agent").join("tool-output")
}

fn next_tool_id() -> String {
    let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("tool_{ms}_{n}")
}

#[derive(Debug, Clone)]
pub struct TruncateResult {
    pub content: String,
    pub truncated: bool,
    pub output_path: Option<PathBuf>,
    pub total_lines: usize,
    pub total_bytes: usize,
}

/// Truncate from the head (keep first N lines / bytes). Spill full text to disk when truncated.
pub fn truncate_or_spill(text: &str) -> TruncateResult {
    truncate_or_spill_with(text, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES, "head")
}

/// Truncate keeping head or tail; spill full text when limits exceeded.
pub fn truncate_or_spill_with(
    text: &str,
    max_lines: usize,
    max_bytes: usize,
    direction: &str,
) -> TruncateResult {
    let total_bytes = text.len();
    let lines: Vec<&str> = if text.is_empty() {
        Vec::new()
    } else {
        let mut v: Vec<&str> = text.split('\n').collect();
        if text.ends_with('\n') {
            v.pop();
        }
        v
    };
    let total_lines = lines.len();

    let within_lines = total_lines <= max_lines;
    let within_bytes = total_bytes <= max_bytes;
    if within_lines && within_bytes {
        return TruncateResult {
            content: text.to_string(),
            truncated: false,
            output_path: None,
            total_lines,
            total_bytes,
        };
    }

    let preview = if direction == "tail" {
        truncate_tail(&lines, max_lines, max_bytes)
    } else {
        truncate_head(&lines, max_lines, max_bytes)
    };

    let path = spill_full(text);
    let path_display = path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(spill failed)".into());

    let content = format!(
        "{preview}\n\n\
         [truncated: showing preview; {total_lines} lines, {total_bytes} bytes total]\n\
         Full output saved to: {path_display}\n\
         Use `read` with offset/limit or `grep` on that path to inspect the rest."
    );

    TruncateResult {
        content,
        truncated: true,
        output_path: path,
        total_lines,
        total_bytes,
    }
}

fn truncate_head(lines: &[&str], max_lines: usize, max_bytes: usize) -> String {
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if i >= max_lines {
            break;
        }
        let size = line.len() + if i > 0 { 1 } else { 0 };
        if bytes + size > max_bytes && !out.is_empty() {
            break;
        }
        if bytes + size > max_bytes && out.is_empty() {
            // Single huge line — take a byte prefix.
            let take = max_bytes.min(line.len());
            return line[..take].to_string();
        }
        out.push(*line);
        bytes += size;
    }
    out.join("\n")
}

fn truncate_tail(lines: &[&str], max_lines: usize, max_bytes: usize) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut bytes = 0usize;
    for line in lines.iter().rev() {
        if out.len() >= max_lines {
            break;
        }
        let size = line.len() + if !out.is_empty() { 1 } else { 0 };
        if bytes + size > max_bytes && !out.is_empty() {
            break;
        }
        if bytes + size > max_bytes && out.is_empty() {
            let take = max_bytes.min(line.len());
            let start = line.len() - take;
            return line[start..].to_string();
        }
        out.push(*line);
        bytes += size;
    }
    out.reverse();
    out.join("\n")
}

fn spill_full(text: &str) -> Option<PathBuf> {
    let dir = tool_output_dir();
    if fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let path = dir.join(format!("{}.txt", next_tool_id()));
    match fs::File::create(&path) {
        Ok(mut f) => {
            if f.write_all(text.as_bytes()).is_ok() {
                Some(path)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// Marker substring used by TUI to detect spilled output.
pub const SPILL_MARKER: &str = "Full output saved to:";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_text_not_truncated() {
        let r = truncate_or_spill("hello\nworld\n");
        assert!(!r.truncated);
        assert!(r.output_path.is_none());
        assert!(r.content.contains("hello"));
    }

    #[test]
    fn large_text_spills() {
        let mut s = String::new();
        for i in 0..3000 {
            s.push_str(&format!("line {i} xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n"));
        }
        let r = truncate_or_spill(&s);
        assert!(r.truncated);
        assert!(r.content.contains(SPILL_MARKER));
        assert!(r.output_path.is_some());
        let path = r.output_path.unwrap();
        assert!(path.exists());
        let full = fs::read_to_string(&path).unwrap();
        assert_eq!(full, s);
        let _ = fs::remove_file(path);
    }
}
