use std::path::PathBuf;

const PROMPTS_DIR: &str = "prompts";

fn embedded_system_prompt() -> &'static str {
    include_str!("../prompts/system.md")
}

fn load_file(name: &str) -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {}", e))?;
    let path = cwd.join(PROMPTS_DIR).join(name);
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))
}

pub fn load_system_prompt() -> String {
    load_file("system.md").unwrap_or_else(|_| embedded_system_prompt().to_string())
}

pub fn resolve_prompt_path(name: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let path = cwd.join(PROMPTS_DIR).join(name);
    if path.exists() { Some(path) } else { None }
}
