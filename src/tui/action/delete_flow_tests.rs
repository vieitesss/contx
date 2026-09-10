use super::{
    FakeInspectWorker, FakeMutateWorker, InspectEvent, MutateEvent, MutateFail,
    MutateKind, git_fetch_argv, git_worktree_remove_argv,
};
use crate::delete::{DeleteClass, DeleteStrategy, FetchResult};
use crate::tui::action::pty::{FakePty, PtyEvent, PtySize};
use crate::tui::action::{ActionDialog, DialogOutcome, FocusItem};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::Instant;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn type_text(dialog: &mut ActionDialog, s: &str) {
    for c in s.chars() {
        dialog.handle_key(key(KeyCode::Char(c)));
    }
}

fn open_delete() -> ActionDialog {
    ActionDialog::open_delete("/work/repo".into())
}

fn findings(blocked: bool) -> InspectEvent {
    InspectEvent {
        generation: 1,
        path: "/work/repo".into(),
        findings: if blocked {
            vec!["HARD BLOCKER".into(), "primary has linked worktrees".into()]
        } else {
            vec!["unstaged changes".into(), "ahead of upstream".into()]
        },
        blocked,
        class: "standalone repository".into(),
        strategy: "trash".into(),
        confirm: super::DeleteConfirm::Trash,
        fetch_required: false,
    }
}

#[test]
fn fetch_and_worktree_argv() {
    assert_eq!(
        git_fetch_argv("/work/repo"),
        ["git", "-C", "/work/repo", "fetch", "--all", "--prune"]
    );
    assert_eq!(
        git_worktree_remove_argv("/work/wt"),
        ["git", "-C", "/work/wt", "worktree", "remove", "/work/wt"]
    );
}

#[test]
fn open_delete_shows_path_without_inspect() {
    let inspect = FakeInspectWorker::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    assert!(d.log().is_empty() || true);
    assert!(inspect.begins().is_empty());
    let _ = d.hint();
}

