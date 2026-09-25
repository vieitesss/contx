use super::Theme;
use ratatui::style::Color;

#[test]
fn ansi_theme_defers_to_terminal_defaults_and_named_colors() {
    assert_eq!(Theme::ANSI.bg, Color::Reset);
    assert_eq!(Theme::ANSI.fg, Color::Reset);
    assert_eq!(Theme::ANSI.accent, Color::Yellow);
    assert_eq!(Theme::ANSI.green, Color::Green);
    assert_eq!(Theme::ANSI.red, Color::Red);
    assert_eq!(Theme::ANSI.git_icon, Color::Blue);
    assert_eq!(Theme::ANSI.worktree, Color::Cyan);
    // Muted roles fall back to the default foreground, not a pale gray.
    assert_eq!(Theme::ANSI.operator, Color::Reset);
    assert_eq!(Theme::ANSI.comment, Color::Reset);
    assert_eq!(Theme::ANSI.bg_alt, Color::DarkGray);
}

#[test]
fn light_palette_blends_selection_toward_foreground() {
    let theme = Theme::from_colors((0, 0, 0), (255, 255, 255));
    let Color::Rgb(r, g, b) = theme.bg_alt else {
        panic!("expected a blended selection background");
    };
    assert!(r < 255 && g < 255 && b < 255);
    assert_eq!((r, g, b), (217, 217, 217));
}

#[test]
fn dark_palette_blends_selection_toward_foreground() {
    let theme = Theme::from_colors((255, 255, 255), (0, 0, 0));
    let Color::Rgb(r, g, b) = theme.bg_alt else {
        panic!("expected a blended selection background");
    };
    assert!(r > 0 && g > 0 && b > 0);
    assert_eq!((r, g, b), (38, 38, 38));
}

#[test]
fn light_palette_mutes_toward_the_dark_background() {
    let theme = Theme::from_colors((0, 0, 0), (255, 255, 255));
    let Color::Rgb(r, g, b) = theme.comment else {
        panic!("expected a blended muted color");
    };
    // A dark gray, far darker than bright-black ANSI DarkGray on a light
    // palette.
    assert_eq!((r, g, b), (77, 77, 77));
    assert_eq!(theme.operator, theme.comment);
}

#[test]
fn dark_palette_mutes_toward_the_light_background() {
    let theme = Theme::from_colors((255, 255, 255), (0, 0, 0));
    let Color::Rgb(r, g, b) = theme.comment else {
        panic!("expected a blended muted color");
    };
    // A light-ish gray that keeps contrast against the dark background.
    assert_eq!((r, g, b), (179, 179, 179));
    assert_eq!(theme.operator, theme.comment);
}

#[test]
fn identical_foreground_and_background_keep_the_ansi_fallback() {
    let theme = Theme::from_colors((10, 20, 30), (10, 20, 30));
    assert_eq!(theme.bg_alt, Theme::ANSI.bg_alt);
    assert_eq!(theme.comment, Theme::ANSI.comment);
    assert_eq!(theme.operator, Theme::ANSI.operator);
}
