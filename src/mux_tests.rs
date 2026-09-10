use super::{
    ActivateError, ActivateResult, Backend, CommandRunner, DetectError,
    PaneCwdOutcome, RawOutput, TmuxProbeError, TmuxTtys, activate_auto_with,
    activate_explicit_herdr_with, detect_auto, pane_cwds_with, probe_tmux_ttys,
};
use crate::config::Multiplexer;
use crate::herdr;
use crate::tmux;
use crate::tmux::errors::ActivationError;
use std::cell::Cell;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, ErrorKind};

fn test_env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let owned: Vec<(String, OsString)> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
        .collect();
    move |name: &str| {
        owned
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

fn ttys(panes: &[&str], clients: &[&str]) -> TmuxTtys {
    TmuxTtys {
        panes: panes.iter().map(|s| (*s).to_string()).collect(),
        clients: clients.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn detect(
    vars: &[(&str, &str)],
    tty: Option<&str>,
    probe: Result<TmuxTtys, TmuxProbeError>,
) -> Result<Backend, DetectError> {
    let env = test_env(vars);
    detect_auto(&env, tty, &mut || probe.clone())
}

fn detect_recording_probe(
    vars: &[(&str, &str)],
    tty: Option<&str>,
    probe: Result<TmuxTtys, TmuxProbeError>,
) -> (Result<Backend, DetectError>, bool) {
    let env = test_env(vars);
    let called = Cell::new(false);
    let result = detect_auto(&env, tty, &mut || {
        called.set(true);
        probe.clone()
    });
    (result, called.get())
}

const PANE: &str = "/dev/ttys001";
const CLIENT: &str = "/dev/ttys002";
const OTHER: &str = "/dev/ttys009";
const TMUX: &str = "/tmp/tmux-1000/default,12345,0";
const SOCK: &str = "/tmp/herdr.sock";

#[test]
fn neither_marker_is_outside() {
    let res = detect(&[], Some(PANE), Ok(ttys(&[PANE], &[])));
    assert_eq!(res, Err(DetectError::Outside));
}

#[test]
fn empty_tmux_and_herdr_env_not_one_are_outside() {
    let res = detect(
        &[("TMUX", ""), ("HERDR_ENV", "true")],
        Some(PANE),
        Ok(ttys(&[PANE], &[])),
    );
    assert_eq!(res, Err(DetectError::Outside));
}

#[test]
fn only_tmux_tty_matching_pane_selects_tmux() {
    let res = detect(&[("TMUX", TMUX)], Some(PANE), Ok(ttys(&[PANE], &[])));
    assert_eq!(res, Ok(Backend::Tmux));
}

#[test]
fn only_tmux_tty_matching_client_selects_tmux() {
    let res = detect(&[("TMUX", TMUX)], Some(CLIENT), Ok(ttys(&[], &[CLIENT])));
    assert_eq!(res, Ok(Backend::Tmux));
}

#[test]
fn only_tmux_unknown_tty_fails_closed() {
    for tty in [None, Some(""), Some("   ")] {
        let res = detect(&[("TMUX", TMUX)], tty, Ok(ttys(&[PANE], &[])));
        assert_eq!(res, Err(DetectError::TmuxOwnershipUnknown));
    }
}

#[test]
fn only_tmux_probe_failure_fails_closed() {
    for err in [
        TmuxProbeError::Io,
        TmuxProbeError::Command,
        TmuxProbeError::InvalidUtf8,
    ] {
        let res = detect(&[("TMUX", TMUX)], Some(PANE), Err(err));
        assert_eq!(res, Err(DetectError::TmuxOwnershipUnknown));
    }
}

#[test]
fn only_tmux_known_tty_without_match_is_stale() {
    let res =
        detect(&[("TMUX", TMUX)], Some(OTHER), Ok(ttys(&[PANE], &[CLIENT])));
    assert_eq!(res, Err(DetectError::StaleTmux));
}

#[test]
fn only_herdr_with_socket_selects_herdr_without_probe() {
    let (res, called) = detect_recording_probe(
        &[("HERDR_ENV", "1"), ("HERDR_SOCKET_PATH", SOCK)],
        None,
        Err(TmuxProbeError::Io),
    );
    assert_eq!(res, Ok(Backend::Herdr));
    assert!(!called);
}

#[test]
fn only_herdr_missing_socket_fails_closed_without_probe() {
    for vars in [
        vec![("HERDR_ENV", "1")],
        vec![("HERDR_ENV", "1"), ("HERDR_SOCKET_PATH", "")],
    ] {
        let (res, called) =
            detect_recording_probe(&vars, None, Err(TmuxProbeError::Io));
        assert_eq!(res, Err(DetectError::MissingSocket));
        assert!(!called);
    }
}

#[test]
fn both_markers_tty_match_selects_tmux() {
    let res = detect(
        &[
            ("TMUX", TMUX),
            ("HERDR_ENV", "1"),
            ("HERDR_SOCKET_PATH", SOCK),
        ],
        Some(PANE),
        Ok(ttys(&[PANE], &[])),
    );
    assert_eq!(res, Ok(Backend::Tmux));
}

#[test]
fn both_markers_tmux_not_owner_with_socket_selects_herdr() {
    let res = detect(
        &[
            ("TMUX", TMUX),
            ("HERDR_ENV", "1"),
            ("HERDR_SOCKET_PATH", SOCK),
        ],
        Some(OTHER),
        Ok(ttys(&[PANE], &[CLIENT])),
    );
    assert_eq!(res, Ok(Backend::Herdr));
}

#[test]
fn both_markers_unknown_tty_is_ambiguous() {
    let res = detect(
        &[
            ("TMUX", TMUX),
            ("HERDR_ENV", "1"),
            ("HERDR_SOCKET_PATH", SOCK),
        ],
        None,
        Ok(ttys(&[PANE], &[])),
    );
    assert_eq!(res, Err(DetectError::Ambiguous));
}

#[test]
fn both_markers_probe_failure_is_ambiguous() {
    let res = detect(
        &[
            ("TMUX", TMUX),
            ("HERDR_ENV", "1"),
            ("HERDR_SOCKET_PATH", SOCK),
        ],
        Some(PANE),
        Err(TmuxProbeError::Command),
    );
    assert_eq!(res, Err(DetectError::Ambiguous));
}

#[test]
fn both_markers_tmux_not_owner_without_socket_fails_closed() {
    let res = detect(
        &[("TMUX", TMUX), ("HERDR_ENV", "1")],
        Some(OTHER),
        Ok(ttys(&[PANE], &[])),
    );
    assert_eq!(res, Err(DetectError::MissingSocket));
}

struct ScriptRunner {
    script: VecDeque<io::Result<RawOutput>>,
    calls: Vec<(String, Vec<String>)>,
}

impl ScriptRunner {
    fn new(script: Vec<io::Result<RawOutput>>) -> Self {
        Self {
            script: script.into(),
            calls: vec![],
        }
    }
}

impl CommandRunner for ScriptRunner {
    fn run(&mut self, program: &str, args: &[&str]) -> io::Result<RawOutput> {
        self.calls.push((
            program.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));
        self.script.pop_front().expect("scripted runner exhausted")
    }
}

fn tty_lines(text: &str) -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: true,
        stdout: text.as_bytes().to_vec(),
    })
}

