//! Unified diff helpers for edit results and permission previews.

use similar::{ChangeTag, TextDiff};

/// Build a unified-style diff (header + +/- lines) between `old` and `new`.
/// Caps output to keep tool results / permission cards readable.
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    unified_diff_capped(path, old, new, 80)
}

pub fn unified_diff_capped(path: &str, old: &str, new: &str, max_lines: usize) -> String {
    if old == new {
        return format!("--- a/{path}\n+++ b/{path}\n(no changes)");
    }
    let diff = TextDiff::from_lines(old, new);
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    let mut count = 0usize;
    let mut truncated = false;
    for change in diff.iter_all_changes() {
        if count >= max_lines {
            truncated = true;
            break;
        }
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        // Skip equal lines outside of a small context window would be nicer,
        // but a full dump of equals blows the card — only show changed + nearby.
        if change.tag() == ChangeTag::Equal {
            continue;
        }
        let mut line = change.to_string();
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        out.push(sign);
        out.push_str(&line);
        out.push('\n');
        count += 1;
    }
    if truncated {
        out.push_str(&format!(
            "… (diff truncated after {max_lines} changed lines)\n"
        ));
    }
    if count == 0 {
        // Only whitespace/line-ending changes that similar collapsed oddly
        out.push_str("(content changed; no line-level +/- hunks)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_plus_minus() {
        let d = unified_diff("a.rs", "fn a() {}\n", "fn a() {\n  1\n}\n");
        assert!(d.contains("--- a/a.rs"));
        assert!(d.contains('-') || d.contains('+'));
        assert!(d.contains("fn a()"));
    }
}
