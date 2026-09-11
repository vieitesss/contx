use super::{ToastKind, modal_rect, render_dialog, render_toast};
use crate::theme::Theme;
use crate::tui::action::{ActionDialog, CloneDestProbe, GRACE_FOR};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
};
use std::ffi::OsString;
use std::time::Instant;

const T: Theme = Theme::LIGHT;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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

fn clone_dialog(parent: Option<&str>) -> ActionDialog {
    ActionDialog::open_clone(parent.map(str::to_string), Probe::new(&[], &[]))
}

fn paint(dialog: &ActionDialog, w: u16, h: u16) -> Buffer {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, ratatui::style::Style::new().bg(T.bg).fg(T.fg));
    render_dialog(dialog, area, &mut buf);
    buf
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let w = buf.area.width;
    (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

fn buf_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y).trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn type_text(dialog: &mut ActionDialog, s: &str) {
    for c in s.chars() {
        dialog.handle_key(key(KeyCode::Char(c)));
    }
}

#[test]
fn modal_rect_is_about_seventy_by_seventy_six_with_margin() {
    let area = Rect::new(0, 0, 100, 50);
    let rect = modal_rect(area);
    assert_eq!(rect.width, 70);
    assert!(
        rect.height <= 41,
        "cap 82% of 50 is 41, got {}",
        rect.height
    );
    assert_eq!(rect.height, 38); // 76% of 50
    assert!(rect.x >= 2, "left margin");
    assert!(area.width - (rect.x + rect.width) >= 2, "right margin");
    assert!(rect.y >= 1, "top margin");
    assert!(area.height - (rect.y + rect.height) >= 1, "bottom margin");
}

#[test]
fn clone_form_double_border_title_waiting_and_invalid_clone() {
    let dialog = clone_dialog(Some("/work"));
    let buf = paint(&dialog, 80, 24);
    let text = buf_text(&buf);
    assert!(text.contains("Clone"), "title: {text}");
    assert!(text.contains("Source & destination"), "{text}");
    assert!(text.contains("Authenticate"), "{text}");
    assert!(text.contains("waiting"), "{text}");
    assert!(text.contains("[Cancel]"), "{text}");
    assert!(text.contains("Clone (invalid)"), "{text}");
    // Double border uses ╔ on the title row.
    let top = row_text(&buf, modal_rect(Rect::new(0, 0, 80, 24)).y);
    assert!(
        top.contains('╔') || top.contains('═'),
        "double border: {top}"
    );
}

#[test]
fn current_stage_body_uses_bg_alt() {
    let dialog = clone_dialog(Some("/work"));
    let buf = paint(&dialog, 80, 24);
    let rect = modal_rect(Rect::new(0, 0, 80, 24));
    let mut saw_alt = false;
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            if buf[(x, y)].bg == T.bg_alt {
                saw_alt = true;
                break;
            }
        }
    }
    assert!(saw_alt, "expanded current stage should tint bg_alt");
}

#[test]
fn completed_stage_collapses_to_summary() {
    let mut dialog = clone_dialog(Some("/work"));
    type_text(&mut dialog, "git@x:y.git");
    dialog.handle_key(key(KeyCode::Enter));
    let buf = paint(&dialog, 80, 24);
    let text = buf_text(&buf);
    assert!(text.contains("→ /work/git@x:y"), "{text}");
    assert!(text.contains("Cancel git"), "{text}");
}

#[test]
fn host_key_shows_accept_reject() {
    let mut dialog = clone_dialog(Some("/work"));
    dialog.set_log(vec![
        "The authenticity of host 'github.com' can't be established.".into(),
    ]);
    dialog.show_host_key();
    let buf = paint(&dialog, 80, 24);
    let text = buf_text(&buf);
    assert!(text.contains("[Reject]"), "{text}");
    assert!(text.contains("[Accept]"), "{text}");
    assert!(text.contains("host-key"), "{text}");
    assert!(text.contains("authenticity of host"), "{text}");
}

