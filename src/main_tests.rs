use super::{report_activation, theme_mode_or_light};
use crate::mux::{ActivateError, ActivateResult, DetectError};
use crate::tmux::errors::{ActivationError, TmuxError};
use std::io;
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

#[test]
fn successful_activation_reports_nothing() {
    assert_eq!(report_activation(Ok(ActivateResult::Completed)), None);
    assert_eq!(report_activation(Ok(ActivateResult::Cancelled)), None);
}

#[test]
fn failed_activation_reports_a_diagnostic_not_a_panic() {
    let err = ActivateError::Tmux(ActivationError::SwitchFailed {
        session: "work_foo".to_string(),
        cause: TmuxError::CommandFailed(io::Error::other("boom")),
    });

    let diagnostic =
        report_activation(Err(err)).expect("expected a diagnostic");

    assert!(diagnostic.contains("work_foo"));
    assert!(diagnostic.contains("switch"));
    assert!(diagnostic.contains("boom"));

    let diagnostic =
        report_activation(Err(ActivateError::Tmux(ActivationError::NotInTmux)))
            .expect("expected a diagnostic");
    assert_eq!(diagnostic, "not running inside tmux");
}

#[test]
fn outside_and_ambiguous_diagnostics_include_override_guidance() {
    let outside =
        report_activation(Err(ActivateError::Detect(DetectError::Outside)))
            .unwrap();
    assert!(outside.contains("not running inside a multiplexer"));
    assert!(outside.contains("--multiplexer tmux"));
    assert!(outside.contains("--multiplexer herdr"));

    let ambiguous =
        report_activation(Err(ActivateError::Detect(DetectError::Ambiguous)))
            .unwrap();
    assert!(ambiguous.contains("ambiguous multiplexer"));
    assert!(ambiguous.contains("--multiplexer tmux"));
}

#[test]
fn explicit_herdr_without_context_is_not_in_herdr() {
    let diagnostic = report_activation(Err(ActivateError::NotInHerdr)).unwrap();
    assert_eq!(diagnostic, "not running inside herdr");
}
