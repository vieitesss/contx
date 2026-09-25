use super::{log_file_path, report_activation};
use crate::mux::{ActivateError, ActivateResult, DetectError};
use crate::tmux::errors::{ActivationError, TmuxError};
use std::{ffi::OsString, io, path::PathBuf};

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

fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.into())
}

#[test]
fn log_file_lives_in_xdg_state_home() {
    let env = vars(&[("XDG_STATE_HOME", "/state"), ("HOME", "/home/me")]);
    assert_eq!(
        log_file_path(&env),
        Some(PathBuf::from("/state/contx/app.log"))
    );
}

#[test]
fn log_file_falls_back_to_local_state_under_home() {
    let env = vars(&[("HOME", "/home/me")]);
    assert_eq!(
        log_file_path(&env),
        Some(PathBuf::from("/home/me/.local/state/contx/app.log"))
    );
}

#[test]
fn log_file_ignores_relative_directories() {
    // A relative path would resolve against the working directory, which
    // is exactly where the log must not end up.
    let env = vars(&[("XDG_STATE_HOME", "state"), ("HOME", "/home/me")]);
    assert_eq!(
        log_file_path(&env),
        Some(PathBuf::from("/home/me/.local/state/contx/app.log"))
    );
    assert_eq!(log_file_path(&vars(&[("HOME", "home")])), None);
    assert_eq!(log_file_path(&vars(&[])), None);
}
