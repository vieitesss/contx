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

fn modal_hint(dialog: &ActionDialog, w: u16, h: u16) -> String {
    let buf = paint(dialog, w, h);
    let row = modal_rect(Rect::new(0, 0, w, h)).y
        + modal_rect(Rect::new(0, 0, w, h)).height
        - 3;
    row_text(&buf, row).trim().to_string()
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
    assert!(text.contains("SSH"), "{text}");
    assert!(text.contains("Repository path"), "{text}");
    assert!(text.contains("Authenticate"), "{text}");
    assert!(text.contains("waiting"), "{text}");
    assert!(text.contains("owner/repo"), "repository path hint: {text}");
    assert!(!text.contains("org/repo"), "old placeholder: {text}");
    let rows: Vec<_> = text.lines().collect();
    let label_row = rows
        .iter()
        .position(|row| row.contains("Repository path"))
        .unwrap();
    assert!(
        rows[label_row].contains("owner/repo"),
        "hint shares label row: {text}"
    );
    assert!(
        !rows[label_row + 1].contains("owner/repo"),
        "input row is empty: {text}"
    );
    assert!(text.contains("repository path is required"), "{text}");
    assert!(!text.contains("[Cancel]"), "{text}");
    assert!(!text.contains("[Clone (invalid)]"), "{text}");
    assert!(dialog.items().iter().all(|item| !matches!(
        item,
        crate::tui::action::FocusItem::Cancel
            | crate::tui::action::FocusItem::Action
    )));
    // Double border uses ╔ on the title row.
    let top = row_text(&buf, modal_rect(Rect::new(0, 0, 80, 24)).y);
    assert!(
        top.contains('╔') || top.contains('═'),
        "double border: {top}"
    );
}

#[test]
fn source_destination_hint_tracks_the_focused_control() {
    use crate::tui::action::FocusItem;
    let mut dialog = clone_dialog(Some("/work"));
    type_text(&mut dialog, "owner/repo");
    for (item, shown, hidden) in [
        (FocusItem::Source, "Ctrl-W Word", "h/l Switch"),
        (FocusItem::ProtocolSsh, "h/l Switch", "Ctrl-W Word"),
        (FocusItem::PresetsToggle, "Space Toggle", "h/l Switch"),
        (FocusItem::AddParent, "Space Toggle", "Ctrl-W Word"),
    ] {
        dialog.focus_item(item);
        let hint = modal_hint(&dialog, 80, 20);
        assert!(hint.contains(shown), "{item:?}: {hint}");
        assert!(!hint.contains(hidden), "{item:?}: {hint}");
        assert!(hint.contains("Enter Clone"), "{item:?}: {hint}");
        assert!(hint.contains("Esc Cancel"), "{item:?}: {hint}");
    }
    dialog.focus_item(FocusItem::PresetsToggle);
    dialog.handle_key(key(KeyCode::Char(' ')));
    dialog.focus_item(FocusItem::SshPrefix);
    let hint = modal_hint(&dialog, 80, 20);
    assert!(hint.contains("Ctrl-W Word"), "{hint}");
    assert!(!hint.contains("Space Toggle"), "{hint}");
}

#[test]
fn source_destination_hint_fits_narrow_modal_without_mid_phrase_truncation() {
    let mut dialog = clone_dialog(Some("/work"));
    type_text(&mut dialog, "owner/repo");
    let hint = modal_hint(&dialog, 60, 20);
    assert!(hint.contains("Ctrl-W Word"), "{hint}");
    assert!(hint.contains("Tab Next"), "{hint}");
    assert!(hint.contains("Enter Clone"), "{hint}");
    assert!(!hint.contains('…'), "{hint}");
}

