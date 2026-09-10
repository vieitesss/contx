use super::pty::{FakePty, PtyEvent, PtySize, PtyTransport};
use super::{
    ActionDialog, CancelState, CloneDestProbe, CloneStage, DeleteStage,
    DialogOutcome, FocusItem, GRACE_FOR,
};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use std::ffi::OsString;
use std::time::{Duration, Instant};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn attach_fake(dialog: &mut ActionDialog) -> FakePty {
    let mut fake = FakePty::new();
    let session = fake
        .spawn(&["git", "clone"], PtySize { cols: 80, rows: 24 })
        .expect("fake spawn");
    dialog.set_child(session);
    fake
}

fn shift_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)
}

fn type_text(dialog: &mut ActionDialog, s: &str) {
    for c in s.chars() {
        dialog.handle_key(key(KeyCode::Char(c)));
    }
}

struct Probe {
    existing: Vec<String>,
    covered: Vec<String>,
    env: Vec<(String, String)>,
}

impl Probe {
    fn new(existing: &[&str], covered: &[&str]) -> Self {
        Self {
            existing: existing.iter().map(|s| s.to_string()).collect(),
            covered: covered.iter().map(|s| s.to_string()).collect(),
            env: vec![],
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

    fn env(&self, name: &str) -> Option<OsString> {
        self.env
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| OsString::from(v))
    }
}

fn clone_dialog(
    parent: Option<&str>,
    existing: &[&str],
    covered: &[&str],
) -> ActionDialog {
    ActionDialog::open_clone(
        parent.map(str::to_string),
        Probe::new(existing, covered),
    )
}

fn fill_valid_clone(dialog: &mut ActionDialog) {
    assert_eq!(dialog.item(), Some(FocusItem::Source));
    type_text(dialog, "git@example.com:acme/repo.git");
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.item(), Some(FocusItem::Dest));
    type_text(dialog, "repo");
}

#[test]
fn field_insert_backspace_delete_and_cursor() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    type_text(&mut dialog, "ab");
    assert_eq!(dialog.source(), "ab");
    assert_eq!(dialog.cursor(), 2);
    dialog.handle_key(key(KeyCode::Left));
    dialog.handle_key(key(KeyCode::Char('X')));
    assert_eq!(dialog.source(), "aXb");
    assert_eq!(dialog.cursor(), 2);
    dialog.handle_key(key(KeyCode::Backspace));
    assert_eq!(dialog.source(), "ab");
    dialog.handle_key(key(KeyCode::Home));
    dialog.handle_key(key(KeyCode::Delete));
    assert_eq!(dialog.source(), "b");
    dialog.handle_key(key(KeyCode::End));
    dialog.handle_key(key(KeyCode::Char('c')));
    assert_eq!(dialog.source(), "bc");
}

#[test]
fn clone_has_four_named_stages() {
    let dialog = clone_dialog(Some("/work"), &[], &[]);
    assert_eq!(dialog.stage_n(), 4);
    assert_eq!(dialog.current_stage(), 0);
    assert_eq!(dialog.stage_title(0), CloneStage::SourceDest.title());
    assert_eq!(dialog.stage_title(1), CloneStage::Authenticate.title());
    assert_eq!(dialog.stage_title(2), CloneStage::Clone.title());
    assert_eq!(dialog.stage_title(3), CloneStage::Result.title());
    assert_eq!(dialog.stage_title(0), "Source & destination");
    assert_eq!(dialog.stage_title(1), "Authenticate");
    assert_eq!(dialog.stage_title(2), "Clone");
    assert_eq!(dialog.stage_title(3), "Result");
}

#[test]
fn delete_has_five_named_stages() {
    let dialog = ActionDialog::open_delete("/work/repo".into());
    assert_eq!(dialog.stage_n(), 5);
    assert_eq!(dialog.current_stage(), 0);
    assert_eq!(dialog.stage_title(0), DeleteStage::Target.title());
    assert_eq!(
        dialog.stage_title(1),
        DeleteStage::RemoteVerification.title()
    );
    assert_eq!(dialog.stage_title(2), DeleteStage::Findings.title());
    assert_eq!(dialog.stage_title(3), DeleteStage::Confirm.title());
    assert_eq!(dialog.stage_title(4), DeleteStage::Delete.title());
    assert_eq!(dialog.stage_title(0), "Target & strategy");
    assert_eq!(dialog.stage_title(1), "Remote verification");
    assert_eq!(dialog.stage_title(2), "Findings");
    assert_eq!(dialog.stage_title(3), "Confirm");
    assert_eq!(dialog.stage_title(4), "Delete");
}

