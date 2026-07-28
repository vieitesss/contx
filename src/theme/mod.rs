use ratatui::style::Color;

#[derive(Clone, Copy, Default)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub bg_alt: Color,
}

impl Theme {
    pub const LIGHT: Self = Self {
        bg: Color::Rgb(0xF5, 0xF5, 0xF5),
        fg: Color::Rgb(0x28, 0x28, 0x28),
        bg_alt: Color::Rgb(0xCE, 0xC9, 0xC0),
    };
}