#[test]
fn probe_tmux_ttys_uses_list_panes_and_list_clients() {
    let mut runner = ScriptRunner::new(vec![
        tty_lines("/dev/ttys001\n"),
        tty_lines("/dev/ttys002\n"),
    ]);

    let ttys = probe_tmux_ttys(&mut runner).unwrap();

    assert_eq!(
        runner.calls,
        [
            (
                "tmux".to_string(),
                vec![
                    "list-panes".into(),
                    "-a".into(),
                    "-F".into(),
                    "#{pane_tty}".into(),
                ]
            ),
            (
                "tmux".to_string(),
                vec![
                    "list-clients".into(),
                    "-F".into(),
                    "#{client_tty}".into(),
                ]
            ),
        ]
    );
    assert_eq!(ttys.panes, ["/dev/ttys001"]);
    assert_eq!(ttys.clients, ["/dev/ttys002"]);
}

#[test]
fn probe_tmux_ttys_classifies_launch_and_command_and_utf8() {
    let mut runner = ScriptRunner::new(vec![Err(io::Error::new(
        ErrorKind::NotFound,
        "no tmux",
    ))]);
    assert_eq!(
        probe_tmux_ttys(&mut runner).unwrap_err(),
        TmuxProbeError::Io
    );

    let mut runner = ScriptRunner::new(vec![Ok(RawOutput {
        success: false,
        stdout: vec![],
    })]);
    assert_eq!(
        probe_tmux_ttys(&mut runner).unwrap_err(),
        TmuxProbeError::Command
    );

    let mut runner = ScriptRunner::new(vec![Ok(RawOutput {
        success: true,
        stdout: vec![0xff, 0xfe],
    })]);
    assert_eq!(
        probe_tmux_ttys(&mut runner).unwrap_err(),
        TmuxProbeError::InvalidUtf8
    );
}

fn tmux_fail(_c: &str) -> Result<tmux::Activation, ActivationError> {
    Err(ActivationError::NotInTmux)
}

fn herdr_ok(_c: &str) -> Result<Option<herdr::Activation>, herdr::OpenError> {
    Ok(Some(herdr::Activation {
        workspace_id: "3".to_string(),
    }))
}

fn herdr_must_not_run(
    _c: &str,
) -> Result<Option<herdr::Activation>, herdr::OpenError> {
    panic!("herdr open must not run")
}