#[test]
fn tab_and_backtab_walk_clone_form_items() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    assert_eq!(
        dialog.items(),
        vec![
            FocusItem::Source,
            FocusItem::Dest,
            FocusItem::Cancel,
            FocusItem::Action,
        ]
    );
    assert_eq!(dialog.item(), Some(FocusItem::Source));
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.item(), Some(FocusItem::Dest));
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.item(), Some(FocusItem::Cancel));
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.item(), Some(FocusItem::Action));
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.item(), Some(FocusItem::Source));
    dialog.handle_key(shift_tab());
    assert_eq!(dialog.item(), Some(FocusItem::Action));
    dialog.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
    assert_eq!(dialog.item(), Some(FocusItem::Cancel));
}

#[test]
fn add_parent_only_when_uncovered_and_toggles() {
    let mut covered = clone_dialog(Some("/work"), &[], &["/work/"]);
    fill_valid_clone(&mut covered);
    assert!(
        !covered.items().contains(&FocusItem::AddParent),
        "covered dest hides add-parent: {:?}",
        covered.items()
    );
    assert!(!covered.add_parent());

    let empty = clone_dialog(Some("/work"), &[], &[]);
    assert!(!empty.items().contains(&FocusItem::AddParent));

    let mut open = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut open);
    assert!(
        open.items().contains(&FocusItem::AddParent),
        "uncovered dest shows add-parent: {:?}",
        open.items()
    );
    open.handle_key(key(KeyCode::Tab));
    assert_eq!(open.item(), Some(FocusItem::AddParent));
    assert!(!open.add_parent());
    open.handle_key(key(KeyCode::Char(' ')));
    assert!(open.add_parent());
    open.handle_key(key(KeyCode::Enter));
    assert!(!open.add_parent());
}

#[test]
fn empty_source_or_dest_blocks_clone() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    assert_eq!(
        dialog.clone_validation_error().as_deref(),
        Some("source is required")
    );
    type_text(&mut dialog, "src.git");
    assert_eq!(
        dialog.clone_validation_error().as_deref(),
        Some("destination is required")
    );
    dialog.handle_key(key(KeyCode::Enter));
    assert!(!dialog.git_started());
    assert_eq!(dialog.current_stage(), 0);
}

#[test]
fn relative_dest_without_parent_blocks_clone() {
    let mut dialog = clone_dialog(None, &[], &[]);
    type_text(&mut dialog, "src.git");
    dialog.handle_key(key(KeyCode::Tab));
    type_text(&mut dialog, "repo");
    let err = dialog.clone_validation_error().expect("relative rejected");
    assert!(
        err.contains("not an absolute path"),
        "expected absolute-path error, got {err}"
    );
    dialog.handle_key(key(KeyCode::Enter));
    assert!(!dialog.git_started());
}

#[test]
fn existing_dest_blocks_clone() {
    let mut dialog = clone_dialog(Some("/work"), &["/work/repo"], &[]);
    fill_valid_clone(&mut dialog);
    let err = dialog.clone_validation_error().expect("exists");
    assert!(err.contains("destination already exists"), "got {err}");
    assert!(err.contains("/work/repo"));
    dialog.handle_key(key(KeyCode::Enter));
    assert!(!dialog.git_started());
}

#[test]
fn valid_form_enter_starts_git_and_freezes_fields() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut dialog);
    assert!(dialog.clone_validation_error().is_none());
    dialog.handle_key(key(KeyCode::Enter));
    assert!(dialog.git_started());
    assert!(dialog.running());
    assert_eq!(dialog.current_stage(), 1);
    assert_eq!(dialog.source(), "git@example.com:acme/repo.git");
    assert_eq!(dialog.dest(), "repo");
    let source = dialog.source().to_string();
    dialog.handle_key(key(KeyCode::Char('z')));
    assert_eq!(dialog.source(), source);
    assert!(!dialog.items().contains(&FocusItem::Source));
    assert!(!dialog.items().contains(&FocusItem::Dest));
    assert!(!dialog.items().contains(&FocusItem::AddParent));
}

