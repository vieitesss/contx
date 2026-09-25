use ratatui::style::Color;
use terminal_colorsaurus::{ColorPalette, QueryOptions, color_palette};

/// TUI role palette. Every role is a named ANSI color or the terminal's
/// default foreground/background ([`Color::Reset`]), except the muted roles
/// and selection background, which are blended from the queried terminal
/// colors, so the picker adopts whatever palette the user's terminal is
/// configured with — in both light and dark terminals — instead of shipping
/// its own hardcoded colors.
#[derive(Clone, Copy)]
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
    /// Linked-worktree icon: cyan, distinct from the blue repo
    /// icon and the yellow accent, so repo vs worktree stays
    /// obvious.
    pub worktree: Color,
}

/// How far `bg_alt` moves from the terminal background toward its
/// foreground. Small enough to stay a highlight, large enough to read as a
/// distinct selection row.
const BG_ALT_BLEND: f32 = 0.15;

/// How far the muted `comment`/`operator` roles move from the terminal
/// foreground toward its background. Large enough to mute, small enough to
/// stay legible on both light and dark palettes.
const MUTED_BLEND: f32 = 0.3;

impl Theme {
    /// ANSI-only roles. `bg`/`fg` defer to the terminal defaults; the muted
    /// roles fall back to the default foreground too, and `bg_alt` to a gray
    /// ANSI color, when the terminal palette cannot be queried, so the picker
    /// stays legible (and still selects visibly) without a query.
    pub const ANSI: Self = Self {
        bg: Color::Reset,
        fg: Color::Reset,
        bg_alt: Color::DarkGray,
        accent: Color::Yellow,
        operator: Color::Reset,
        green: Color::Green,
        red: Color::Red,
        comment: Color::Reset,
        git_icon: Color::Blue,
        worktree: Color::Cyan,
    };

    /// Build the theme from the terminal's own colors. ANSI roles come
    /// straight from [`Theme::ANSI`]; the selection background and the muted
    /// roles have no ANSI equivalent, so they are derived from the terminal's
    /// foreground and background. Falls back to [`Theme::ANSI`] when the
    /// terminal palette cannot be queried, e.g. with no controlling terminal.
    pub fn detect() -> Self {
        match color_palette(QueryOptions::default()) {
            Ok(palette) => Self::from_palette(&palette),
            Err(e) => {
                log::warn!(
                    "failed to query terminal colors: {e}; using ANSI colors"
                );
                Self::ANSI
            }
        }
    }

    fn from_palette(palette: &ColorPalette) -> Self {
        Self::from_colors(
            palette.foreground.scale_to_8bit(),
            palette.background.scale_to_8bit(),
        )
    }

    /// `fg`/`bg` are 8-bit terminal channels. A pathological palette whose
    /// foreground equals its background keeps the ANSI fallback, since the
    /// blends would be invisible.
    fn from_colors(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> Self {
        if fg == bg {
            return Self::ANSI;
        }
        let muted = blend(fg, bg, MUTED_BLEND);
        Self {
            bg_alt: blend(bg, fg, BG_ALT_BLEND),
            operator: muted,
            comment: muted,
            ..Self::ANSI
        }
    }
}

/// Move `from` toward `to` by `factor`, channel by channel.
fn blend(from: (u8, u8, u8), to: (u8, u8, u8), factor: f32) -> Color {
    let channel = |b: u8, f: u8| {
        (b as f32 + (f as f32 - b as f32) * factor).round() as u8
    };
    Color::Rgb(
        channel(from.0, to.0),
        channel(from.1, to.1),
        channel(from.2, to.2),
    )
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