#[test]
fn clone_validation_error_does_not_override_running_footer() {
    let mut dialog = clone_dialog(Some("/work"));
    dialog.mark_child_started();

    let text = buf_text(&paint(&dialog, 80, 24));
    assert!(text.contains("Ctrl-G requests cancel"), "{text}");
    assert!(
        !text.contains("Ctrl-W Word"),
        "running hint is retained: {text}"
    );
    assert!(!text.contains("destination already exists"), "{text}");
}

#[test]
fn invalid_clone_error_remains_visible_at_short_terminal_height() {
    let mut dialog = clone_dialog(None);
    type_text(&mut dialog, "abc");
    dialog.handle_key(key(KeyCode::Enter));

    let text = buf_text(&paint(&dialog, 80, 14));
    assert!(
        text.contains("repository path must include owner/repo"),
        "{text}"
    );
    assert!(
        !text.contains("Ctrl-W Word"),
        "validation has precedence: {text}"
    );
    assert!(!dialog.git_started());
}

#[test]
fn clone_composer_shows_protocols_without_prefix_or_command_preview() {
    let mut dialog = clone_dialog(Some("/work"));
    type_text(&mut dialog, "acme/repo");
    let text = buf_text(&paint(&dialog, 100, 42));
    assert!(text.contains("[ SSH ] [ HTTPS ]"), "{text}");
    assert!(text.contains("Repository path"), "{text}");
    assert!(text.contains("→ /work/repo"), "{text}");
    assert!(!text.contains("git@github.com:"), "{text}");
    assert!(!text.contains("https://github.com"), "{text}");
    assert!(!text.contains("git clone"), "{text}");
}

#[test]
fn focused_and_selected_clone_controls_use_accent_foreground() {
    use crate::tui::action::FocusItem;
    let mut dialog = clone_dialog(Some("/work"));
    let protocol_cell = |dialog: &ActionDialog, label: &str| {
        let buf = paint(dialog, 100, 42);
        let (row, x) = (0..buf.area.height)
            .find_map(|y| row_text(&buf, y).find(label).map(|x| (y, x as u16)))
            .unwrap();
        assert_eq!(buf[(x, row)].fg, T.accent, "{label}");
    };

    // SSH is selected initially, even while the repository path is focused.
    protocol_cell(&dialog, "[ SSH ]");
    dialog.handle_key(key(KeyCode::BackTab));
    assert_eq!(dialog.item(), Some(FocusItem::ProtocolSsh));
    dialog.handle_key(key(KeyCode::Right));
    assert_eq!(dialog.item(), Some(FocusItem::ProtocolHttps));
    protocol_cell(&dialog, "[ HTTPS ]");
    dialog.handle_key(key(KeyCode::Left));
    assert_eq!(dialog.item(), Some(FocusItem::ProtocolSsh));
    protocol_cell(&dialog, "[ SSH ]");

    dialog.focus_item(FocusItem::PresetsToggle);
    let buf = paint(&dialog, 100, 42);
    let (row, x) = (0..buf.area.height)
        .find_map(|y| {
            row_text(&buf, y)
                .find("Edit prefixes")
                .map(|x| (y, x as u16))
        })
        .unwrap();
    assert_eq!(buf[(x, row)].fg, T.accent, "focused Edit prefixes");
}

#[test]
fn prefixes_toggle_renders_as_an_interactive_button() {
    let dialog = clone_dialog(Some("/work"));
    let text = buf_text(&paint(&dialog, 100, 42));
    assert!(text.contains("[ Edit prefixes: show ]"), "{text}");
}

#[test]
fn prefixes_are_hidden_until_expanded_and_then_editable() {
    use crate::tui::action::FocusItem;
    let mut dialog = clone_dialog(Some("/work"));
    assert!(!dialog.items().contains(&FocusItem::SshPrefix));
    assert!(!dialog.items().contains(&FocusItem::HttpsPrefix));
    let mut text = buf_text(&paint(&dialog, 100, 42));
    assert!(text.contains("[ Edit prefixes: show ]"), "{text}");
    assert!(!text.contains("git@github.com:"), "{text}");

    dialog.focus_item(FocusItem::PresetsToggle);
    dialog.handle_key(key(KeyCode::Char(' ')));
    assert!(dialog.items().contains(&FocusItem::SshPrefix));
    assert!(dialog.items().contains(&FocusItem::HttpsPrefix));
    text = buf_text(&paint(&dialog, 100, 42));
    assert!(text.contains("git@github.com:"), "{text}");
    assert!(text.contains("https://github.com"), "{text}");
}