#[test]
fn prompt_and_perm_fields_edit() {
    let mut clone = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut clone);
    clone.handle_key(key(KeyCode::Enter));
    clone.show_native_prompt();
    assert_eq!(clone.item(), Some(FocusItem::Prompt));
    type_text(&mut clone, "secret");
    assert_eq!(clone.prompt(), "secret");
    clone.handle_key(key(KeyCode::Backspace));
    assert_eq!(clone.prompt(), "secre");

    let mut delete = ActionDialog::open_delete("/tmp/scratch".into());
    delete.show_permanent_confirm();
    assert_eq!(delete.item(), Some(FocusItem::PermPath));
    type_text(&mut delete, "/tmp/scratch");
    assert_eq!(delete.perm(), "/tmp/scratch");
    delete.handle_key(key(KeyCode::Home));
    delete.handle_key(key(KeyCode::Delete));
    assert_eq!(delete.perm(), "tmp/scratch");
}

#[test]
fn completed_stages_reopen_read_only() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut dialog);
    dialog.handle_key(key(KeyCode::Enter));
    assert_eq!(dialog.current_stage(), 1);
    assert_eq!(dialog.inspect(), None);
    dialog.handle_key(key(KeyCode::Char('[')));
    assert_eq!(dialog.selected_stage(), 0);
    dialog.handle_key(key(KeyCode::Char('i')));
    assert_eq!(dialog.inspect(), Some(0));
    let source = dialog.source().to_string();
    dialog.handle_key(key(KeyCode::Char('x')));
    assert_eq!(dialog.source(), source, "inspected form stays frozen");
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(dialog.inspect(), None);
    dialog.handle_key(key(KeyCode::Char(']')));
    dialog.handle_key(key(KeyCode::Char(']')));
    assert_eq!(dialog.selected_stage(), 2);
    dialog.handle_key(key(KeyCode::Char('i')));
    assert_eq!(dialog.inspect(), None, "future stages are not inspectable");
}

#[test]
fn esc_before_child_yields_cancelled() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    let before = dialog.generation();
    let out = dialog.handle_key(key(KeyCode::Esc));
    assert_eq!(out, Some(DialogOutcome::Cancelled));
    assert_eq!(dialog.outcome(), Some(&DialogOutcome::Cancelled));
    assert!(dialog.generation() > before);

    let mut delete = ActionDialog::open_delete("/work/repo".into());
    assert_eq!(
        delete.handle_key(key(KeyCode::Esc)),
        Some(DialogOutcome::Cancelled)
    );
}

#[test]
fn esc_while_running_does_not_dismiss() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut dialog);
    dialog.handle_key(key(KeyCode::Enter));
    let before = dialog.generation();
    assert_eq!(dialog.handle_key(key(KeyCode::Esc)), None);
    assert!(dialog.running());
    assert_eq!(dialog.generation(), before);
    assert!(dialog.hint().contains("Esc does not stop git"));
}

#[test]
fn esc_on_error_stays_until_ack() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    dialog.present_error("git clone failed");
    let before = dialog.generation();
    assert_eq!(dialog.handle_key(key(KeyCode::Esc)), None);
    assert_eq!(dialog.generation(), before);
    assert!(dialog.hint().contains("acknowledged"));
    match dialog.outcome() {
        Some(DialogOutcome::Failed { message }) => {
            assert_eq!(message, "git clone failed");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    let out = dialog.handle_key(key(KeyCode::Enter));
    assert_eq!(
        out,
        Some(DialogOutcome::Failed {
            message: "git clone failed".into(),
        })
    );
    assert!(dialog.generation() > before);
}

#[test]
fn dialog_outcome_distinguishes_cancelled_failed_completed_and_ancillary() {
    assert!(!DialogOutcome::Cancelled.needs_ack());
    assert!(
        DialogOutcome::Failed {
            message: "x".into(),
        }
        .needs_ack()
    );
    let full = DialogOutcome::Completed {
        summary: "cloned to `/work/repo`".into(),
        config_error: None,
        refresh_error: None,
    };
    assert!(!full.needs_ack());
    let config = DialogOutcome::Completed {
        summary: "cloned to `/work/repo`".into(),
        config_error: Some("write failed".into()),
        refresh_error: None,
    };
    assert!(config.needs_ack());
    let refresh = DialogOutcome::Completed {
        summary: "cloned to `/work/repo`".into(),
        config_error: None,
        refresh_error: Some("reread failed".into()),
    };
    assert!(refresh.needs_ack());

    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    dialog.present_completed("cloned to `/work/repo`", None, None);
    assert_eq!(dialog.outcome(), Some(&full));
    assert!(!dialog.outcome().unwrap().needs_ack());

    let mut partial = clone_dialog(Some("/work"), &[], &[]);
    partial.present_completed(
        "cloned to `/work/repo`",
        Some("write failed".into()),
        None,
    );
    assert_eq!(partial.handle_key(key(KeyCode::Esc)), None);
    let out = partial.handle_key(key(KeyCode::Enter));
    assert_eq!(out, Some(config));
}

#[test]
fn key_release_is_ignored() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    let mut release = key(KeyCode::Char('a'));
    release.kind = KeyEventKind::Release;
    dialog.handle_key(release);
    assert_eq!(dialog.source(), "");
}

