use ratatui::style::Color;
use terminal_colorsaurus::ThemeMode;

#[derive(Clone, Copy, Default)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub bg_alt: Color,
    pub accent: Color,
    pub operator: Color,
    pub green: Color,
    pub red: Color,
    pub comment: Color,
    pub git_icon: Color,
    /// Linked-worktree icon: a distinct teal, neither the
    /// yellow accent nor the red/green counts, so repo vs
    /// worktree stays obvious.
    pub worktree: Color,
}

impl Theme {
    pub const LIGHT: Self = Self {
        bg: Color::Rgb(0xF5, 0xF5, 0xF5),
        fg: Color::Rgb(0x28, 0x28, 0x28),
        bg_alt: Color::Rgb(0xEA, 0xE7, 0xE1),
        accent: Color::Rgb(0xCC, 0x96, 0x00),
        operator: Color::Rgb(0xAA, 0xA4, 0x9C),
        green: Color::Rgb(0x82, 0xA7, 0x62),
        red: Color::Rgb(0x98, 0x22, 0x2A),
        comment: Color::Rgb(0x45, 0x55, 0x4D),
        git_icon: Color::Rgb(0x37, 0x52, 0x6D),
        worktree: Color::Rgb(0x3D, 0x7A, 0x6F),
    };

    pub fn get(mode: ThemeMode) -> Theme {
        match mode {
            ThemeMode::Dark => panic!("dark theme not yet implemented"),
            ThemeMode::Light => Theme::LIGHT,
        }
    }
}
