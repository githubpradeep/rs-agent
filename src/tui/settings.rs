//! Tabbed settings modal (herdr settings pattern).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;
use ratatui::layout::Rect;

use super::theme::{Palette, ThemeName};
use super::widgets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Theme,
    Keys,
    Input,
    Alerts,
}

impl SettingsTab {
    pub fn all() -> &'static [SettingsTab] {
        &[
            SettingsTab::Theme,
            SettingsTab::Keys,
            SettingsTab::Input,
            SettingsTab::Alerts,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Keys => "Keys",
            Self::Input => "Input",
            Self::Alerts => "Alerts",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Theme => Self::Keys,
            Self::Keys => Self::Input,
            Self::Input => Self::Alerts,
            Self::Alerts => Self::Theme,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Theme => Self::Alerts,
            Self::Keys => Self::Theme,
            Self::Input => Self::Keys,
            Self::Alerts => Self::Input,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub tab: SettingsTab,
    pub theme: ThemeName,
    pub mouse_enabled: bool,
    pub toast: bool,
    pub toast_sound: bool,
    pub notify: String,
}

impl SettingsState {
    pub fn from_app(
        theme: ThemeName,
        mouse_enabled: bool,
        toast: bool,
        toast_sound: bool,
        notify: &str,
    ) -> Self {
        Self {
            tab: SettingsTab::Theme,
            theme,
            mouse_enabled,
            toast,
            toast_sound,
            notify: notify.to_string(),
        }
    }
}

pub fn render_settings(frame: &mut Frame, full: Rect, state: &SettingsState, palette: &Palette) {
    let area = widgets::centered_rect(full, 64, 14);
    let mut lines = vec![Line::from(
        SettingsTab::all()
            .iter()
            .flat_map(|t| {
                let selected = *t == state.tab;
                let style = if selected {
                    Style::default()
                        .fg(palette.contrast_on_accent())
                        .bg(palette.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.overlay1)
                };
                vec![
                    Span::styled(format!(" {} ", t.label()), style),
                    Span::raw(" "),
                ]
            })
            .collect::<Vec<_>>(),
    )];
    lines.push(Line::from(""));

    match state.tab {
        SettingsTab::Theme => {
            for name in [ThemeName::Dark, ThemeName::Light, ThemeName::Forest] {
                let mark = if name == state.theme { "●" } else { "○" };
                lines.push(Line::from(Span::styled(
                    format!(" {mark} {}", name.as_str()),
                    Style::default().fg(if name == state.theme {
                        palette.accent
                    } else {
                        palette.subtext
                    }),
                )));
            }
            lines.push(Line::from(Span::styled(
                " ←/→ tabs · ↑/↓ or 1/2/3 set theme · Enter apply · Esc close",
                Style::default().fg(palette.overlay0),
            )));
        }
        SettingsTab::Keys => {
            lines.push(Line::from(Span::styled(
                " Remap in ~/.rs-agent/config.toml [keybindings]",
                Style::default().fg(palette.subtext),
            )));
            lines.push(Line::from(Span::styled(
                " /keys or ? for live cheat sheet",
                Style::default().fg(palette.subtext),
            )));
        }
        SettingsTab::Input => {
            let mark = if state.mouse_enabled { "●" } else { "○" };
            lines.push(Line::from(Span::styled(
                format!(" {mark} mouse capture (toggle with m)"),
                Style::default().fg(palette.text),
            )));
            lines.push(Line::from(Span::styled(
                " disable_mouse in config.toml for native selection",
                Style::default().fg(palette.overlay0),
            )));
        }
        SettingsTab::Alerts => {
            lines.push(Line::from(Span::styled(
                format!(
                    " {} toast   {} sound   notify={}",
                    if state.toast { "●" } else { "○" },
                    if state.toast_sound { "●" } else { "○" },
                    state.notify
                ),
                Style::default().fg(palette.text),
            )));
            lines.push(Line::from(Span::styled(
                " t toast · s sound · n cycle notify off|terminal|system",
                Style::default().fg(palette.overlay0),
            )));
        }
    }

    widgets::render_modal_shell(frame, area, "settings", palette.accent, palette, lines);
}
