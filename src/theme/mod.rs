use ratatui::style::Color;
use terminal_colorsaurus::ThemeMode;

#[derive(Clone, Copy, Default)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub bg_alt: Color,
    pub accent: Color,
}

impl Theme {
    pub const LIGHT: Self = Self {
        bg: Color::Rgb(0xF5, 0xF5, 0xF5),
        fg: Color::Rgb(0x28, 0x28, 0x28),
        bg_alt: Color::Rgb(0xCE, 0xC9, 0xC0),
        accent: Color::Rgb(0xCC, 0x96, 0x00),
    };

    pub fn get(mode: ThemeMode) -> Theme {
        match mode {
            ThemeMode::Dark => panic!("dark theme not yet implemented"),
            ThemeMode::Light => Theme::LIGHT,
        }
    }
}