#[test]
fn ctrl_g_before_child_does_not_bump_or_interrupt() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    let fake = attach_fake(&mut dialog);
    let before = dialog.generation();
    dialog.handle_key(ctrl('g'));
    assert_eq!(dialog.cancel_state(), CancelState::Idle);
    assert_eq!(dialog.generation(), before);
    assert_eq!(fake.interrupts(), 0);
    assert_eq!(fake.force_kills(), 0);
}

#[test]
fn ctrl_g_interrupts_once_and_enters_grace() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut dialog);
    dialog.handle_key(key(KeyCode::Enter));
    let fake = attach_fake(&mut dialog);
    let before = dialog.generation();
    dialog.handle_key(ctrl('g'));
    assert_eq!(dialog.cancel_state(), CancelState::Grace);
    assert_eq!(dialog.generation(), before);
    assert_eq!(fake.interrupts(), 1);
    assert_eq!(fake.force_kills(), 0);
    assert!(dialog.items().is_empty());
    dialog.handle_key(ctrl('g'));
    assert_eq!(fake.interrupts(), 1);
    assert_eq!(dialog.handle_key(key(KeyCode::Esc)), None);
    assert_eq!(fake.interrupts(), 1);
    assert_eq!(fake.force_kills(), 0);
    assert_eq!(dialog.generation(), before);
    assert!(dialog.hint().contains("Esc does not stop git"));
}

#[test]
fn grace_then_force_stop_kills() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut dialog);
    dialog.handle_key(key(KeyCode::Enter));
    let fake = attach_fake(&mut dialog);
    dialog.handle_key(ctrl('g'));
    let t0 = Instant::now();
    dialog.tick(t0);
    assert_eq!(dialog.cancel_state(), CancelState::Grace);
    dialog.tick(t0 + GRACE_FOR - Duration::from_millis(1));
    assert_eq!(dialog.cancel_state(), CancelState::Grace);
    dialog.tick(t0 + GRACE_FOR);
    assert_eq!(dialog.cancel_state(), CancelState::ForceReady);
    assert_eq!(dialog.items(), vec![FocusItem::ForceStop]);
    assert_eq!(dialog.item(), Some(FocusItem::ForceStop));
    assert_eq!(fake.force_kills(), 0);
    dialog.handle_key(key(KeyCode::Enter));
    assert_eq!(fake.force_kills(), 1);
    assert_eq!(fake.interrupts(), 1);
    assert!(dialog.running());
    fake.inject(PtyEvent::Exit { code: Some(1) });
    assert_eq!(dialog.pump(), Some(DialogOutcome::Cancelled));
}

#[test]
fn exit_during_grace_cancels_without_force_kill() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut dialog);
    dialog.handle_key(key(KeyCode::Enter));
    let fake = attach_fake(&mut dialog);
    let before = dialog.generation();
    dialog.handle_key(ctrl('g'));
    fake.inject(PtyEvent::Exit { code: Some(0) });
    assert_eq!(dialog.pump(), Some(DialogOutcome::Cancelled));
    assert_eq!(fake.force_kills(), 0);
    assert!(dialog.generation() > before);
    assert!(!dialog.running());
}

#[test]
fn stale_exit_after_generation_bump_is_ignored() {
    let mut dialog = clone_dialog(Some("/work"), &[], &[]);
    fill_valid_clone(&mut dialog);
    dialog.handle_key(key(KeyCode::Enter));
    let fake = attach_fake(&mut dialog);
    dialog.bump_generation();
    fake.inject(PtyEvent::Exit { code: Some(0) });
    assert_eq!(dialog.pump(), None);
    assert_eq!(dialog.outcome(), None);
    assert_eq!(dialog.cancel_state(), CancelState::Idle);
}
