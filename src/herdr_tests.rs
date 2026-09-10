use super::{
    Activation, Choice, CommandRunner, HerdrError, OpenError, RawOutput,
    choose_from, open_with, pane_cwds_with, pane_list, workspace_create,
    workspace_focus,
};
use crate::label::project_target_label;
use crate::utils::test_utils::TempDir;
use serde_json::json;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Cursor, ErrorKind};

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

fn json_out(code: i32, body: &str) -> io::Result<RawOutput> {
    Ok(RawOutput {
        code,
        stdout: body.as_bytes().to_vec(),
        stderr: vec![],
    })
}

fn json_err(code: i32, body: &str) -> io::Result<RawOutput> {
    Ok(RawOutput {
        code,
        stdout: vec![],
        stderr: body.as_bytes().to_vec(),
    })
}

fn usage_exit() -> io::Result<RawOutput> {
    Ok(RawOutput {
        code: 2,
        stdout: vec![],
        stderr: b"unknown command\n".to_vec(),
    })
}

const PANE_LIST_OK: &str =
    r#"{"id":"cli:pane:list","result":{"type":"pane_list","panes":[]}}"#;
const FOCUS_OK: &str = r#"{"id":"cli:workspace:focus","result":{"type":"workspace_info","workspace":{"workspace_id":"3"}}}"#;
const CREATE_OK: &str = r#"{"id":"cli:workspace:create","result":{"type":"workspace_created","workspace":{"workspace_id":"4"}}}"#;

#[test]
fn pane_list_uses_herdr_on_path_when_bin_unset() {
    let mut runner = ScriptRunner::new(vec![json_out(0, PANE_LIST_OK)]);
    let env = test_env(&[]);

    let result = pane_list(&mut runner, &env).unwrap();

    assert_eq!(result["type"], "pane_list");
    assert_eq!(
        runner.calls,
        [("herdr".to_string(), vec!["pane".into(), "list".into()])]
    );
}

#[test]
fn empty_herdr_bin_path_falls_back_to_herdr() {
    let mut runner = ScriptRunner::new(vec![json_out(0, PANE_LIST_OK)]);
    let env = test_env(&[("HERDR_BIN_PATH", "")]);

    pane_list(&mut runner, &env).unwrap();

    assert_eq!(runner.calls[0].0, "herdr");
}

#[test]
fn injected_herdr_bin_path_is_the_program() {
    let mut runner = ScriptRunner::new(vec![json_out(0, PANE_LIST_OK)]);
    let env = test_env(&[("HERDR_BIN_PATH", "/opt/herdr/bin/herdr")]);

    pane_list(&mut runner, &env).unwrap();

    assert_eq!(runner.calls[0].0, "/opt/herdr/bin/herdr");
    assert_eq!(runner.calls[0].1, ["pane", "list"]);
}

