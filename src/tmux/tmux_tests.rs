use super::errors::{ActivationError, TmuxError};
use super::{CommandRunner, RawOutput, open_with, pane_cwds_with};
use crate::label::project_target_label;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, ErrorKind};

/// Scripted adapter at the tmux command boundary: replays queued results
/// and records every argv it receives.
struct ScriptRunner {
    script: VecDeque<io::Result<RawOutput>>,
    calls: Vec<Vec<String>>,
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
    fn run(&mut self, args: &[&str]) -> io::Result<RawOutput> {
        self.calls
            .push(args.iter().map(|s| s.to_string()).collect());
        self.script.pop_front().expect("scripted runner exhausted")
    }
}

fn succeeded() -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: true,
        stdout: vec![],
        stderr: vec![],
    })
}

fn absent(stderr: &str) -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: false,
        stdout: vec![],
        stderr: stderr.as_bytes().to_vec(),
    })
}

fn launch_failed() -> io::Result<RawOutput> {
    Err(io::Error::new(ErrorKind::NotFound, "no tmux binary"))
}

fn garbage_stdout() -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: true,
        stdout: vec![0xff, 0xfe],
        stderr: vec![],
    })
}

fn test_env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let owned: Vec<(String, OsString)> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), OsString::from(v)))
        .collect();
    move |name: &str| {
        owned
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

fn inside_env() -> impl Fn(&str) -> Option<OsString> {
    test_env(&[
        ("TMUX", "/tmp/tmux-1000/default,12345,0"),
        ("HOME", "/home/tester"),
        ("USER", "tester"),
    ])
}

const CANDIDATE: &str = "/home/tester/work/foo";
const SESSION: &str = "work_foo";
const ABSENT: &str = "can't find session: work_foo";

fn has_call(session: &str) -> Vec<String> {
    vec![
        "has-session".to_string(),
        "-t".to_string(),
        session.to_string(),
    ]
}

fn switch_call(session: &str) -> Vec<String> {
    vec![
        "switch-client".to_string(),
        "-t".to_string(),
        session.to_string(),
    ]
}

fn create_call(session: &str, path: &str) -> Vec<String> {
    vec![
        "new-session".to_string(),
        "-ds".to_string(),
        session.to_string(),
        "-c".to_string(),
        path.to_string(),
    ]
}

#[test]
fn refuses_to_run_outside_tmux() {
    for vars in [
        vec![("HOME", "/home/tester"), ("USER", "tester")],
        vec![("TMUX", ""), ("HOME", "/home/tester"), ("USER", "tester")],
    ] {
        let mut runner = ScriptRunner::new(vec![]);
        let env = test_env(&vars);
        let res = open_with(CANDIDATE, &mut runner, &env);
        assert!(matches!(res, Err(ActivationError::NotInTmux)));
        assert!(runner.calls.is_empty());
    }
}

#[test]
fn attaches_to_existing_session_without_creating() {
    let mut runner = ScriptRunner::new(vec![succeeded(), succeeded()]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    assert_eq!(res.unwrap().session, SESSION);
    assert_eq!(runner.calls, [has_call(SESSION), switch_call(SESSION)]);
}

#[test]
fn creates_absent_session_at_candidate_path_then_switches() {
    let mut runner =
        ScriptRunner::new(vec![absent(ABSENT), succeeded(), succeeded()]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    assert_eq!(res.unwrap().session, SESSION);
    assert_eq!(
        runner.calls,
        [
            has_call(SESSION),
            create_call(SESSION, CANDIDATE),
            switch_call(SESSION)
        ]
    );
}

#[test]
fn has_launch_failure_stops_without_create_or_switch() {
    let mut runner = ScriptRunner::new(vec![launch_failed()]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    match res {
        Err(ActivationError::CheckFailed { session, cause }) => {
            assert_eq!(session, SESSION);
            assert!(matches!(cause, TmuxError::IoError(_)));
        }
        other => panic!("expected CheckFailed, got {other:?}"),
    }
    assert_eq!(runner.calls, [has_call(SESSION)]);
}

#[test]
fn has_other_failure_stops_without_create_or_switch() {
    let mut runner =
        ScriptRunner::new(vec![absent("no server running on /tmp/tmux")]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    match res {
        Err(ActivationError::CheckFailed { session, cause }) => {
            assert_eq!(session, SESSION);
            assert!(matches!(cause, TmuxError::CommandFailed(_)));
        }
        other => panic!("expected CheckFailed, got {other:?}"),
    }
    assert_eq!(runner.calls, [has_call(SESSION)]);
}

#[test]
fn create_failure_stops_without_switch() {
    let mut runner = ScriptRunner::new(vec![
        absent(ABSENT),
        absent("duplicate session: work_foo"),
    ]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    match res {
        Err(ActivationError::CreateFailed { session, cause }) => {
            assert_eq!(session, SESSION);
            assert!(matches!(cause, TmuxError::CommandFailed(_)));
        }
        other => panic!("expected CreateFailed, got {other:?}"),
    }
    assert_eq!(
        runner.calls,
        [has_call(SESSION), create_call(SESSION, CANDIDATE)]
    );
}

#[test]
fn switch_failure_reports_the_switch_step() {
    let mut runner =
        ScriptRunner::new(vec![succeeded(), absent("connection refused")]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    match res {
        Err(ActivationError::SwitchFailed { session, cause }) => {
            assert_eq!(session, SESSION);
            assert!(matches!(cause, TmuxError::CommandFailed(_)));
        }
        other => panic!("expected SwitchFailed, got {other:?}"),
    }
    assert_eq!(runner.calls, [has_call(SESSION), switch_call(SESSION)]);
}

#[test]
fn switch_failure_after_create_reports_the_switch_step() {
    let mut runner =
        ScriptRunner::new(vec![absent(ABSENT), succeeded(), launch_failed()]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    match res {
        Err(ActivationError::SwitchFailed { session, cause }) => {
            assert_eq!(session, SESSION);
            assert!(matches!(cause, TmuxError::IoError(_)));
        }
        other => panic!("expected SwitchFailed, got {other:?}"),
    }
    assert_eq!(
        runner.calls,
        [
            has_call(SESSION),
            create_call(SESSION, CANDIDATE),
            switch_call(SESSION)
        ]
    );
}

#[test]
fn invalid_output_is_a_classified_check_failure() {
    let mut runner = ScriptRunner::new(vec![garbage_stdout()]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    match res {
        Err(ActivationError::CheckFailed { session, cause }) => {
            assert_eq!(session, SESSION);
            match cause {
                TmuxError::IoError(e) => {
                    assert_eq!(e.kind(), ErrorKind::InvalidData)
                }
                other => panic!("expected IoError, got {other:?}"),
            }
        }
        other => panic!("expected CheckFailed, got {other:?}"),
    }
    assert_eq!(runner.calls, [has_call(SESSION)]);
}

#[test]
fn disappeared_session_is_reported_not_recreated() {
    let mut runner = ScriptRunner::new(vec![succeeded(), absent(ABSENT)]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    match res {
        Err(ActivationError::SessionDisappeared { session }) => {
            assert_eq!(session, SESSION);
        }
        other => panic!("expected SessionDisappeared, got {other:?}"),
    }
    assert_eq!(runner.calls, [has_call(SESSION), switch_call(SESSION)]);
}

#[test]
fn absent_switch_after_create_is_reported_not_retried() {
    let mut runner =
        ScriptRunner::new(vec![absent(ABSENT), succeeded(), absent(ABSENT)]);
    let env = inside_env();

    let res = open_with(CANDIDATE, &mut runner, &env);

    assert!(matches!(
        res,
        Err(ActivationError::SessionDisappeared { .. })
    ));
    assert_eq!(
        runner.calls,
        [
            has_call(SESSION),
            create_call(SESSION, CANDIDATE),
            switch_call(SESSION)
        ]
    );
}

#[test]
fn derives_session_names_from_candidates() {
    let cases = [
        ("/home/tester/contx", "tester_contx"),
        ("/home/tester/work/something", "work_something"),
        ("/home/tester/personal/contx", "personal_contx"),
        ("/home/tester/personal/.dot", "personal__dot"),
        ("/home/tester/tester/proj", "proj"),
        ("/srv/other/proj", "proj"),
        ("/home/tester/a/b/c", "b_c"),
    ];
    for (candidate, expected) in cases {
        let mut runner = ScriptRunner::new(vec![succeeded(), succeeded()]);
        let env = inside_env();

        let res = open_with(candidate, &mut runner, &env);

        assert_eq!(res.unwrap().session, expected);
        assert_eq!(runner.calls, [has_call(expected), switch_call(expected)]);
    }
}

#[test]
fn lookup_name_equals_shared_label_policy() {
    let cases = [
        "/home/tester/contx",
        "/home/tester/work/something",
        "/home/tester/personal/.dot",
        "/home/tester/tester/proj",
        "/srv/other/proj",
        "/home/tester/a/b/c",
    ];
    let env = inside_env();
    for candidate in cases {
        let label = project_target_label(candidate, &env).unwrap();
        let mut runner = ScriptRunner::new(vec![succeeded(), succeeded()]);
        let res = open_with(candidate, &mut runner, &env);
        assert_eq!(res.unwrap().session, label);
        assert_eq!(runner.calls, [has_call(&label), switch_call(&label)]);
    }
}

#[test]
fn invalid_candidate_runs_no_commands() {
    let env = inside_env();

    // "/foo" lives directly under root, which has no file name.
    let mut runner = ScriptRunner::new(vec![]);
    let res = open_with("/foo", &mut runner, &env);
    assert!(matches!(res, Err(ActivationError::InvalidCandidate(_))));
    assert!(runner.calls.is_empty());

    // Naming needs $HOME to apply the home-relative rules.
    let env = test_env(&[("TMUX", "x"), ("USER", "tester")]);
    let mut runner = ScriptRunner::new(vec![]);
    let res = open_with(CANDIDATE, &mut runner, &env);
    assert!(matches!(res, Err(ActivationError::InvalidCandidate(_))));
    assert!(runner.calls.is_empty());

    // Naming needs $USER for a session candidate inside a direct child
    // of $HOME.
    let env = test_env(&[("TMUX", "x"), ("HOME", "/home/tester")]);
    let mut runner = ScriptRunner::new(vec![]);
    let res = open_with("/home/tester/sub/proj", &mut runner, &env);
    assert!(matches!(res, Err(ActivationError::InvalidCandidate(_))));
    assert!(runner.calls.is_empty());
}

#[test]
fn errors_render_actionable_diagnostics() {
    assert_eq!(
        format!("{}", ActivationError::NotInTmux),
        "not running inside tmux"
    );

    let e = ActivationError::SwitchFailed {
        session: SESSION.to_string(),
        cause: TmuxError::CommandFailed(io::Error::other("boom")),
    };
    let diagnostic = format!("{e}");
    assert!(diagnostic.contains(SESSION));
    assert!(diagnostic.contains("switch"));
    assert!(diagnostic.contains("boom"));

    let e = ActivationError::SessionDisappeared {
        session: SESSION.to_string(),
    };
    let diagnostic = format!("{e}");
    assert!(diagnostic.contains(SESSION));
    assert!(diagnostic.contains("not recreating"));

    let e = ActivationError::InvalidCandidate(CANDIDATE.to_string());
    assert!(format!("{e}").contains(CANDIDATE));
}

fn pane_cwd_stdout(text: &str) -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: true,
        stdout: text.as_bytes().to_vec(),
        stderr: vec![],
    })
}

#[test]
fn pane_cwds_lists_nonempty_paths() {
    let mut runner =
        ScriptRunner::new(vec![pane_cwd_stdout("/work/a\n/work/b\n\n")]);
    let cwds = pane_cwds_with(&mut runner).unwrap();
    assert_eq!(cwds, vec!["/work/a", "/work/b"]);
    assert_eq!(
        runner.calls[0],
        ["list-panes", "-a", "-F", "#{pane_current_path}",]
    );
}

#[test]
fn pane_cwds_command_failure_is_error() {
    let mut runner = ScriptRunner::new(vec![Ok(RawOutput {
        success: false,
        stdout: vec![],
        stderr: b"no server".to_vec(),
    })]);
    assert!(pane_cwds_with(&mut runner).is_err());
}
