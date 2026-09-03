//! Colours and border styles.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;

/// Box-drawing characters are `East_Asian_Width=Ambiguous`: 1 cell in a Western
/// terminal, 2 in one configured for East Asian ambiguous-wide. There is no way
/// to detect which, so ASCII borders are offered as a one-switch escape hatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    Unicode,
    Ascii,
}

impl BorderStyle {
    pub fn border_type(self) -> BorderType {
        match self {
            BorderStyle::Unicode => BorderType::Rounded,
            BorderStyle::Ascii => BorderType::Plain,
        }
    }
}

pub const ACCENT: Color = Color::Cyan;
pub const DANGER: Color = Color::Red;
pub const WARN: Color = Color::Yellow;
pub const OK: Color = Color::Green;
pub const DIM: Color = Color::DarkGray;

pub fn title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
pub fn hint() -> Style {
    Style::default().fg(DIM)
}
pub fn warn() -> Style {
    Style::default().fg(WARN)
}
pub fn danger() -> Style {
    Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
}
