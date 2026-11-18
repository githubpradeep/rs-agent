//! Shared slash / palette command catalog (Wave 5 structure).

/// Commands shown in the Ctrl+K palette and used for discovery.
pub const PALETTE_COMMANDS: &[&str] = &[
    "/help",
    "/keys",
    "/settings",
    "/theme",
    "/tree",
    "/timeline",
    "/output",
    "/mode",
    "/model",
    "/provider",
    "/goal",
    "/compact",
    "/recall",
    "/new",
    "/fork",
    "/sessions",
    "/export",
    "/clear",
    "/context",
    "/skills",
    "/skill",
    "/prompt",
    "/reload",
    "/trust",
    "/rename",
    "/history",
    "/lsp",
];

pub fn filter_commands(query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    PALETTE_COMMANDS
        .iter()
        .filter(|c| q.is_empty() || c.to_lowercase().contains(&q))
        .map(|s| (*s).to_string())
        .collect()
}
