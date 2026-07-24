use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ContextFile {
    pub path: PathBuf,
    pub content: String,
}

const CANDIDATES: &[&str] = &["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];

fn load_first_from_dir(dir: &Path) -> Option<ContextFile> {
    for name in CANDIDATES {
        let p = dir.join(name);
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                if !content.trim().is_empty() {
                    return Some(ContextFile { path: p, content });
                }
            }
        }
    }
    None
}

fn config_dir() -> PathBuf {
    crate::config::config_dir()
}

pub fn discover_context_files() -> Vec<ContextFile> {
    let mut files: Vec<ContextFile> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let global = config_dir().join("AGENTS.md");
    if global.exists() {
        if let Ok(content) = std::fs::read_to_string(&global) {
            if !content.trim().is_empty() {
                seen.insert(global.canonicalize().unwrap_or(global.clone()));
                files.push(ContextFile { path: global, content });
            }
        }
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    let root = PathBuf::from("/");
    let mut current = if cwd.exists() { cwd.canonicalize().unwrap_or(cwd) } else { return files };

    let mut ancestors: Vec<ContextFile> = Vec::new();
    loop {
        if let Some(cf) = load_first_from_dir(&current) {
            let canon = cf.path.canonicalize().unwrap_or(cf.path.clone());
            if seen.insert(canon) {
                ancestors.push(cf);
            }
        }
        if current == root {
            break;
        }
        match current.parent() {
            Some(p) if p != current => current = p.to_path_buf(),
            _ => break,
        }
    }

    ancestors.reverse();
    files.extend(ancestors);
    files
}

pub fn build_context_section(files: &[ContextFile]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut section = String::from("\n\n<project_context>\n\nProject-specific instructions and guidelines:\n\n");
    for cf in files {
        let path_str = cf.path.display().to_string();
        section.push_str(&format!(
            "<project_instructions path=\"{}\">\n{}\n</project_instructions>\n\n",
            path_str, cf.content
        ));
    }
    section.push_str("</project_context>\n");
    section
}

const RS_AGENT_DIR: &str = ".rs-agent";
const COMMANDS_DIR: &str = "commands";

pub fn discover_agent_commands() -> Vec<ContextFile> {
    let mut commands = Vec::new();
    let cwd = std::env::current_dir().unwrap_or_default();
    let cmd_dir = cwd.join(RS_AGENT_DIR).join(COMMANDS_DIR);
    if !cmd_dir.is_dir() {
        return commands;
    }
    let mut entries: Vec<_> = match std::fs::read_dir(&cmd_dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
        Err(_) => return commands,
    };
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if !content.trim().is_empty() {
                    commands.push(ContextFile { path, content });
                }
            }
        }
    }
    commands
}

pub fn build_commands_section(commands: &[ContextFile]) -> String {
    if commands.is_empty() {
        return String::new();
    }
    let mut section = String::from("\n\n<agent_commands>\nAvailable commands the user may reference:\n\n");
    for cmd in commands {
        let name = cmd.path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        section.push_str(&format!(
            "<command name=\"{}\">\n{}\n</command>\n\n",
            name, cmd.content
        ));
    }
    section.push_str("</agent_commands>\n");
    section
}

pub fn resolve_append_arg(arg: &str) -> Result<String, String> {
    if let Some(path) = arg.strip_prefix('@') {
        let p = PathBuf::from(path);
        std::fs::read_to_string(&p).map_err(|e| format!("Cannot read {}: {}", path, e))
    } else {
        Ok(arg.to_string())
    }
}
