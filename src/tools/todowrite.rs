//! In-session todo list tool (opencode/pi parity).

use crate::agent::tool::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "pending".into()
}

#[derive(Deserialize)]
struct TodoWriteArgs {
    todos: Vec<TodoItem>,
    /// When true, replace the whole list. Default: merge by id.
    #[serde(default)]
    merge: Option<bool>,
}

thread_local! {
    static TODOS: RefCell<Vec<TodoItem>> = const { RefCell::new(Vec::new()) };
}

/// Snapshot of the current in-memory todo list (for session persistence).
pub fn snapshot() -> Vec<TodoItem> {
    TODOS.with(|t| t.borrow().clone())
}

/// Restore todos from a resumed session.
pub fn restore(items: Vec<TodoItem>) {
    TODOS.with(|t| {
        *t.borrow_mut() = items;
    });
}

/// Clear the in-memory list (new session).
pub fn clear() {
    TODOS.with(|t| t.borrow_mut().clear());
}

pub fn format_summary(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "No todos.".into();
    }
    let mut out = String::from("Todos:\n");
    for t in items {
        let mark = match t.status.as_str() {
            "completed" | "done" => "[x]",
            "in_progress" | "active" => "[~]",
            "cancelled" => "[-]",
            _ => "[ ]",
        };
        out.push_str(&format!("  {mark} {} ({})\n", t.content, t.id));
    }
    out
}

fn normalize_status(s: &str) -> String {
    match s.trim().to_lowercase().as_str() {
        "done" | "complete" | "completed" => "completed".into(),
        "active" | "doing" | "in_progress" | "in-progress" | "wip" => "in_progress".into(),
        "cancel" | "cancelled" | "canceled" => "cancelled".into(),
        _ => "pending".into(),
    }
}

pub struct TodoWriteTool;

#[async_trait]
impl AgentTool for TodoWriteTool {
    fn name(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Create or update a structured todo list for the current session. \
         Pass todos=[{id, content, status}] where status is pending|in_progress|completed|cancelled. \
         By default merges by id; set merge=false to replace the whole list."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "Todo items to write",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "content": {"type": "string"},
                            "status": {
                                "type": "string",
                                "description": "pending | in_progress | completed | cancelled"
                            }
                        },
                        "required": ["id", "content"]
                    }
                },
                "merge": {
                    "type": "boolean",
                    "description": "Merge by id (default true). false replaces the list."
                }
            },
            "required": ["todos"]
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: TodoWriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid todowrite args: {e}. Expected {{todos: [{{id, content, status}}]}}."
                ))
            }
        };

        if parsed.todos.is_empty() {
            return ToolExecuteResult::error("todos array is empty.");
        }

        let merge = parsed.merge.unwrap_or(true);
        let incoming: Vec<TodoItem> = parsed
            .todos
            .into_iter()
            .map(|mut t| {
                if t.id.trim().is_empty() {
                    t.id = uuid::Uuid::new_v4().to_string();
                }
                t.status = normalize_status(&t.status);
                t
            })
            .collect();

        let summary = TODOS.with(|cell| {
            let mut guard = cell.borrow_mut();
            if merge {
                for item in incoming {
                    if let Some(existing) = guard.iter_mut().find(|t| t.id == item.id) {
                        *existing = item;
                    } else {
                        guard.push(item);
                    }
                }
            } else {
                *guard = incoming;
            }
            format_summary(&guard)
        });

        ToolExecuteResult::ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn merge_and_replace() {
        clear();
        let tool = TodoWriteTool;
        let r = tool
            .execute(
                "1",
                json!({"todos": [{"id": "a", "content": "one", "status": "pending"}]}),
            )
            .await;
        assert!(!r.is_error);
        assert!(r.content.contains("one"));

        let r = tool
            .execute(
                "2",
                json!({"todos": [{"id": "a", "content": "one done", "status": "completed"}]}),
            )
            .await;
        assert!(r.content.contains("[x]"));
        assert_eq!(snapshot().len(), 1);

        let r = tool
            .execute(
                "3",
                json!({
                    "merge": false,
                    "todos": [{"id": "b", "content": "fresh", "status": "pending"}]
                }),
            )
            .await;
        assert!(!r.is_error);
        assert_eq!(snapshot().len(), 1);
        assert_eq!(snapshot()[0].id, "b");
        clear();
    }
}