#[test]
fn set_but_unlaunchable_bin_does_not_fall_back_to_path() {
    let mut runner = ScriptRunner::new(vec![Err(io::Error::new(
        ErrorKind::NotFound,
        "no such file",
    ))]);
    let env = test_env(&[("HERDR_BIN_PATH", "/missing/herdr")]);

    let err = pane_list(&mut runner, &env).unwrap_err();

    match err {
        HerdrError::NotFound { program } => {
            assert_eq!(program, "/missing/herdr");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
    assert_eq!(
        runner.calls,
        [(
            "/missing/herdr".to_string(),
            vec!["pane".into(), "list".into()]
        )]
    );
}

#[test]
fn workspace_focus_argv_and_success_envelope() {
    let mut runner = ScriptRunner::new(vec![json_out(0, FOCUS_OK)]);
    let env = test_env(&[]);

    let result = workspace_focus(&mut runner, &env, "3").unwrap();

    assert_eq!(result["type"], "workspace_info");
    assert_eq!(
        runner.calls,
        [(
            "herdr".to_string(),
            vec!["workspace".into(), "focus".into(), "3".into()]
        )]
    );
}

#[test]
fn workspace_create_argv_and_success_envelope() {
    let mut runner = ScriptRunner::new(vec![json_out(0, CREATE_OK)]);
    let env = test_env(&[]);

    let result =
        workspace_create(&mut runner, &env, "/tmp/proj", "work_proj").unwrap();

    assert_eq!(result["type"], "workspace_created");
    assert_eq!(
        runner.calls,
        [(
            "herdr".to_string(),
            vec![
                "workspace".into(),
                "create".into(),
                "--cwd".into(),
                "/tmp/proj".into(),
                "--label".into(),
                "work_proj".into(),
                "--focus".into(),
            ]
        )]
    );
}

#[test]
fn exit_two_is_unknown_command() {
    let mut runner = ScriptRunner::new(vec![usage_exit()]);
    let env = test_env(&[]);

    let err = pane_list(&mut runner, &env).unwrap_err();

    match err {
        HerdrError::UnknownCommand { detail } => {
            assert!(detail.contains("unknown command"));
        }
        other => panic!("expected UnknownCommand, got {other:?}"),
    }
}

#[test]
fn garbage_json_on_success_is_classified() {
    let mut runner = ScriptRunner::new(vec![json_out(0, "not json")]);
    let env = test_env(&[]);

    assert!(matches!(
        pane_list(&mut runner, &env),
        Err(HerdrError::GarbageJson)
    ));
}

#[test]
fn server_not_running_on_stderr() {
    let body = json!({
        "id": "cli:pane:list",
        "error": {
            "code": "server_not_running",
            "message": "no herdr server is running"
        }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![json_err(1, &body)]);
    let env = test_env(&[]);

    match pane_list(&mut runner, &env).unwrap_err() {
        HerdrError::ServerNotRunning { message } => {
            assert!(message.contains("no herdr server"));
        }
        other => panic!("expected ServerNotRunning, got {other:?}"),
    }
}

#[test]
fn protocol_mismatch_on_stderr() {
    let body = json!({
        "id": "cli:pane:list",
        "error": {
            "code": "protocol_mismatch",
            "message": "client protocol is older"
        }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![json_err(1, &body)]);
    let env = test_env(&[]);

    match pane_list(&mut runner, &env).unwrap_err() {
        HerdrError::ProtocolMismatch { message } => {
            assert!(message.contains("older"));
        }
        other => panic!("expected ProtocolMismatch, got {other:?}"),
    }
}

#[test]
fn other_api_error_keeps_code() {
    let body = json!({
        "id": "cli:workspace:focus",
        "error": {
            "code": "workspace_not_found",
            "message": "workspace not found"
        }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![json_err(1, &body)]);
    let env = test_env(&[]);

    match workspace_focus(&mut runner, &env, "9").unwrap_err() {
        HerdrError::Api { code, message } => {
            assert_eq!(code, "workspace_not_found");
            assert!(message.contains("not found"));
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

fn pane(
    workspace_id: &str,
    cwd: Option<&str>,
    foreground_cwd: Option<&str>,
    label: Option<&str>,
) -> serde_json::Value {
    let mut p = json!({ "workspace_id": workspace_id });
    if let Some(cwd) = cwd {
        p["cwd"] = json!(cwd);
    }
    if let Some(fg) = foreground_cwd {
        p["foreground_cwd"] = json!(fg);
    }
    if let Some(label) = label {
        p["label"] = json!(label);
    }
    p
}

fn pane_list_body(panes: Vec<serde_json::Value>) -> String {
    json!({
        "id": "cli:pane:list",
        "result": { "type": "pane_list", "panes": panes }
    })
    .to_string()
}

fn focus_err_not_found(id: &str) -> io::Result<RawOutput> {
    json_err(
        1,
        &json!({
            "id": "cli:workspace:focus",
            "error": {
                "code": "workspace_not_found",
                "message": format!("workspace {id} not found")
            }
        })
        .to_string(),
    )
}

fn open_env(home: &str) -> impl Fn(&str) -> Option<OsString> + use<> {
    let owned: Vec<(String, OsString)> = vec![
        ("HOME".to_string(), OsString::from(home)),
        ("USER".to_string(), OsString::from("tester")),
    ];
    move |name: &str| {
        owned
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

fn choose_id(
    id: &str,
) -> impl FnMut(&str, &[String]) -> Result<Choice, OpenError> {
    let id = id.to_string();
    move |_, _| Ok(Choice::Workspace(id.clone()))
}

fn choose_cancel() -> impl FnMut(&str, &[String]) -> Result<Choice, OpenError> {
    move |_, _| Ok(Choice::Cancel)
}

#[test]
fn canonicalize_failure_does_not_create() {
    let mut runner = ScriptRunner::new(vec![]);
    let env = open_env("/tmp");
    let mut chooser = choose_cancel();

    let err = open_with(
        "/definitely/not/a/contx/candidate",
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap_err();

    assert!(matches!(err, OpenError::CanonicalizeFailed { .. }));
    assert!(runner.calls.is_empty());
}

#[test]
fn unique_cwd_match_focuses_without_create() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let canonical = project.canonicalize().unwrap();
    let mut runner = ScriptRunner::new(vec![
        json_out(
            0,
            &pane_list_body(vec![pane(
                "3",
                Some(&canonical.display().to_string()),
                None,
                Some("other_label"),
            )]),
        ),
        json_out(0, FOCUS_OK),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    let outcome = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(
        outcome,
        Some(Activation {
            workspace_id: "3".to_string()
        })
    );
    assert_eq!(runner.calls.len(), 2);
    assert_eq!(runner.calls[0].1, ["pane", "list"]);
    assert_eq!(runner.calls[1].1, ["workspace", "focus", "3"]);
}

#[test]
fn unique_foreground_cwd_match_focuses() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let canonical = project.canonicalize().unwrap();
    let mut runner = ScriptRunner::new(vec![
        json_out(
            0,
            &pane_list_body(vec![pane(
                "7",
                None,
                Some(&canonical.display().to_string()),
                None,
            )]),
        ),
        json_out(0, FOCUS_OK),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(runner.calls[1].1, ["workspace", "focus", "7"]);
}

#[test]
fn duplicate_panes_same_workspace_are_one_focus() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let cwd = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(
            0,
            &pane_list_body(vec![
                pane("3", Some(&cwd), None, None),
                pane("3", None, Some(&cwd), None),
            ]),
        ),
        json_out(0, FOCUS_OK),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(runner.calls[1].1, ["workspace", "focus", "3"]);
}

#[test]
fn malformed_reported_paths_do_not_abort_or_false_match() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let other = d.child("work/bar");
    let cwd = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(
            0,
            &pane_list_body(vec![
                pane("1", Some("/definitely/missing/herdr-cwd"), None, None),
                pane("2", Some(""), None, Some("foo")),
                pane("3", Some(&other.display().to_string()), None, None),
                pane("4", Some(&cwd), None, None),
            ]),
        ),
        json_out(0, FOCUS_OK),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(runner.calls[1].1, ["workspace", "focus", "4"]);
}

#[test]
fn label_is_not_used_to_match() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let other = d.child("other/place");
    let other_cwd = other.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(
            0,
            &pane_list_body(vec![pane(
                "9",
                Some(&other_cwd),
                None,
                Some("work_foo"),
            )]),
        ),
        json_out(0, CREATE_OK),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(runner.calls[1].1[0], "workspace");
    assert_eq!(runner.calls[1].1[1], "create");
}

#[test]
fn zero_matches_creates_with_canonical_cwd_and_tmux_label() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let canonical = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(0, &pane_list_body(vec![])),
        json_out(0, CREATE_OK),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    let outcome = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(
        outcome,
        Some(Activation {
            workspace_id: "4".to_string()
        })
    );
    assert_eq!(
        runner.calls[1].1,
        [
            "workspace",
            "create",
            "--cwd",
            canonical.as_str(),
            "--label",
            "work_foo",
            "--focus",
        ]
    );
}

#[test]
fn create_label_equals_shared_label_policy() {
    let d = TempDir::new();
    let outside = TempDir::new();
    let home = d.path().display().to_string();
    let env = open_env(&home);
    let relatives = [
        "contx",
        "work/something",
        "personal/.dot",
        "tester/proj",
        "a/b/c",
    ];
    for rel in relatives {
        let project = d.child(rel);
        let path = project.display().to_string();
        let label = project_target_label(&path, &env).unwrap();
        let canonical = project.canonicalize().unwrap().display().to_string();
        let mut runner = ScriptRunner::new(vec![
            json_out(0, &pane_list_body(vec![])),
            json_out(0, CREATE_OK),
        ]);
        let mut chooser = choose_cancel();
        open_with(&path, &mut runner, &env, &mut chooser).unwrap();
        assert_eq!(
            runner.calls[1].1,
            [
                "workspace",
                "create",
                "--cwd",
                canonical.as_str(),
                "--label",
                label.as_str(),
                "--focus",
            ],
            "{rel}"
        );
    }
    let project = outside.child("other/proj");
    let path = project.display().to_string();
    let label = project_target_label(&path, &env).unwrap();
    let canonical = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(0, &pane_list_body(vec![])),
        json_out(0, CREATE_OK),
    ]);
    let mut chooser = choose_cancel();
    open_with(&path, &mut runner, &env, &mut chooser).unwrap();
    assert_eq!(label, "proj");
    assert_eq!(runner.calls[1].1[5], label);
    assert_eq!(runner.calls[1].1[3], canonical);
}

#[test]
fn invalid_candidate_runs_no_commands() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let path = project.display().to_string();
    let mut chooser = choose_cancel();

    let env = test_env(&[("USER", "tester")]);
    let mut runner = ScriptRunner::new(vec![]);
    let err = open_with(&path, &mut runner, &env, &mut chooser).unwrap_err();
    assert!(matches!(err, OpenError::InvalidCandidate(_)));
    assert!(runner.calls.is_empty());

    let nested = d.child("sub/proj");
    let home = d.path().display().to_string();
    let vars = [("HOME", home.as_str())];
    let env = test_env(&vars);
    let mut runner = ScriptRunner::new(vec![]);
    let err = open_with(
        &nested.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap_err();
    assert!(matches!(err, OpenError::InvalidCandidate(_)));
    assert!(runner.calls.is_empty());
}

#[test]
fn several_ids_chooser_selects_focus() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let cwd = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(
            0,
            &pane_list_body(vec![
                pane("3", Some(&cwd), None, None),
                pane("8", Some(&cwd), None, None),
            ]),
        ),
        json_out(0, FOCUS_OK),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_id("8");

    open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(runner.calls[1].1, ["workspace", "focus", "8"]);
}

#[test]
fn several_ids_cancel_has_no_focus_or_create() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let cwd = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![json_out(
        0,
        &pane_list_body(vec![
            pane("3", Some(&cwd), None, None),
            pane("8", Some(&cwd), None, None),
        ]),
    )]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    let outcome = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap();

    assert_eq!(outcome, None);
    assert_eq!(runner.calls.len(), 1);
    assert_eq!(runner.calls[0].1, ["pane", "list"]);
}

#[test]
fn choose_from_empty_q_and_eof_cancel() {
    let ids = vec!["3".to_string(), "8".to_string()];
    for input in ["\n", "q\n", ""] {
        let mut stdin = Cursor::new(input.as_bytes().to_vec());
        let mut stderr = Vec::new();
        let choice =
            choose_from("cand", &ids, true, &mut stdin, &mut stderr).unwrap();
        assert_eq!(choice, Choice::Cancel);
    }
}

#[test]
fn choose_from_non_tty_lists_ids_and_does_not_read() {
    let ids = vec!["3".to_string(), "8".to_string()];
    let mut stdin = Cursor::new(b"1\n".to_vec());
    let mut stderr = Vec::new();
    match choose_from("cand", &ids, false, &mut stdin, &mut stderr) {
        Err(OpenError::NotATty { ids: got }) => {
            assert_eq!(got, ids);
        }
        other => panic!("expected NotATty, got {other:?}"),
    }
}

#[test]
fn choose_from_invalid_input_is_error() {
    let ids = vec!["3".to_string(), "8".to_string()];
    for input in ["nope\n", "0\n", "3\n"] {
        let mut stdin = Cursor::new(input.as_bytes().to_vec());
        let mut stderr = Vec::new();
        let err = choose_from("cand", &ids, true, &mut stdin, &mut stderr)
            .unwrap_err();
        assert!(matches!(err, OpenError::InvalidChoice));
    }
}

#[test]
fn several_ids_invalid_choice_does_not_create() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let cwd = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![json_out(
        0,
        &pane_list_body(vec![
            pane("3", Some(&cwd), None, None),
            pane("8", Some(&cwd), None, None),
        ]),
    )]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = |_c: &str, _ids: &[String]| Err(OpenError::InvalidChoice);

    let err = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap_err();

    assert!(matches!(err, OpenError::InvalidChoice));
    assert_eq!(runner.calls.len(), 1);
}

#[test]
fn observed_focus_not_found_is_disappeared_not_created() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let cwd = project.canonicalize().unwrap().display().to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(0, &pane_list_body(vec![pane("3", Some(&cwd), None, None)])),
        focus_err_not_found("3"),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    match open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    ) {
        Err(OpenError::Disappeared { workspace_id }) => {
            assert_eq!(workspace_id, "3");
        }
        other => panic!("expected Disappeared, got {other:?}"),
    }
    assert_eq!(runner.calls.len(), 2);
    assert_eq!(runner.calls[1].1, ["workspace", "focus", "3"]);
}

#[test]
fn pane_list_missing_panes_errors_without_create() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let body = json!({
        "id": "cli:pane:list",
        "result": { "type": "pane_list" }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![json_out(0, &body)]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    let err = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap_err();

    assert!(matches!(err, OpenError::MalformedResult));
    assert_eq!(runner.calls.len(), 1);
}

#[test]
fn pane_list_wrong_type_errors_without_create() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let body = json!({
        "id": "cli:pane:list",
        "result": { "type": "workspace_list", "panes": [] }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![json_out(0, &body)]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    let err = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap_err();

    assert!(matches!(err, OpenError::MalformedResult));
    assert_eq!(runner.calls.len(), 1);
}

#[test]
fn focus_missing_workspace_id_errors() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let cwd = project.canonicalize().unwrap().display().to_string();
    let focus = json!({
        "id": "cli:workspace:focus",
        "result": { "type": "workspace_info", "workspace": {} }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(0, &pane_list_body(vec![pane("3", Some(&cwd), None, None)])),
        json_out(0, &focus),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    let err = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap_err();

    assert!(matches!(err, OpenError::MalformedResult));
}

#[test]
fn create_wrong_type_errors() {
    let d = TempDir::new();
    let project = d.child("work/foo");
    let create = json!({
        "id": "cli:workspace:create",
        "result": { "type": "workspace_info", "workspace": { "workspace_id": "4" } }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![
        json_out(0, &pane_list_body(vec![])),
        json_out(0, &create),
    ]);
    let env = open_env(&d.path().display().to_string());
    let mut chooser = choose_cancel();

    let err = open_with(
        &project.display().to_string(),
        &mut runner,
        &env,
        &mut chooser,
    )
    .unwrap_err();

    assert!(matches!(err, OpenError::MalformedResult));
}

#[test]
fn pane_cwds_collects_cwd_and_foreground_without_duplicates() {
    let mut runner = ScriptRunner::new(vec![json_out(
        0,
        &pane_list_body(vec![
            pane("1", Some("/work/a"), Some("/work/b"), None),
            pane("2", Some("/work/a"), None, None),
            pane("3", Some(""), Some("/work/c"), None),
        ]),
    )]);
    let env = test_env(&[]);
    let cwds = pane_cwds_with(&mut runner, &env).unwrap();
    assert_eq!(cwds, vec!["/work/a", "/work/b", "/work/c"]);
}

#[test]
fn pane_cwds_wrong_type_is_garbage() {
    let body = json!({
        "id": "cli:pane:list",
        "result": { "type": "workspace_info", "panes": [] }
    })
    .to_string();
    let mut runner = ScriptRunner::new(vec![json_out(0, &body)]);
    let env = test_env(&[]);
    let err = pane_cwds_with(&mut runner, &env).unwrap_err();
    assert!(matches!(err, HerdrError::GarbageJson));
}