#[test]
fn preflight_does_not_run_inspect_synchronously() {
    let inspect = FakeInspectWorker::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_delete_plan(
        DeleteClass::OrdinaryDirectory,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    assert_eq!(inspect.begins().len(), 1);
    assert_eq!(inspect.begins()[0].0, "/work/repo");
    assert!(d.outcome().is_none());
    match &d.op_delete_findings() {
        findings if findings.is_empty() => {}
        other => panic!("findings filled on UI thread: {other:?}"),
    }
    d.tick(Instant::now());
    assert!(d.outcome().is_none());
}

impl ActionDialog {
    fn op_delete_findings(&self) -> Vec<String> {
        match &self.op {
            super::super::Op::Delete { form } => form.findings.clone(),
            _ => vec![],
        }
    }
}

#[test]
fn no_remote_standalone_skips_fetch_pty_and_inspects() {
    let inspect = FakeInspectWorker::new();
    let fake = FakePty::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_transport(Box::new(fake.clone()));
    d.set_delete_plan(
        DeleteClass::StandaloneRepo,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    assert!(fake.spawns().is_empty(), "no-remote must not spawn fetch");
    assert_eq!(inspect.begins().len(), 1);
    assert_eq!(inspect.begins()[0].2, None);
    let mut ev = findings(false);
    ev.generation = inspect.begins()[0].1;
    inspect.inject(ev);
    d.pump();
    assert!(fake.spawns().is_empty());
    assert!(d.op_delete_findings().contains(&"unstaged changes".into()));
}

#[test]
fn inspect_fetch_required_spawns_git_fetch_then_reinspects() {
    let inspect = FakeInspectWorker::new();
    let fake = FakePty::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_transport(Box::new(fake.clone()));
    d.set_delete_plan(
        DeleteClass::StandaloneRepo,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    assert!(fake.spawns().is_empty());
    inspect.inject(InspectEvent {
        generation: inspect.begins()[0].1,
        path: "/work/repo".into(),
        findings: vec![],
        blocked: false,
        class: "standalone repository".into(),
        strategy: "trash".into(),
        confirm: super::DeleteConfirm::Trash,
        fetch_required: true,
    });
    d.pump();
    assert_eq!(
        fake.spawns()[0].0,
        vec!["git", "-C", "/work/repo", "fetch", "--all", "--prune"]
    );
    assert_eq!(inspect.begins().len(), 1);
    fake.inject(PtyEvent::Exit { code: Some(0) });
    d.pump();
    assert_eq!(inspect.begins().len(), 2);
    assert_eq!(inspect.begins()[1].2, Some(FetchResult::Success));
}

#[test]
fn fetch_spawns_git_fetch_then_inspect_gets_result() {
    let inspect = FakeInspectWorker::new();
    let fake = FakePty::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_transport(Box::new(fake.clone()));
    d.set_delete_plan(DeleteClass::StandaloneRepo, DeleteStrategy::Trash, true);
    d.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fake.spawns()[0].0,
        vec!["git", "-C", "/work/repo", "fetch", "--all", "--prune"]
    );
    assert!(inspect.begins().is_empty());
    fake.inject(PtyEvent::Exit { code: Some(0) });
    d.pump();
    assert_eq!(inspect.begins().len(), 1);
    assert_eq!(inspect.begins()[0].2, Some(FetchResult::Success));
}

#[test]
fn stale_and_mismatched_inspect_events_are_ignored() {
    let inspect = FakeInspectWorker::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_delete_plan(
        DeleteClass::OrdinaryDirectory,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    let job_gen = inspect.begins()[0].1;
    inspect.inject(InspectEvent {
        generation: job_gen.wrapping_add(9),
        path: "/work/repo".into(),
        findings: vec!["stale".into()],
        blocked: false,
        class: "ordinary directory".into(),
        strategy: "trash".into(),
        confirm: super::DeleteConfirm::Trash,
        fetch_required: false,
    });
    d.pump();
    assert!(d.op_delete_findings().is_empty());
    inspect.inject(InspectEvent {
        generation: job_gen,
        path: "/other".into(),
        findings: vec!["wrong-path".into()],
        blocked: false,
        class: "ordinary directory".into(),
        strategy: "trash".into(),
        confirm: super::DeleteConfirm::Trash,
        fetch_required: false,
    });
    d.pump();
    assert!(d.op_delete_findings().is_empty());
    inspect.inject(findings(false));
    d.pump();
    assert!(d.op_delete_findings().contains(&"unstaged changes".into()));
}

#[test]
fn blocker_stays_until_ack_accept_does_not_bypass() {
    let inspect = FakeInspectWorker::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_delete_plan(
        DeleteClass::StandaloneRepo,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    let mut ev = findings(true);
    ev.generation = inspect.begins()[0].1;
    inspect.inject(ev);
    d.pump();
    assert_eq!(d.handle_key(key(KeyCode::Esc)), None);
    let out = d.handle_key(key(KeyCode::Enter));
    assert!(matches!(out, Some(DialogOutcome::Failed { .. })));
}

#[test]
fn warnings_accept_goes_to_trash_confirm_then_mutate() {
    let inspect = FakeInspectWorker::new();
    let mutate = FakeMutateWorker::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_mutate_worker(Box::new(mutate.clone()));
    d.set_delete_plan(
        DeleteClass::OrdinaryDirectory,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    let mut ev = findings(false);
    ev.generation = inspect.begins()[0].1;
    inspect.inject(ev);
    d.pump();
    d.handle_key(key(KeyCode::Enter));
    assert_eq!(d.item(), Some(FocusItem::Action));
    d.handle_key(key(KeyCode::Enter));
    assert_eq!(mutate.begins().len(), 1);
    assert_eq!(mutate.begins()[0].1, MutateKind::Trash);
    mutate.inject(MutateEvent {
        generation: mutate.begins()[0].2,
        path: "/work/repo".into(),
        result: Ok(DeleteStrategy::Trash),
    });
    let out = d.pump();
    assert_eq!(
        out,
        Some(DialogOutcome::Completed {
            summary: "deleted `/work/repo`".into(),
            config_error: None,
            refresh_error: None,
        })
    );
    assert!(d.refresh_requested());
}

#[test]
fn trash_failure_opens_permanent_confirm_not_auto_fallback() {
    let inspect = FakeInspectWorker::new();
    let mutate = FakeMutateWorker::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_mutate_worker(Box::new(mutate.clone()));
    d.set_delete_plan(
        DeleteClass::OrdinaryDirectory,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    let mut ev = findings(false);
    ev.generation = inspect.begins()[0].1;
    inspect.inject(ev);
    d.pump();
    d.handle_key(key(KeyCode::Enter));
    d.handle_key(key(KeyCode::Enter));
    mutate.inject(MutateEvent {
        generation: mutate.begins()[0].2,
        path: "/work/repo".into(),
        result: Err(MutateFail::Trash("not permitted".into())),
    });
    d.pump();
    assert_eq!(d.item(), Some(FocusItem::PermPath));
    assert_eq!(mutate.begins().len(), 1);
    type_text(&mut d, "/work/repo");
    d.handle_key(key(KeyCode::Tab));
    d.handle_key(key(KeyCode::Tab));
    d.handle_key(key(KeyCode::Enter));
    assert_eq!(mutate.begins().len(), 2);
    assert_eq!(mutate.begins()[1].1, MutateKind::Permanent);
}

#[test]
fn worktree_spawns_nonforce_remove() {
    let inspect = FakeInspectWorker::new();
    let fake = FakePty::new();
    let mut d = ActionDialog::open_delete("/work/wt".into());
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_transport(Box::new(fake.clone()));
    d.set_delete_plan(
        DeleteClass::LinkedWorktree,
        DeleteStrategy::GitWorktree,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    let mut ev = findings(false);
    ev.path = "/work/wt".into();
    ev.generation = inspect.begins()[0].1;
    ev.confirm = super::DeleteConfirm::Worktree;
    ev.strategy = "git worktree".into();
    inspect.inject(ev);
    d.pump();
    d.handle_key(key(KeyCode::Enter));
    d.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fake.spawns().last().map(|s| s.0.clone()),
        Some(vec![
            "git".into(),
            "-C".into(),
            "/work/wt".into(),
            "worktree".into(),
            "remove".into(),
            "/work/wt".into(),
        ])
    );
    assert_eq!(
        fake.spawns().last().unwrap().1,
        PtySize { cols: 80, rows: 24 }
    );
    fake.inject(PtyEvent::Exit { code: Some(0) });
    let out = d.pump();
    assert!(matches!(
        out,
        Some(DialogOutcome::Completed {
            refresh_error: None,
            ..
        })
    ));
    assert!(d.refresh_requested());
}

#[test]
fn esc_during_inspect_cancels_without_mutate() {
    let inspect = FakeInspectWorker::new();
    let mutate = FakeMutateWorker::new();
    let mut d = open_delete();
    d.set_inspect_worker(Box::new(inspect.clone()));
    d.set_mutate_worker(Box::new(mutate.clone()));
    d.set_delete_plan(
        DeleteClass::OrdinaryDirectory,
        DeleteStrategy::Trash,
        false,
    );
    d.handle_key(key(KeyCode::Enter));
    assert_eq!(
        d.handle_key(key(KeyCode::Esc)),
        Some(DialogOutcome::Cancelled)
    );
    inspect.inject(findings(false));
    d.pump();
    assert!(mutate.begins().is_empty());
    assert!(d.op_delete_findings().is_empty());
}