#[test]
fn passphrase_field_is_masked() {
    let mut dialog = clone_dialog(Some("/work"));
    dialog.show_native_prompt();
    type_text(&mut dialog, "hunter2");
    let buf = paint(&dialog, 80, 24);
    let text = buf_text(&buf);
    assert!(!text.contains("hunter2"), "secret leaked: {text}");
    assert!(text.contains('•'), "expected bullets: {text}");
    assert!(text.contains("passphrase"), "{text}");
}

#[test]
fn findings_are_scrollable() {
    let mut dialog = ActionDialog::open_delete("/work/repo".into());
    let findings: Vec<String> =
        (0..12).map(|i| format!("warning-{i}")).collect();
    dialog.present_findings(findings, false);
    let before = buf_text(&paint(&dialog, 80, 24));
    assert!(before.contains("warning-0"), "{before}");
    for _ in 0..6 {
        dialog.handle_key(key(KeyCode::Down));
    }
    let after = buf_text(&paint(&dialog, 80, 24));
    assert_ne!(after, before, "scroll should move findings");
    assert!(
        after.contains("warning-6") || !after.contains("warning-0"),
        "{after}"
    );
}

#[test]
fn permanent_confirm_shows_exact_path_field() {
    let mut dialog = ActionDialog::open_delete("/tmp/scratch".into());
    dialog.show_permanent_confirm();
    type_text(&mut dialog, "/tmp/nope");
    let buf = paint(&dialog, 80, 28);
    let text = buf_text(&buf);
    assert!(text.contains("Permanently delete"), "{text}");
    assert!(text.contains("Type the exact path"), "{text}");
    assert!(text.contains("/tmp/nope"), "{text}");
    assert!(text.contains("path does not match"), "{text}");
    assert!(text.contains("Delete permanently"), "{text}");
}

#[test]
fn toast_success_is_green_cancel_is_accent() {
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, ratatui::style::Style::new().bg(T.bg));
    render_toast(area, &mut buf, ToastKind::Success, "cloned to `/work/x`");
    let text = buf_text(&buf);
    assert!(text.contains("cloned to `/work/x`"), "{text}");
    assert!(text.contains('✓'), "{text}");
    let mut saw_green = false;
    for y in 0..area.height {
        for x in 0..area.width {
            if buf[(x, y)].fg == T.green {
                saw_green = true;
            }
        }
    }
    assert!(saw_green, "success toast uses green");

    let mut buf = Buffer::empty(area);
    buf.set_style(area, ratatui::style::Style::new().bg(T.bg));
    render_toast(area, &mut buf, ToastKind::Cancel, "clone cancelled");
    let text = buf_text(&buf);
    assert!(text.contains("clone cancelled"), "{text}");
    let mut saw_accent = false;
    for y in 0..area.height {
        for x in 0..area.width {
            if buf[(x, y)].fg == T.accent {
                saw_accent = true;
            }
        }
    }
    assert!(saw_accent, "cancel toast uses accent");
}

#[test]
fn force_stop_is_the_only_action_after_grace() {
    let mut dialog = clone_dialog(Some("/work"));
    type_text(&mut dialog, "src.git");
    dialog.handle_key(key(KeyCode::Enter));
    dialog.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
    let t0 = Instant::now();
    dialog.tick(t0);
    dialog.tick(t0 + GRACE_FOR);
    let buf = paint(&dialog, 80, 24);
    let text = buf_text(&buf);
    assert!(text.contains("[Force Stop]"), "{text}");
    assert!(!text.contains("[Cancel git]"), "{text}");
}

#[test]
fn delete_target_shows_class_and_strategy() {
    let dialog = ActionDialog::open_delete("/work/skills".into());
    let buf = paint(&dialog, 80, 24);
    let text = buf_text(&buf);
    assert!(text.contains("Delete"), "{text}");
    assert!(text.contains("Target & strategy"), "{text}");
    assert!(text.contains("/work/skills"), "{text}");
    assert!(text.contains("trash"), "{text}");
    assert!(text.contains("[Preflight]"), "{text}");
}
