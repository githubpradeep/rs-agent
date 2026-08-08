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
    "/city",
    "/fleet",
    "/wish",
    "/mode",
    "/model",
    "/provider",
    "/goal",
    "/route",
    "/handoff",
    "/seat",
    "/beads",
    "/city",
    "/compact",
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
    "/worker",
    "/marshal",
    "/detach",
    "/mail",
    "/wish",
    "/brain",
    "/trust",
    "/rename",
    "/history",
    "/lsp",
    "/image",
];

pub fn filter_commands(query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    PALETTE_COMMANDS
        .iter()
        .filter(|c| q.is_empty() || c.to_lowercase().contains(&q))
        .map(|s| (*s).to_string())
        .collect()
}