fn tmux_must_not_run(_c: &str) -> Result<tmux::Activation, ActivationError> {
    panic!("tmux open must not run")
}

#[test]
fn auto_tmux_backend_failure_does_not_fall_back_to_herdr() {
    let env = test_env(&[("TMUX", TMUX)]);
    let ttys = ttys(&[PANE], &[]);
    let err = activate_auto_with(
        "/tmp/proj",
        &env,
        Some(PANE),
        &mut || Ok(ttys.clone()),
        &mut tmux_fail,
        &mut herdr_must_not_run,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ActivateError::Tmux(ActivationError::NotInTmux)
    ));
}

#[test]
fn auto_herdr_does_not_call_tmux() {
    let env = test_env(&[("HERDR_ENV", "1"), ("HERDR_SOCKET_PATH", SOCK)]);
    let outcome = activate_auto_with(
        "/tmp/proj",
        &env,
        None,
        &mut || panic!("probe must not run"),
        &mut tmux_must_not_run,
        &mut herdr_ok,
    )
    .unwrap();
    assert_eq!(outcome, ActivateResult::Completed);
}

#[test]
fn explicit_herdr_without_context_does_not_invoke_open() {
    let env = test_env(&[]);
    let err = activate_explicit_herdr_with(
        "/tmp/proj",
        &env,
        &mut herdr_must_not_run,
    )
    .unwrap_err();
    assert!(matches!(err, ActivateError::NotInHerdr));
}

#[test]
fn explicit_herdr_with_context_invokes_open() {
    let env = test_env(&[("HERDR_ENV", "1"), ("HERDR_SOCKET_PATH", SOCK)]);
    let outcome =
        activate_explicit_herdr_with("/tmp/proj", &env, &mut herdr_ok).unwrap();
    assert_eq!(outcome, ActivateResult::Completed);
}

fn panes(
    mux: Multiplexer,
    vars: &[(&str, &str)],
    tty: Option<&str>,
    probe: Result<TmuxTtys, TmuxProbeError>,
    tmux: Result<Vec<String>, ()>,
    herdr: Result<Vec<String>, ()>,
) -> PaneCwdOutcome {
    let env = test_env(vars);
    pane_cwds_with(
        mux,
        &env,
        tty,
        &mut || probe.clone(),
        &mut || tmux.clone(),
        &mut || herdr.clone(),
    )
}

#[test]
fn pane_cwds_auto_outside_skips() {
    assert_eq!(
        panes(
            Multiplexer::Auto,
            &[],
            None,
            Err(TmuxProbeError::Io),
            Ok(vec!["/x".into()]),
            Ok(vec!["/y".into()]),
        ),
        PaneCwdOutcome::Skipped
    );
}

#[test]
fn pane_cwds_explicit_tmux_without_context_skips() {
    assert_eq!(
        panes(
            Multiplexer::Tmux,
            &[],
            Some(PANE),
            Ok(ttys(&[PANE], &[])),
            Ok(vec!["/x".into()]),
            Ok(vec!["/y".into()]),
        ),
        PaneCwdOutcome::Skipped
    );
}

#[test]
fn pane_cwds_explicit_tmux_lists_or_fails() {
    let listed = panes(
        Multiplexer::Tmux,
        &[("TMUX", TMUX)],
        Some(PANE),
        Ok(ttys(&[PANE], &[])),
        Ok(vec!["/work/a".into()]),
        Ok(vec!["/herdr".into()]),
    );
    assert_eq!(listed, PaneCwdOutcome::Listed(vec!["/work/a".into()]));

    let failed = panes(
        Multiplexer::Tmux,
        &[("TMUX", TMUX)],
        Some(PANE),
        Ok(ttys(&[PANE], &[])),
        Err(()),
        Ok(vec!["/herdr".into()]),
    );
    assert_eq!(failed, PaneCwdOutcome::Failed);
}

#[test]
fn pane_cwds_explicit_herdr_lists_only_herdr() {
    let listed = panes(
        Multiplexer::Herdr,
        &[("HERDR_ENV", "1"), ("HERDR_SOCKET_PATH", SOCK)],
        None,
        Err(TmuxProbeError::Io),
        Ok(vec!["/tmux".into()]),
        Ok(vec!["/herdr".into()]),
    );
    assert_eq!(listed, PaneCwdOutcome::Listed(vec!["/herdr".into()]));
}

#[test]
fn pane_cwds_auto_tmux_does_not_list_herdr() {
    let listed = panes(
        Multiplexer::Auto,
        &[("TMUX", TMUX)],
        Some(PANE),
        Ok(ttys(&[PANE], &[])),
        Ok(vec!["/tmux".into()]),
        Ok(vec!["/herdr".into()]),
    );
    assert_eq!(listed, PaneCwdOutcome::Listed(vec!["/tmux".into()]));
}
