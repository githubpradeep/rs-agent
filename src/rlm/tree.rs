use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Root,
    Agent,
    Llm,
    Repl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    Running,
    Done,
    Error,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: CallKind,
    pub task: String,
    pub status: CallStatus,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallTreeInner {
    pub nodes: Vec<CallNode>,
    pub root_id: Option<String>,
}

impl CallTreeInner {
    /// One-line human summary of a (possibly stale/loaded-from-disk) call
    /// tree snapshot: node counts by status plus the root task, if any.
    /// Used by `/tree` in the TUI when there's no live run to show a
    /// breadcrumb for.
    pub fn summary(&self) -> String {
        if self.nodes.is_empty() {
            return "(empty call tree)".to_string();
        }
        let total = self.nodes.len();
        let done = self
            .nodes
            .iter()
            .filter(|n| n.status == CallStatus::Done)
            .count();
        let running = self
            .nodes
            .iter()
            .filter(|n| n.status == CallStatus::Running)
            .count();
        let errors = self
            .nodes
            .iter()
            .filter(|n| n.status == CallStatus::Error)
            .count();
        let aborted = self
            .nodes
            .iter()
            .filter(|n| n.status == CallStatus::Aborted)
            .count();
        let root_task = self
            .root_id
            .as_ref()
            .and_then(|rid| self.nodes.iter().find(|n| &n.id == rid))
            .map(|n| n.task.as_str())
            .unwrap_or("(unknown)");
        format!(
            "{} node(s) — {} done, {} running, {} error(s), {} aborted. Root: {}",
            total, done, running, errors, aborted, root_task
        )
    }
}

/// Collapse whitespace/newlines and cap length so Call Tree panel stays readable.
fn sanitize_task_label(task: &str, max_chars: usize) -> String {
    let collapsed = task.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(max_chars).collect();
    if collapsed.chars().count() > max_chars {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Shared call tree for RLM recursive decomposition.
#[derive(Clone, Default)]
pub struct CallTree {
    inner: Arc<Mutex<CallTreeInner>>,
}

impl CallTree {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CallTreeInner::default())),
        }
    }

    pub fn ensure_root(&self, task: &str) -> String {
        let mut g = self.inner.lock().unwrap();
        if let Some(id) = &g.root_id {
            return id.clone();
        }
        let id = format!("root_{}", g.nodes.len());
        g.nodes.push(CallNode {
            id: id.clone(),
            parent_id: None,
            kind: CallKind::Root,
            task: sanitize_task_label(task, 60),
            // Idle placeholder until Deep Context work actually starts.
            status: CallStatus::Done,
            summary: None,
        });
        g.root_id = Some(id.clone());
        id
    }

    pub fn spawn(&self, parent_id: Option<&str>, kind: CallKind, task: &str) -> String {
        let mut g = self.inner.lock().unwrap();
        let id = format!(
            "{}_{}",
            match kind {
                CallKind::Root => "root",
                CallKind::Agent => "agent",
                CallKind::Llm => "llm",
                CallKind::Repl => "repl",
            },
            g.nodes.len()
        );
        g.nodes.push(CallNode {
            id: id.clone(),
            parent_id: parent_id.map(|s| s.to_string()),
            kind,
            task: sanitize_task_label(task, 48),
            status: CallStatus::Running,
            summary: None,
        });
        if g.root_id.is_none() && parent_id.is_none() {
            g.root_id = Some(id.clone());
        }
        // Mark parent (usually session root) active while children run.
        if let Some(pid) = parent_id {
            if let Some(parent) = g.nodes.iter_mut().find(|n| n.id == pid) {
                if parent.status == CallStatus::Done {
                    parent.status = CallStatus::Running;
                }
            }
        }
        id
    }

    pub fn finish(&self, id: &str, status: CallStatus, summary: Option<String>) {
        let mut g = self.inner.lock().unwrap();
        let parent_id = g
            .nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.parent_id.clone());
        if let Some(node) = g.nodes.iter_mut().find(|n| n.id == id) {
            node.status = status;
            node.summary = summary.map(|s| sanitize_task_label(&s, 80));
        }
        // If all siblings under the parent are settled, mark parent Done again.
        if let Some(pid) = parent_id {
            let any_running = g.nodes.iter().any(|n| {
                n.parent_id.as_deref() == Some(pid.as_str()) && n.status == CallStatus::Running
            });
            if !any_running {
                if let Some(parent) = g.nodes.iter_mut().find(|n| n.id == pid) {
                    if parent.kind == CallKind::Root && parent.status == CallStatus::Running {
                        parent.status = CallStatus::Done;
                    }
                }
            }
        }
    }

    pub fn snapshot(&self) -> CallTreeInner {
        self.inner.lock().unwrap().clone()
    }

    pub fn render(&self) -> String {
        let g = self.inner.lock().unwrap();
        if g.nodes.is_empty() {
            return "(empty call tree)".to_string();
        }
        let mut out = String::new();
        fn walk(nodes: &[CallNode], parent: Option<&str>, depth: usize, out: &mut String) {
            for n in nodes.iter().filter(|n| n.parent_id.as_deref() == parent) {
                let indent = "  ".repeat(depth);
                let status = match n.status {
                    CallStatus::Running => "…",
                    CallStatus::Done => "✓",
                    CallStatus::Error => "✗",
                    CallStatus::Aborted => "⊘",
                };
                let kind = match n.kind {
                    CallKind::Root => "root",
                    CallKind::Agent => "agent",
                    CallKind::Llm => "llm",
                    CallKind::Repl => "repl",
                };
                out.push_str(&format!(
                    "{}{} [{}] {} — {}\n",
                    indent, status, kind, n.id, n.task
                ));
                walk(nodes, Some(&n.id), depth + 1, out);
            }
        }
        let root = g.root_id.clone();
        if let Some(ref rid) = root {
            if let Some(n) = g.nodes.iter().find(|n| &n.id == rid) {
                let status = match n.status {
                    CallStatus::Running => "…",
                    CallStatus::Done => "✓",
                    CallStatus::Error => "✗",
                    CallStatus::Aborted => "⊘",
                };
                out.push_str(&format!("{} [root] {} — {}\n", status, n.id, n.task));
                walk(&g.nodes, Some(rid.as_str()), 1, &mut out);
            }
        } else {
            walk(&g.nodes, None, 0, &mut out);
        }
        if out.is_empty() {
            for n in &g.nodes {
                out.push_str(&format!("{:?} [{}] {}\n", n.status, n.id, n.task));
            }
        }
        out
    }

    pub fn breadcrumb(&self) -> String {
        let g = self.inner.lock().unwrap();
        let running: Vec<&CallNode> = g
            .nodes
            .iter()
            .filter(|n| n.status == CallStatus::Running)
            .collect();
        // `attach_repl_tool` / `attach_task_tool` call ensure_root("session") at
        // startup, which leaves a lone Running root forever. That is not active
        // Deep Context — treat it as idle so the TUI does not show `[D]` / `root`.
        if running.is_empty() || (running.len() == 1 && running[0].kind == CallKind::Root) {
            return "idle".to_string();
        }
        running
            .iter()
            .map(|n| match n.kind {
                CallKind::Root => "root",
                CallKind::Agent => "agent",
                CallKind::Llm => "llm",
                CallKind::Repl => "repl",
            })
            .collect::<Vec<_>>()
            .join(">")
    }
}

