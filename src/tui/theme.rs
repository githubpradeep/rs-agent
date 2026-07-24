//! Named TUI color palettes (`dark` / `light` / `forest`).

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
            "light" => Self::Light,
            "forest" => Self::Forest,
            _ => Self::Dark,
        }
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

#[derive(Debug, Clone, Copy)]
pub struct Palette {
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
}

impl Palette {
    pub fn for_theme(name: ThemeName) -> Self {
        match name {
            ThemeName::Dark => Self {
                user: Color::Green,
                assistant: Color::Yellow,
                system: Color::Cyan,
                tool: Color::Cyan,
                muted: Color::DarkGray,
                accent: Color::Cyan,
                danger: Color::Red,
                warn: Color::Yellow,
                ok: Color::Green,
                text: Color::White,
                border: Color::DarkGray,
                highlight_fg: Color::Black,
                highlight_bg: Color::White,
            },
            ThemeName::Light => Self {
                user: Color::Green,
                assistant: Color::Magenta,
                system: Color::Blue,
                tool: Color::Blue,
                muted: Color::Gray,
                accent: Color::Blue,
                danger: Color::Red,
                warn: Color::Rgb(180, 100, 0),
                ok: Color::Green,
                text: Color::Black,
                border: Color::Gray,
                highlight_fg: Color::White,
                highlight_bg: Color::Blue,
            },
            ThemeName::Forest => Self {
                user: Color::Rgb(120, 200, 120),
                assistant: Color::Rgb(200, 180, 100),
                system: Color::Rgb(100, 180, 160),
                tool: Color::Rgb(100, 180, 160),
                muted: Color::Rgb(90, 110, 90),
                accent: Color::Rgb(80, 160, 120),
                danger: Color::Rgb(220, 80, 80),
                warn: Color::Rgb(220, 160, 60),
                ok: Color::Rgb(100, 200, 100),
                text: Color::Rgb(220, 230, 210),
                border: Color::Rgb(70, 100, 70),
                highlight_fg: Color::Black,
                highlight_bg: Color::Rgb(140, 200, 140),
            },
        }
    }
}
