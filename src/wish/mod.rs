//! Wish Factory — intake human wishes as beads.

use crate::beads::{self, Bead, BeadKind};

/// Create a wish bead (design by default, or task with `--task`).
pub fn create_wish(text: &str, as_task: bool, auto_ready: bool) -> Result<Bead, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("wish text must not be empty".into());
    }
    let kind = if as_task {
        BeadKind::Task
    } else {
        BeadKind::Design
    };
    let title = if text.chars().count() > 72 {
        format!("Wish: {}…", text.chars().take(69).collect::<String>())
    } else {
        format!("Wish: {text}")
    };
    let notes = format!(
        "label:wish\nauto:{}\n\n{}",
        if auto_ready { "ready" } else { "triage" },
        text
    );
    let priority = if auto_ready { 50 } else { 80 };
    beads::add_full(None, &title, &notes, vec![], None, priority, kind, None)
}

pub fn format_created(b: &Bead) -> String {
    format!(
        "Wish accepted as {} [{}] priority={} — {}\nNotes preview: {}",
        b.id,
        b.kind.as_str(),
        b.priority,
        b.title,
        b.notes.chars().take(120).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wish_creates_design() {
        crate::with_temp_cwd(|_| {
            let b = create_wish("add dark mode", false, true).unwrap();
            assert_eq!(b.kind, BeadKind::Design);
            assert!(b.notes.contains("label:wish"));
        });
    }
}
