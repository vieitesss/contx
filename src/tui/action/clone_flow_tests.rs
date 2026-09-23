use super::{ConfigAppendEvent, FakeConfigAppend, git_clone_argv};
use crate::tui::action::pty::{FakePty, PtyEvent, PtySize};
use crate::tui::action::{
    ActionDialog, CloneDestProbe, DialogOutcome, FocusItem,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::ffi::OsString;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn type_text(dialog: &mut ActionDialog, s: &str) {
    for c in s.chars() {
        dialog.handle_key(key(KeyCode::Char(c)));
    }
}

struct Probe {
    existing: Vec<String>,
    covered: Vec<String>,
}

impl Probe {
    fn new(existing: &[&str], covered: &[&str]) -> Self {
        Self {
            existing: existing.iter().map(|s| s.to_string()).collect(),
            covered: covered.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl CloneDestProbe for Probe {
    fn exists(&self, abs: &str) -> bool {
        self.existing.iter().any(|e| e == abs)
    }

    fn covered(&self, abs: &str) -> bool {
        self.covered.iter().any(|c| abs == c || abs.starts_with(c))
    }

    fn env(&self, _name: &str) -> Option<OsString> {
        None
    }
}

fn dialog() -> ActionDialog {
    ActionDialog::open_clone(Some("/work".into()), Probe::new(&[], &[]))
}

fn fill_valid(dialog: &mut ActionDialog) {
    type_text(dialog, "acme/repo.git");
    assert_eq!(dialog.dest(), "repo");
}

fn start_with_fake() -> (ActionDialog, FakePty) {
    let mut d = dialog();
    let fake = FakePty::new();
    d.set_transport(Box::new(fake.clone()));
    fill_valid(&mut d);
    d.handle_key(key(KeyCode::Enter));
    (d, fake)
}

#[test]
fn git_clone_argv_has_no_extra_flags() {
    assert_eq!(
        git_clone_argv("src", "/dest"),
        ["git", "clone", "src", "/dest"]
    );
}

#[test]
fn dest_exists_is_exported() {
    assert!(!crate::clone::dest_exists("/no/such/contx-clone-dest"));
}

#[test]
fn invalid_clone_does_not_spawn() {
    for source in [
        "abc",
        "git@github.com:owner/repo",
        "git@github.com/owner/repo",
        "github.com/owner/repo",
    ] {
        let mut d = dialog();
        let fake = FakePty::new();
        d.set_transport(Box::new(fake.clone()));
        type_text(&mut d, source);
        d.focus_item(FocusItem::ProtocolSsh);
        d.handle_key(key(KeyCode::Enter));
        assert!(fake.spawns().is_empty(), "source {source:?}");
        assert_eq!(
            d.clone_validation_error().as_deref(),
            Some("repository path must include owner/repo"),
            "source {source:?}"
        );
        assert!(!d.git_started(), "source {source:?}");
    }
}

#[test]
fn enter_from_protocol_or_option_control_starts_valid_clone() {
    for focused in [FocusItem::ProtocolSsh, FocusItem::PresetsToggle] {
        let mut d = dialog();
        let fake = FakePty::new();
        d.set_transport(Box::new(fake.clone()));
        fill_valid(&mut d);
        d.focus_item(focused);
        d.handle_key(key(KeyCode::Enter));
        assert_eq!(fake.spawns().len(), 1, "focused {focused:?}");
        assert!(d.git_started(), "focused {focused:?}");
    }
}

#[test]
fn valid_clone_spawns_git_clone_and_freezes() {
    let (d, fake) = start_with_fake();
    assert_eq!(
        fake.spawns(),
        vec![(
            vec![
                "git".into(),
                "clone".into(),
                "git@github.com:acme/repo.git".into(),
                "/work/repo".into(),
            ],
            PtySize { cols: 80, rows: 24 }
        )]
    );
    assert!(d.git_started());
    assert!(d.running());
}

#[test]
fn host_key_accept_writes_yes() {
    let (mut d, fake) = start_with_fake();
    fake.inject(PtyEvent::Output(
        b"Are you sure you want to continue connecting (yes/no/[fingerprint])? "
            .to_vec(),
    ));
    d.pump();
    assert_eq!(d.item(), Some(FocusItem::AcceptKey));
    d.handle_key(key(KeyCode::Enter));
    assert!(
        fake.writes().iter().any(|w| w == b"yes\n"),
        "writes: {:?}",
        fake.writes()
    );
}

#[test]
fn passphrase_secret_is_written_but_not_logged() {
    let (mut d, fake) = start_with_fake();
    fake.inject(PtyEvent::Output(
        b"Enter passphrase for key '/tmp/id': ".to_vec(),
    ));
    d.pump();
    assert_eq!(d.item(), Some(FocusItem::Prompt));
    type_text(&mut d, "hunter2");
    d.handle_key(key(KeyCode::Tab));
    assert_eq!(d.item(), Some(FocusItem::Action));
    d.handle_key(key(KeyCode::Enter));
    assert!(
        fake.writes().iter().any(|w| w == b"hunter2\n"),
        "writes: {:?}",
        fake.writes()
    );
    assert!(
        d.log().iter().all(|l| !l.contains("hunter2")),
        "log leaked secret: {:?}",
        d.log()
    );
}

#[test]
fn unknown_output_forwards_keys() {
    let (mut d, fake) = start_with_fake();
    fake.inject(PtyEvent::Output(
        b"Receiving objects:  62% (2165/3492)\r\n".to_vec(),
    ));
    d.pump();
    d.handle_key(key(KeyCode::Char('x')));
    d.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert!(
        fake.writes().iter().any(|w| w == b"x"),
        "writes: {:?}",
        fake.writes()
    );
    assert!(
        fake.writes().iter().any(|w| w == b"\x17"),
        "Ctrl-W was not forwarded: {:?}",
        fake.writes()
    );
}

#[test]
fn git_success_without_add_parent_completes_and_requests_refresh() {
    let (mut d, fake) = start_with_fake();
    fake.inject(PtyEvent::Exit { code: Some(0) });
    let out = d.pump();
    assert_eq!(
        out,
        Some(DialogOutcome::Completed {
            summary: "cloned to `/work/repo`".into(),
            config_error: None,
            refresh_error: None,
        })
    );
    assert!(d.refresh_requested());
    assert!(!out.unwrap().needs_ack());
}

#[test]
fn git_success_config_failure_is_partial_sticky_and_still_refreshes() {
    let mut d = dialog();
    let fake = FakePty::new();
    let cfg = FakeConfigAppend::new();
    d.set_transport(Box::new(fake.clone()));
    d.set_config_append(Box::new(cfg.clone()));
    fill_valid(&mut d);
    for _ in 0..3 {
        d.handle_key(key(KeyCode::Tab));
    }
    assert_eq!(d.item(), Some(FocusItem::AddParent));
    d.handle_key(key(KeyCode::Char(' ')));
    assert!(d.add_parent());
    d.handle_key(key(KeyCode::Enter));
    fake.inject(PtyEvent::Exit { code: Some(0) });
    assert_eq!(d.pump(), None);
    assert!(d.refresh_requested());
    let begins = cfg.begins();
    assert_eq!(begins.len(), 1);
    assert_eq!(begins[0].0, "/work/repo");
    cfg.inject(ConfigAppendEvent {
        generation: begins[0].1,
        dest: "/work/repo".into(),
        result: Err("write failed".into()),
    });
    let out = d.pump();
    assert_eq!(out, None, "partial stays until ack");
    match d.outcome() {
        Some(DialogOutcome::Completed {
            summary,
            config_error: Some(err),
            refresh_error: None,
        }) => {
            assert!(summary.contains("/work/repo"));
            assert_eq!(err, "write failed");
        }
        other => panic!("expected partial completed, got {other:?}"),
    }
    assert!(d.outcome().unwrap().needs_ack());
    let ack = d.handle_key(key(KeyCode::Enter));
    assert!(matches!(
        ack,
        Some(DialogOutcome::Completed {
            config_error: Some(_),
            ..
        })
    ));
}

#[test]
fn git_failure_does_not_refresh_or_rollback() {
    let (mut d, fake) = start_with_fake();
    fake.inject(PtyEvent::Exit { code: Some(128) });
    assert_eq!(d.pump(), None);
    assert!(!d.refresh_requested());
    match d.outcome() {
        Some(DialogOutcome::Failed { message }) => {
            assert!(message.contains("git clone failed"));
            assert!(message.contains("/work/repo"));
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn interrupt_cancel_does_not_request_refresh() {
    let (mut d, fake) = start_with_fake();
    d.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
    fake.inject(PtyEvent::Exit { code: Some(1) });
    assert_eq!(d.pump(), Some(DialogOutcome::Cancelled));
    assert!(!d.refresh_requested());
}
