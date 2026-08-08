//! Semantic TUI palettes (herdr-style tokens + role colors).

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeName {
    Dark,
    Light,
    Forest,
}

impl ThemeName {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "auto" => Self::from_host(),
            "light" => Self::Light,
            "forest" => Self::Forest,
            _ => Self::Dark,
        }
    }

    /// Infer light/dark from the host terminal (`COLORFGBG` / `COLORTERM_BG`).
    pub fn from_host() -> Self {
        // COLORFGBG is typically "fg;bg" with 0–15 ANSI indices (or 256-color).
        if let Ok(v) = std::env::var("COLORFGBG") {
            if let Some(bg) = v.split(';').last() {
                if let Ok(n) = bg.trim().parse::<u16>() {
                    // Common heuristic: bright backgrounds → light theme.
                    if n >= 7 && n != 8 {
                        return Self::Light;
                    }
                    return Self::Dark;
                }
            }
        }
        if let Ok(v) = std::env::var("RS_AGENT_THEME") {
            return Self::parse(&v);
        }
        Self::Dark
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Forest => "forest",
        }
    }

    pub fn syntect_theme(self) -> &'static str {
        match self {
            Self::Light => "base16-ocean.light",
            Self::Dark | Self::Forest => "base16-ocean.dark",
        }
    }
}

/// All UI colors. Semantic tokens (herdr) + role aliases used by chat rows.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    // Role colors (chat)
    pub user: Color,
    pub assistant: Color,
    pub system: Color,
    pub tool: Color,
    pub muted: Color,
    pub accent: Color,
    pub danger: Color,
    pub warn: Color,
    pub ok: Color,
    pub text: Color,
    pub border: Color,
    pub highlight_fg: Color,
    pub highlight_bg: Color,

    // Surfaces (herdr)
    pub panel_bg: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub surface_dim: Color,
    pub overlay0: Color,
    pub overlay1: Color,
    pub subtext: Color,

    // Agent-state semantics (herdr): blocked / working / done / idle
    pub state_blocked: Color,
    pub state_working: Color,
    pub state_done: Color,
    pub state_idle: Color,
}

impl Palette {
    pub fn for_theme(name: ThemeName) -> Self {
        match name {
            ThemeName::Dark => Self::mocha(),
            ThemeName::Light => Self::light(),
            ThemeName::Forest => Self::forest(),
        }
    }

    /// Catppuccin Mocha — default dark (aligned with herdr).
    fn mocha() -> Self {
        Self {
            accent: Color::Rgb(137, 180, 250),
            panel_bg: Color::Rgb(24, 24, 37),
            surface0: Color::Rgb(49, 50, 68),
            surface1: Color::Rgb(69, 71, 90),
            surface_dim: Color::Rgb(30, 30, 46),
            overlay0: Color::Rgb(108, 112, 134),
            overlay1: Color::Rgb(127, 132, 156),
            text: Color::Rgb(205, 214, 244),
            subtext: Color::Rgb(166, 173, 200),
            muted: Color::Rgb(108, 112, 134),
            border: Color::Rgb(69, 71, 90),
            user: Color::Rgb(166, 227, 161),
            assistant: Color::Rgb(249, 226, 175),
            system: Color::Rgb(148, 226, 213),
            tool: Color::Rgb(137, 180, 250),
            danger: Color::Rgb(243, 139, 168),
            warn: Color::Rgb(250, 179, 135),
            ok: Color::Rgb(166, 227, 161),
            highlight_fg: Color::Rgb(24, 24, 37),
            highlight_bg: Color::Rgb(137, 180, 250),
            state_blocked: Color::Rgb(243, 139, 168),
            state_working: Color::Rgb(249, 226, 175),
            state_done: Color::Rgb(148, 226, 213),
            state_idle: Color::Rgb(166, 227, 161),
        }
    }

    /// Clean light — high-contrast ink on host terminal (Reset surfaces).
    /// Avoids Latte's muddy gray bands that fight white terminals.
    fn light() -> Self {
        Self {
            // GitHub-ish blue accent; readable on white without neon.
            accent: Color::Rgb(9, 105, 218),
            // Opaque modal fill; chrome uses Reset so it blends with the host.
            panel_bg: Color::Rgb(255, 255, 255),
            surface0: Color::Rgb(246, 248, 250),
            surface1: Color::Rgb(208, 215, 222),
            surface_dim: Color::Reset,
            overlay0: Color::Rgb(110, 118, 129),
            overlay1: Color::Rgb(87, 96, 106),
            // Near-black ink, not washed purple-gray.
            text: Color::Rgb(31, 35, 40),
            subtext: Color::Rgb(65, 74, 84),
            muted: Color::Rgb(110, 118, 129),
            border: Color::Rgb(208, 215, 222),
            user: Color::Rgb(26, 127, 55),
            assistant: Color::Rgb(130, 80, 223),
            system: Color::Rgb(17, 128, 128),
            tool: Color::Rgb(9, 105, 218),
            danger: Color::Rgb(207, 34, 46),
            warn: Color::Rgb(154, 103, 0),
            ok: Color::Rgb(26, 127, 55),
            highlight_fg: Color::Rgb(255, 255, 255),
            highlight_bg: Color::Rgb(9, 105, 218),
            state_blocked: Color::Rgb(207, 34, 46),
            state_working: Color::Rgb(154, 103, 0),
            state_done: Color::Rgb(17, 128, 128),
            state_idle: Color::Rgb(26, 127, 55),
        }
    }

    /// Everforest Dark Medium — warm paper text, clear green accent.
    fn forest() -> Self {
        Self {
            accent: Color::Rgb(167, 192, 128), // green
            panel_bg: Color::Rgb(39, 46, 51),  // bg0
            surface0: Color::Rgb(55, 63, 69),  // bg1
            surface1: Color::Rgb(65, 75, 82),  // bg2
            surface_dim: Color::Rgb(45, 53, 59),
            overlay0: Color::Rgb(133, 146, 137), // grey1
            overlay1: Color::Rgb(157, 169, 160), // grey2
            text: Color::Rgb(211, 198, 170),     // fg
            subtext: Color::Rgb(179, 188, 161),
            muted: Color::Rgb(133, 146, 137),
            border: Color::Rgb(65, 75, 82),
            user: Color::Rgb(167, 192, 128),      // green
            assistant: Color::Rgb(219, 188, 127), // yellow
            system: Color::Rgb(131, 192, 146),    // aqua
            tool: Color::Rgb(127, 187, 179),      // blue
            danger: Color::Rgb(230, 126, 128),    // red
            warn: Color::Rgb(230, 152, 117),      // orange
            ok: Color::Rgb(167, 192, 128),
            highlight_fg: Color::Rgb(39, 46, 51),
            highlight_bg: Color::Rgb(167, 192, 128),
            state_blocked: Color::Rgb(230, 126, 128),
            state_working: Color::Rgb(219, 188, 127),
            state_done: Color::Rgb(131, 192, 146),
            state_idle: Color::Rgb(167, 192, 128),
        }
    }

    /// Readable fg on accent / filled buttons (herdr `panel_contrast_fg`).
    pub fn contrast_on_accent(self) -> Color {
        self.highlight_fg
    }
}
