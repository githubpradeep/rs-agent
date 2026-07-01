use crate::agent::tool::*;
use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use serde::Deserialize;
use serde_json::Value;
use walkdir::WalkDir;

#[derive(Deserialize)]
pub struct FindArgs {
    pub pattern: String,
    pub path: Option<String>,
    pub max_depth: Option<usize>,
}

pub struct FindTool;

#[async_trait]
impl AgentTool for FindTool {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find files and directories matching a glob pattern. Supports wildcards like *.rs, src/**/*.py, .env*, etc. Recursive by default. Use this to locate files by name pattern instead of bash find."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match (e.g., *.rs, **/*.py, src/**/*.ts)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to current dir)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum directory depth to search"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> ToolExecuteResult {
        let parsed: FindArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolExecuteResult::error(format!("Invalid args: {}", e)),
        };

        let root = parsed.path.unwrap_or_else(|| ".".to_string());

        let mut builder = GlobSetBuilder::new();
        let glob_str = if parsed.pattern.contains('/') {
            parsed.pattern.clone()
        } else {
            format!("**/{}", parsed.pattern)
        };
        match Glob::new(&glob_str) {
            Ok(g) => builder.add(g),
            Err(e) => return ToolExecuteResult::error(format!("Invalid glob pattern: {}", e)),
        };
        let glob_set = match builder.build() {
            Ok(g) => g,
            Err(e) => return ToolExecuteResult::error(format!("Invalid glob pattern: {}", e)),
        };

        let mut walker = WalkDir::new(&root).follow_links(false).sort_by_file_name();
        if let Some(depth) = parsed.max_depth {
            walker = walker.max_depth(depth);
        }

        let mut results: Vec<String> = Vec::new();
        for entry in walker.into_iter().filter_entry(|e| {
            !e.file_name().to_string_lossy().starts_with('.')
        }) {
            let entry = match entry {
                Ok(e) => e,
                _ => continue,
            };

            let path_str = entry.path().to_string_lossy().to_string();
            if glob_set.is_match(&path_str) {
                let display = if entry.file_type().is_dir() {
                    format!("{}/", path_str)
                } else {
                    path_str
                };
                results.push(display);
            }
        }

        if results.is_empty() {
            return ToolExecuteResult::ok("No matching files found.");
        }

        if results.len() > 200 {
            let tail = results.len() - 200;
            results.truncate(200);
            results.push(format!("... and {} more matches", tail));
        }

        ToolExecuteResult::ok(results.join("\n"))
    }
}
