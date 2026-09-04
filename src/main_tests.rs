use super::theme_mode_or_light;
use terminal_colorsaurus::ThemeMode;

#[test]
fn falls_back_to_light_theme_when_detection_fails() {
    // The reported crash: with no usable terminal device (e.g. no
    // controlling terminal) the query fails with ENXIO ("Device not
    // configured") and startup must not panic. Light is the only
    // implemented theme, so it is the safe fallback.
    let err: terminal_colorsaurus::Error =
        std::io::Error::from_raw_os_error(6).into();
    assert_eq!(theme_mode_or_light(Err(err)), ThemeMode::Light);
}

#[test]
fn keeps_detected_theme_mode_when_detection_succeeds() {
    assert_eq!(theme_mode_or_light(Ok(ThemeMode::Dark)), ThemeMode::Dark);
    assert_eq!(theme_mode_or_light(Ok(ThemeMode::Light)), ThemeMode::Light);
}