impl std::fmt::Debug for CallTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CallTree({})", self.render().replace('\n', " | "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_spawn_and_render() {
        let tree = CallTree::new();
        let root = tree.ensure_root("do the thing");
        let child = tree.spawn(Some(&root), CallKind::Llm, "slice A");
        tree.finish(&child, CallStatus::Done, Some("ok".into()));
        let rendered = tree.render();
        assert!(rendered.contains("root"));
        assert!(rendered.contains("llm"));
        assert_eq!(tree.breadcrumb(), "idle"); // lone running root is not active Deep Context
        let child2 = tree.spawn(Some(&root), CallKind::Repl, "active");
        assert_eq!(tree.breadcrumb(), "root>repl");
        tree.finish(&child2, CallStatus::Done, None);
        assert_eq!(tree.breadcrumb(), "idle");
    }

    #[test]
    fn snapshot_roundtrips_via_json() {
        let tree = CallTree::new();
        let root = tree.ensure_root("root task");
        let child = tree.spawn(Some(&root), CallKind::Agent, "child");
        tree.finish(&child, CallStatus::Done, Some("summary".into()));
        let snap = tree.snapshot();
        let json = serde_json::to_value(&snap).expect("serialize");
        let back: CallTreeInner = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.root_id.as_deref(), Some(root.as_str()));
    }

    #[test]
    fn call_tree_render_collapses_multiline_task() {
        let tree = CallTree::new();
        let root = tree.ensure_root("session");
        let code = "from pathlib import Path\nimport textwrap\nbase = Path('/tmp')\n";
        let child = tree.spawn(Some(&root), CallKind::Repl, code);
        tree.finish(&child, CallStatus::Done, Some("ok".into()));
        let rendered = tree.render();
        // Each node label itself must be single-line: no raw "import textwrap" line.
        assert!(
            !rendered
                .lines()
                .any(|l| l.trim_start().starts_with("import ")),
            "multiline code leaked into tree:\n{rendered}"
        );
        assert!(rendered.contains("[repl]"));
        assert!(rendered.contains("from pathlib"));
    }

    #[test]
    fn call_tree_inner_summary_reports_counts_and_root_task() {
        let tree = CallTree::new();
        let root = tree.ensure_root("summarize me");
        let a = tree.spawn(Some(&root), CallKind::Llm, "a");
        let b = tree.spawn(Some(&root), CallKind::Llm, "b");
        tree.finish(&a, CallStatus::Done, None);
        tree.finish(&b, CallStatus::Error, Some("boom".into()));

        let summary = tree.snapshot().summary();
        assert!(summary.contains("3 node(s)"));
        // Root returns to Done once children settle; plus child `a`.
        assert!(summary.contains("2 done"));
        assert!(summary.contains("1 error"));
        assert!(summary.contains("summarize me"));
    }

    #[test]
    fn call_tree_inner_summary_reports_empty_for_default() {
        assert_eq!(CallTreeInner::default().summary(), "(empty call tree)");
    }

    #[tokio::test]
    async fn repl_starts_when_python3_available() {
        if std::process::Command::new("python3")
            .arg("-c")
            .arg("print(1)")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            == false
        {
            return;
        }
        let mut repl = crate::rlm::repl::ReplSession::spawn()
            .await
            .expect("start repl");
        let out = repl
            .exec_with_host("print(1+1)", |_m, _a, _k| async {
                Ok(serde_json::Value::Null)
            })
            .await
            .expect("exec");
        assert!(out.ok, "stdout={} stderr={}", out.stdout, out.stderr);
        assert!(out.stdout.contains('2'));
    }
}
