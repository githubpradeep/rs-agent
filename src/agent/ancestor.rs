//! Short brief for a nested agent: goal, parent reason, recent attempts.
//! The parent transcript stays out.

use crate::rlm::tree::{CallStatus, CallTree};
use std::sync::Mutex;

static GOAL: Mutex<String> = Mutex::new(String::new());

pub fn publish_goal(text: Option<&str>) {
    if let Ok(mut g) = GOAL.lock() {
        *g = text.unwrap_or("").trim().to_string();
    }
}

pub fn current_goal() -> String {
    GOAL.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Build the ancestor block from an explicit goal so tests do not share the static.
pub fn render_brief(goal: &str, parent: &str, tried: &[(String, String)]) -> String {
    let goal = goal.trim();
    let parent = parent.trim();
    if goal.is_empty() && parent.is_empty() && tried.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ancestor>\n");
    if !goal.is_empty() {
        let g: String = goal.chars().take(400).collect();
        out.push_str(&format!("goal: {g}\n"));
    }
    if !parent.is_empty() {
        let p: String = parent.chars().take(400).collect();
        out.push_str(&format!("parent: {p}\n"));
    }
    if !tried.is_empty() {
        out.push_str("already tried:\n");
        for (task, summary) in tried.iter().take(3) {
            let t: String = task.chars().take(160).collect();
            let s: String = summary.chars().take(200).collect();
            out.push_str(&format!("- {t} — {s}\n"));
        }
    }
    out.push_str("</ancestor>");
    if out.chars().count() > 2000 {
        out.chars().take(2000).collect()
    } else {
        out
    }
}

pub fn brief_for(tree: &CallTree, parent_id: &str) -> String {
    let snap = tree.snapshot();
    let parent = snap
        .nodes
        .iter()
        .find(|n| n.id == parent_id)
        .map(|n| n.task.clone())
        .filter(|t| t != "session")
        .unwrap_or_default();
    let mut tried: Vec<(String, String)> = snap
        .nodes
        .iter()
        .filter(|n| n.parent_id.as_deref() == Some(parent_id))
        .filter(|n| matches!(n.status, CallStatus::Done | CallStatus::Error))
        .map(|n| {
            (
                n.task.clone(),
                n.summary.clone().unwrap_or_else(|| match n.status {
                    CallStatus::Error => "error".into(),
                    CallStatus::Aborted => "aborted".into(),
                    _ => "done".into(),
                }),
            )
        })
        .collect();
    if tried.len() > 3 {
        tried = tried.split_off(tried.len() - 3);
    }
    render_brief(&current_goal(), &parent, &tried)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_is_short_and_skips_empty() {
        assert!(render_brief("", "", &[]).is_empty());
        let b = render_brief(
            "close bead 12",
            "parse the log",
            &[("grep dates".into(), "no hits".into())],
        );
        assert!(b.contains("goal: close bead 12"));
        assert!(b.contains("parent: parse the log"));
        assert!(b.contains("grep dates"));
        assert!(!b.contains("transcript"));
        assert!(b.chars().count() < 2000);
    }
}