#[test]
fn tab_keeps_each_clone_field_visible_at_eighty_by_twenty() {
    use crate::tui::action::FocusItem;
    let mut dialog = clone_dialog(Some("/work"));
    type_text(&mut dialog, "acme/repo");
    dialog.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
    assert_eq!(dialog.item(), Some(FocusItem::ProtocolSsh));
    let text = buf_text(&paint(&dialog, 80, 20));
    assert!(
        text.contains("[ HTTPS ]"),
        "focused protocol clipped: {text}"
    );
    dialog.handle_key(key(KeyCode::Tab));
    assert_eq!(dialog.item(), Some(FocusItem::Source));
    for (item, visible) in [
        (FocusItem::Dest, "repo█"),
        (FocusItem::PresetsToggle, "Edit prefixes"),
    ] {
        dialog.handle_key(key(KeyCode::Tab));
        assert_eq!(dialog.item(), Some(item));
        let text = buf_text(&paint(&dialog, 80, 20));
        assert!(text.contains(visible), "focused {item:?} clipped: {text}");
    }
    dialog.handle_key(key(KeyCode::Char(' ')));
    for (item, visible) in [
        (FocusItem::SshPrefix, "git@github.com:█"),
        (FocusItem::HttpsPrefix, "https://github.com█"),
        (FocusItem::AddParent, "Add `/work` to paths"),
    ] {
        dialog.handle_key(key(KeyCode::Tab));
        assert_eq!(dialog.item(), Some(item));
        let text = buf_text(&paint(&dialog, 80, 20));
        assert!(text.contains(visible), "focused {item:?} clipped: {text}");
    }
}

#[test]
fn short_terminal_keeps_focused_destination_visible_with_absolute_preview() {
    use crate::tui::action::FocusItem;
    let mut dialog = clone_dialog(Some("/work"));
    type_text(&mut dialog, "acme/repo");
    dialog.focus_item(FocusItem::Dest);

    let text = buf_text(&paint(&dialog, 80, 14));
    assert!(
        text.contains("repo█"),
        "focused destination clipped: {text}"
    );
}

#[test]
fn short_terminal_can_scroll_to_both_prefix_fields() {
    use crate::tui::action::FocusItem;
    let mut dialog = clone_dialog(Some("/work"));
    let before = buf_text(&paint(&dialog, 80, 24));
    assert!(before.contains("SSH"), "{before}");
    dialog.focus_item(FocusItem::PresetsToggle);
    dialog.handle_key(key(KeyCode::Char(' ')));
    for (item, key_code) in [
        (FocusItem::SshPrefix, KeyCode::Tab),
        (FocusItem::HttpsPrefix, KeyCode::Tab),
    ] {
        dialog.handle_key(key(key_code));
        assert_eq!(dialog.item(), Some(item));
        let after = buf_text(&paint(&dialog, 80, 24));
        assert!(after.contains("Edit prefixes"), "{after}");
    }
    let after = buf_text(&paint(&dialog, 80, 24));
    assert!(after.contains("https://github.com"), "{after}");
    assert!(!after.contains("[Clone (invalid)]"), "{after}");
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
    type_text(&mut dialog, "acme/y.git");
    dialog.handle_key(key(KeyCode::Enter));
    let buf = paint(&dialog, 80, 24);
    let text = buf_text(&buf);
    assert!(text.contains("→ /work/y"), "{text}");
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
    type_text(&mut dialog, "acme/src.git");
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
