//! Agent interaction modes: agent (default), plan (read-only), ask (no tools).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentMode {
    #[default]
    Agent,
    Plan,
    Ask,
}

impl AgentMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "agent" | "a" => Some(Self::Agent),
            "plan" | "p" => Some(Self::Plan),
            "ask" | "q" => Some(Self::Ask),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Plan => "plan",
            Self::Ask => "ask",
        }
    }

    /// Tools allowed in this mode. `None` means all tools.
    pub fn allows_tool(self, name: &str) -> bool {
        match self {
            Self::Agent => true,
            Self::Ask => false,
            Self::Plan => {
                matches!(
                    name,
                    "read" | "grep" | "ls" | "find" | "webfetch" | "websearch" | "bead"
                ) || name.contains("__read") // common MCP read-only naming
            }
        }
    }

    pub fn system_note(self) -> Option<&'static str> {
        match self {
            Self::Agent => None,
            Self::Plan => Some(
                "MODE: plan. You may only use read-only tools (read, grep, ls, find, webfetch, websearch, bead list/show). Do not edit or run shell. Propose a plan; wait for the user to switch to agent mode to implement.",
            ),
            Self::Ask => Some(
                "MODE: ask. Answer questions only. Do not call any tools.",
            ),
        }
    }
}
