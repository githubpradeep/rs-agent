//! Ask the user a question mid-turn (TUI overlay / stdin fallback).

use crate::agent::tool::*;
use async_trait::async_trait;
use crossbeam_channel as channel;
use serde::Deserialize;
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct QuestionRequest {
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum QuestionReply {
    Answer(String),
    Cancelled,
}

pub struct PendingQuestion {
    pub request: QuestionRequest,
    pub reply_tx: tokio::sync::oneshot::Sender<QuestionReply>,
}

static QUESTION_TX: OnceLock<channel::Sender<PendingQuestion>> = OnceLock::new();

/// Wire the TUI (or other UI) question channel. First call wins.
pub fn set_question_channel(tx: channel::Sender<PendingQuestion>) {
    let _ = QUESTION_TX.set(tx);
}

fn question_tx() -> Option<&'static channel::Sender<PendingQuestion>> {
    QUESTION_TX.get()
}

#[derive(Deserialize)]
struct QuestionArgs {
    question: String,
    #[serde(default)]
    options: Option<Vec<String>>,
}

pub struct QuestionTool;

#[async_trait]
impl AgentTool for QuestionTool {
    fn name(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Ask the user a clarifying question and wait for their answer. \
         Optional options=[] presents choices; free-form answers are always accepted. \
         Use when blocked on a preference, ambiguous requirement, or irreversible choice."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to show the user"
                },
                "options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional suggested answers"
                }
            },
            "required": ["question"]
        })
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: QuestionArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecuteResult::error(format!(
                    "Invalid question args: {e}. Expected {{question: \"...\", options?: [...]}}."
                ))
            }
        };
        let question = parsed.question.trim().to_string();
        if question.is_empty() {
            return ToolExecuteResult::error("question must not be empty");
        }
        let options = parsed.options.unwrap_or_default();

        if let Some(tx) = question_tx() {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            if tx
                .send(PendingQuestion {
                    request: QuestionRequest {
                        question: question.clone(),
                        options: options.clone(),
                    },
                    reply_tx,
                })
                .is_err()
            {
                return ToolExecuteResult::error("Question channel closed");
            }
            return match reply_rx.await {
                Ok(QuestionReply::Answer(ans)) => {
                    ToolExecuteResult::ok(format!("User answered: {ans}"))
                }
                Ok(QuestionReply::Cancelled) => {
                    ToolExecuteResult::error("User cancelled the question")
                }
                Err(_) => ToolExecuteResult::error("Question prompt cancelled"),
            };
        }

        // Headless / no TUI: prompt on stdin.
        ToolExecuteResult::ok(stdin_ask(&question, &options))
    }
}

fn stdin_ask(question: &str, options: &[String]) -> String {
    let _ = writeln!(io::stderr(), "\n[question] {question}");
    if !options.is_empty() {
        for (i, opt) in options.iter().enumerate() {
            let _ = writeln!(io::stderr(), "  {}) {}", i + 1, opt);
        }
        let _ = write!(io::stderr(), "Answer (number or text): ");
    } else {
        let _ = write!(io::stderr(), "Answer: ");
    }
    let _ = io::stderr().flush();
    let mut line = String::new();
    let stdin = io::stdin();
    let _ = stdin.lock().read_line(&mut line);
    let ans = line.trim().to_string();
    if ans.is_empty() {
        return "User answered: (empty)".into();
    }
    if let Ok(n) = ans.parse::<usize>() {
        if n >= 1 && n <= options.len() {
            return format!("User answered: {}", options[n - 1]);
        }
    }
    format!("User answered: {ans}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn rejects_empty_question() {
        let tool = QuestionTool;
        let r = tool.execute("1", json!({"question": "  "})).await;
        assert!(r.is_error);
    }
}
