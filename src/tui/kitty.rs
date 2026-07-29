//! Kitty graphics protocol helpers — display local image files in the terminal.

use base64::Engine;
use std::fs;
use std::io::Write;
use std::path::Path;

/// True if `path` looks like a common raster image by extension.
pub fn is_image_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
}

/// Scan text for absolute/relative paths that look like image files and exist.
pub fn find_image_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| {
            matches!(c, '"' | '\'' | '`' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}')
        });
        if is_image_path(cleaned) && Path::new(cleaned).is_file() {
            if !out.iter().any(|p| p == cleaned) {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

/// Emit a Kitty graphics protocol image to `out` (direct, not via ratatui).
/// Uses transmission medium `t` (temporary file) when possible; falls back to
/// inline base64 (`d=a`) for small files.
///
/// Returns Ok(true) if bytes were written; Ok(false) if skipped (too large / unreadable).
pub fn write_kitty_image(out: &mut dyn Write, path: &str, max_cols: u16) -> std::io::Result<bool> {
    let path = Path::new(path);
    if !path.is_file() {
        return Ok(false);
    }
    let meta = fs::metadata(path)?;
    // Cap at 4MB raw to avoid flooding the terminal.
    if meta.len() > 4 * 1024 * 1024 {
        writeln!(out, "[image too large for Kitty display: {}]", path.display())?;
        return Ok(false);
    }
    let bytes = fs::read(path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let cols = max_cols.max(20).min(120);

    // Chunked transmission (Kitty recommends chunks ≤ 4096).
    let chunk_size = 4096;
    let mut first = true;
    let mut offset = 0;
    while offset < b64.len() {
        let end = (offset + chunk_size).min(b64.len());
        let chunk = &b64[offset..end];
        let more = if end < b64.len() { 1 } else { 0 };
        if first {
            // a=T place+transmit, f=100 PNG auto, m=more, c=columns
            write!(
                out,
                "\x1b_Ga=T,f=100,m={more},c={cols};{chunk}\x1b\\"
            )?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
        offset = end;
    }
    writeln!(out)?;
    let _ = out.flush();
    Ok(true)
}

/// Detect whether TERM / env suggests Kitty or a compatible graphics terminal.
pub fn kitty_graphics_likely() -> bool {
    let term = std::env::var("TERM").unwrap_or_default().to_lowercase();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    term.contains("kitty")
        || program.contains("kitty")
        || program.contains("wezterm")
        || program.contains("ghostty")
        || std::env::var("KITTY_WINDOW_ID").is_ok()
        || std::env::var("RS_AGENT_KITTY").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_extensions() {
        assert!(is_image_path("foo.PNG"));
        assert!(is_image_path("/tmp/x.webp"));
        assert!(!is_image_path("foo.rs"));
    }

    #[test]
    fn finds_paths_in_text() {
        let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        let path = tmp.path().to_string_lossy().to_string();
        let text = format!("saved to {path} ok");
        let found = find_image_paths(&text);
        assert_eq!(found, vec![path]);
    }
}
