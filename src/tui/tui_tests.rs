use super::{RefreshEvent, RefreshKind, Tui};
use crate::config::{Command, Multiplexer, ResolvedConfig, SessionCandidate};
use crate::theme::Theme;
use crate::tui::action::{FakePty, PtyEvent};
use crate::tui::hints::{
    DialogAwaiting, HintSurface, format_hint_rows, shortcut_hints,
};
use crate::tui::sessions_list::VisualTarget;
use crate::utils::test_utils::TempDir;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::Rect,
    style::Modifier,
    widgets::Widget,
};
use std::fs;
use std::path::Path;
use std::process::Command as GitCmd;
use std::time::{Duration, Instant};
use terminal_colorsaurus::ThemeMode;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn two() -> Tui {
    Tui::new(
        &[
            SessionCandidate::new("/g1/a1".into(), "/g1".into()),
            SessionCandidate::new("/g1/a2".into(), "/g1".into()),
        ],
        ThemeMode::Light,
    )
}

fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
    (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

fn buf_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y, buf.area.width).trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn paint(tui: &mut Tui, w: u16, h: u16) -> Buffer {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    Widget::render(tui, area, &mut buf);
    buf
}

/// Joined text of every hint row the renderer reserves at 80x24,
/// using the same packing and small-height budget as `Tui::render`.
fn bar(tui: &mut Tui) -> String {
    let chips = shortcut_hints(tui.hint_surface());
    let max_rows = 24u16.saturating_sub(3).max(1) as usize;
    let rows = format_hint_rows(&chips, 80, max_rows);
    let n = rows.len().max(1) as u16;
    let buf = paint(tui, 80, 24);
    (24 - n..24)
        .map(|y| row_text(&buf, y, 80).trim_end().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn hint_bar_caps_for_picker_content_at_small_height() {
    let mut tui = two();
    let buf = paint(&mut tui, 40, 5);
    let top = row_text(&buf, 0, 40);
    assert!(top.starts_with("Search:"), "{top}");
    // Search plus a reserved two-row list area survive; the
    // focused folded header is the first list row.
    let header = row_text(&buf, 1, 40);
    assert!(header.contains("g1"), "focused header row: {header}");
    let second_list_row = row_text(&buf, 2, 40);
    assert!(
        second_list_row.trim().is_empty(),
        "second list row stays reserved: {second_list_row:?}"
    );
    let hint_row = row_text(&buf, 3, 40);
    assert!(
        hint_row.contains("Ctrl-X"),
        "hints start below the list area: {hint_row:?}"
    );
    // Hints are capped and ellipsize instead of taking further rows.
    let text = buf_text(&buf);
    assert!(!text.contains("Ctrl-W/Alt-BS Word"), "{text}");
    assert!(!text.contains("Space Expand"), "{text}");
    assert!(text.contains('…'), "capped hints ellipsize: {text}");
}

#[test]
fn clone_dialog_keeps_search_and_title_at_small_height() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    let buf = paint(&mut tui, 40, 5);
    let top = row_text(&buf, 0, 40);
    assert!(top.starts_with("Search:"), "{top}");
    let text = buf_text(&buf);
    assert!(text.contains("Clone"), "dialog title: {text}");
    assert!(text.contains("Ctrl-C Quit"), "hint bar still drawn: {text}");
}

#[test]
fn escape_waits_for_pending_refresh_and_keeps_catalog() {
    let d = TempDir::new();
    let group = d.child("group");
    fs::create_dir_all(&group).unwrap();
    let mut tui = tui_for_repo(&group);
    tui.hold_refresh_for_test(true);
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    let fake = FakePty::new();
    tui.set_dialog_transport(fake.clone());
    for c in "acme/repo.git".chars() {
        tui.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(tui.dialog.as_ref().unwrap().dest(), "repo");
    tui.handle_key(key(KeyCode::Enter));
    let generation = tui.dialog.as_ref().unwrap().generation();
    fake.inject(PtyEvent::Exit { code: Some(0) });
    tui.pump(Instant::now());

    // The clone succeeded and its catalog refresh is still pending.
    assert!(tui.dialog_open());
    assert!(tui.refresh_in_flight);
    assert_eq!(tui.toast_message(), None);
    let chips = shortcut_hints(tui.hint_surface());
    assert!(
        !chips.iter().any(|c| c.label.contains("Esc")),
        "no cancel to advertise while pending: {chips:?}"
    );

    tui.handle_key(key(KeyCode::Esc));
    assert!(
        tui.dialog_open(),
        "escape must not drop the pending refresh"
    );
    assert_eq!(tui.toast_message(), None);
    assert!(tui.refresh_in_flight);
    assert!(
        tui.dialog.as_ref().unwrap().hint().contains("refresh"),
        "blocked escape explains why"
    );

    let dest = tui
        .dialog
        .as_ref()
        .unwrap()
        .mutation_path()
        .unwrap()
        .to_string();
    let candidates = vec![SessionCandidate::new(
        dest.clone(),
        group.display().to_string(),
    )];
    tui.refresh_tx
        .send(RefreshEvent {
            generation,
            kind: RefreshKind::Clone {
                dest: dest.clone(),
                config_error: None,
            },
            result: Ok(candidates),
        })
        .unwrap();
    tui.pump(Instant::now());

    assert!(!tui.dialog_open(), "refresh completes the dialog");
    assert!(!tui.refresh_in_flight);
    assert_eq!(
        tui.toast_message(),
        Some(format!("cloned to `{dest}`").as_str())
    );
    assert_eq!(
        tui.picker.focus(),
        Some(VisualTarget::Child(dest)),
        "the picker receives the refreshed catalog"
    );
}

#[test]
fn ack_waits_for_pending_refresh() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    let generation = tui.dialog.as_ref().unwrap().generation();
    tui.dialog.as_mut().unwrap().present_completed(
        "cloned to `/work/repo`",
        Some("config write failed".into()),
        None,
    );
    tui.refresh_in_flight = true;
    tui.handle_key(key(KeyCode::Enter));
    assert!(tui.dialog_open(), "ack must wait for the refresh");
    assert_eq!(tui.dialog.as_ref().unwrap().generation(), generation);
    assert!(
        tui.dialog.as_ref().unwrap().hint().contains("refresh"),
        "blocked ack explains why"
    );

    // The refresh landing clears the wait hint but keeps the sticky
    // completion until it is acknowledged.
    tui.refresh_tx
        .send(RefreshEvent {
            generation,
            kind: RefreshKind::Clone {
                dest: "/work/repo".into(),
                config_error: Some("config write failed".into()),
            },
            result: Ok(vec![]),
        })
        .unwrap();
    tui.pump(Instant::now());
    assert!(tui.dialog_open(), "sticky completion survives the refresh");
    assert!(!tui.refresh_in_flight);
    assert!(
        !tui.dialog.as_ref().unwrap().hint().contains("refresh"),
        "wait hint clears once the refresh lands"
    );

    // With the refresh settled, the same Enter acknowledges.
    tui.handle_key(key(KeyCode::Enter));
    assert!(!tui.dialog_open());
}

#[test]
fn hint_bar_is_last_row_on_idle_picker() {
    let mut tui = two();
    let buf = paint(&mut tui, 80, 24);
    let top = row_text(&buf, 0, 80);
    assert!(top.contains("Search:"), "{top}");
    let chips = shortcut_hints(HintSurface::PickerIdle {
        expand: false,
        collapse: false,
    });
    let text = buf_text(&buf);
    for chip in &chips {
        assert!(text.contains(chip.label), "{chip:?} missing in {text}");
    }
    let last = row_text(&buf, 23, 80);
    assert!(!last.contains("Search:"), "{last}");
    assert!(
        (1..23).any(|y| !row_text(&buf, y, 80).trim().is_empty()),
        "list should remain above the hint bar"
    );
}

#[test]
fn clone_dialog_does_not_cover_hint_bar() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    let buf = paint(&mut tui, 80, 24);
    let last = row_text(&buf, 23, 80);
    assert!(last.contains("Tab"), "{last}");
    assert!(
        !last.contains('╔') && !last.contains('╚') && !last.contains('═'),
        "dialog border must not cover hint bar: {last}"
    );
    let text = buf_text(&buf);
    assert!(text.contains("Search:"), "{text}");
    assert!(text.contains("Clone"), "{text}");
}

#[test]
fn hint_bar_truncates_on_narrow_width() {
    let mut tui = two();
    for w in [10u16, 20] {
        let buf = paint(&mut tui, w, 24);
        let text = buf_text(&buf);
        for y in 0..24 {
            let row = row_text(&buf, y, w);
            assert!(
                row.chars().count() <= w as usize,
                "w={w} y={y} row={row:?}"
            );
        }
        assert!(!text.contains("Delete"), "w={w} {text}");
        let last = row_text(&buf, 23, w).trim_end().to_string();
        assert!(
            last.ends_with('…') || last.chars().count() <= w as usize,
            "w={w} last={last:?}"
        );
    }
}

#[test]
fn hint_bar_skipped_when_height_is_one() {
    let mut tui = two();
    let buf = paint(&mut tui, 80, 1);
    let row = row_text(&buf, 0, 80);
    assert!(row.contains("Search:"), "{row}");
    assert!(
        !row.contains("Ctrl-X") && !row.contains("Ctrl-G"),
        "search wins at height 1: {row}"
    );
}

#[test]
fn hint_bar_fits_height_two_without_disabled_keys() {
    let mut tui = two();
    let buf = paint(&mut tui, 80, 2);
    let top = row_text(&buf, 0, 80);
    let last = row_text(&buf, 1, 80);
    assert!(top.contains("Search:"), "{top}");
    assert!(last.contains("Ctrl-X") && last.contains("Ctrl-G"), "{last}");
    assert!(!last.contains("Delete"), "{last}");
    let t = Theme::LIGHT;
    let mut saw_operator = false;
    let mut saw_comment = false;
    for x in 0..80 {
        let fg = buf[(x, 1)].fg;
        saw_operator |= fg == t.operator;
        saw_comment |= fg == t.comment;
    }
    assert!(
        saw_operator && saw_comment,
        "hint bar uses comment/operator colors"
    );
}

#[test]
fn hint_bar_prefix_esc_or_ctrl_x_restores_idle() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    let prefix = bar(&mut tui);
    assert!(prefix.contains("c Clone"), "{prefix}");
    tui.handle_key(key(KeyCode::Esc));
    let idle = bar(&mut tui);
    assert!(idle.contains("Ctrl-X") && idle.contains("Last"), "{idle}");
    assert!(!idle.contains("c Clone"), "{idle}");

    tui.handle_key(ctrl('x'));
    tui.handle_key(ctrl('x'));
    let again = bar(&mut tui);
    assert!(again.contains("Last"), "{again}");
    assert!(!again.contains("c Clone"), "{again}");
}

#[test]
fn hint_bar_invalid_prefix_key_keeps_chips_and_query() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('z')));
    assert_eq!(tui.query(), "");
    let prefix = bar(&mut tui);
    assert!(prefix.contains("c Clone"), "{prefix}");
    assert!(prefix.contains("Esc/Ctrl-X Cancel"), "{prefix}");
}

#[test]
fn hint_bar_clone_form_shows_tab_and_shift_tab() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    let last = bar(&mut tui);
    assert!(last.contains("Tab Next"), "{last}");
    assert!(last.contains("Shift-Tab Prev"), "{last}");
    assert!(!last.contains("Last"), "{last}");
}

fn hint_x(row: &str, needle: &str) -> u16 {
    row.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {row:?}")) as u16
}

#[test]
fn prefix_paints_c_and_d_on_hint_bar_without_actions_row() {
    let mut tui = two();
    let idle = buf_text(&paint(&mut tui, 80, 24));
    assert!(!idle.contains("Actions:"), "{idle}");

    tui.handle_key(ctrl('x'));
    let buf = paint(&mut tui, 80, 24);
    let text = buf_text(&buf);
    assert!(!text.contains("Actions:"), "{text}");
    let t = Theme::LIGHT;
    let row23 = row_text(&buf, 23, 80);
    assert!(row23.contains("c Clone"), "{row23}");
    assert!(row23.contains("d Delete"), "{row23}");
    let cx = hint_x(&row23, "c Clone");
    assert_eq!(buf[(cx, 23)].fg, t.comment, "{row23}");
    assert!(!buf[(cx, 23)].modifier.contains(Modifier::BOLD), "{row23}");
    let dx = hint_x(&row23, "d Delete");
    assert_eq!(buf[(dx, 23)].fg, t.comment, "{row23}");
    assert_eq!(buf[(dx + 2, 23)].fg, t.comment, "{row23}");
    let row1 = row_text(&buf, 1, 80);
    assert!(!row1.contains("Actions"), "{row1}");
}

#[test]
fn prefix_selected_badge_highlights_key_only() {
    let mut tui = two();
    tui.handle_key(key(KeyCode::Enter));
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('j')));
    tui.handle_key(key(KeyCode::Char('k')));
    let buf = paint(&mut tui, 80, 24);
    let row = row_text(&buf, 23, 80);
    let t = Theme::LIGHT;
    let cx = hint_x(&row, "c Clone");
    // The badge pads both sides of the selected key.
    assert_eq!(buf[(cx - 1, 23)].bg, t.bg_alt, "{row}");
    assert_eq!(buf[(cx + 1, 23)].bg, t.bg_alt, "{row}");
    assert_eq!(buf[(cx, 23)].fg, t.accent, "{row}");
    assert!(buf[(cx, 23)].modifier.contains(Modifier::BOLD), "{row}");
    // The action label stays normal weight on the plain background.
    let lx = hint_x(&row, "Clone");
    assert_eq!(buf[(lx, 23)].fg, t.operator, "{row}");
    assert_eq!(buf[(lx, 23)].bg, t.bg, "{row}");
    assert!(!buf[(lx, 23)].modifier.contains(Modifier::BOLD), "{row}");
}

#[test]
fn prefix_opening_menu_leaves_both_badges_unhighlighted() {
    let mut tui = two();
    tui.handle_key(key(KeyCode::Enter));
    tui.handle_key(ctrl('x'));
    let buf = paint(&mut tui, 80, 24);
    let row = row_text(&buf, 23, 80);
    let t = Theme::LIGHT;
    let cx = hint_x(&row, "c Clone");
    assert_eq!(buf[(cx, 23)].fg, t.comment, "{row}");
    assert!(!buf[(cx, 23)].modifier.contains(Modifier::BOLD), "{row}");
    let dx = hint_x(&row, "d Delete");
    assert_eq!(buf[(dx, 23)].fg, t.comment, "{row}");
    assert!(!buf[(dx, 23)].modifier.contains(Modifier::BOLD), "{row}");
}

#[test]
fn prefix_selected_delete_highlights_only_delete_badge() {
    let mut tui = two();
    tui.handle_key(key(KeyCode::Enter));
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('j')));
    tui.handle_key(key(KeyCode::Char('j')));
    let buf = paint(&mut tui, 80, 24);
    let row = row_text(&buf, 23, 80);
    let t = Theme::LIGHT;
    let cx = hint_x(&row, "c Clone");
    assert_eq!(buf[(cx, 23)].fg, t.comment, "{row}");
    assert!(!buf[(cx, 23)].modifier.contains(Modifier::BOLD), "{row}");
    let dx = hint_x(&row, "d Delete");
    assert_eq!(buf[(dx, 23)].fg, t.accent, "{row}");
    assert!(buf[(dx, 23)].modifier.contains(Modifier::BOLD), "{row}");
    let lx = hint_x(&row, "Delete");
    assert_eq!(buf[(lx, 23)].fg, t.operator, "{row}");
    assert!(!buf[(lx, 23)].modifier.contains(Modifier::BOLD), "{row}");
}

#[test]
fn prefix_disabled_delete_badge_stays_dim() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    let buf = paint(&mut tui, 80, 24);
    let row = row_text(&buf, 23, 80);
    let t = Theme::LIGHT;
    let dx = hint_x(&row, "d Delete");
    assert_eq!(buf[(dx, 23)].fg, t.comment, "{row}");
    assert_eq!(buf[(dx, 23)].bg, t.bg_alt, "{row}");
    assert!(buf[(dx, 23)].modifier.contains(Modifier::DIM), "{row}");
    let lx = hint_x(&row, "Delete");
    assert_eq!(buf[(lx, 23)].fg, t.comment, "{row}");
    assert!(buf[(lx, 23)].modifier.contains(Modifier::DIM), "{row}");
    assert!(!buf[(lx, 23)].modifier.contains(Modifier::BOLD), "{row}");
}

#[test]
fn hint_bar_wraps_on_badge_padding() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    let buf = paint(&mut tui, 17, 24);
    let rows: Vec<String> = (0..24)
        .map(|y| row_text(&buf, y, 17).trim_end().to_string())
        .collect();
    // The plain labels "c Clone d Delete" fit in 17 cells, but the
    // padded key badges need 18, so each action takes its own row.
    let c_row = rows.iter().position(|r| r.contains("c Clone")).unwrap();
    let d_row = rows.iter().position(|r| r.contains("d Delete")).unwrap();
    assert!(c_row < d_row, "{rows:?}");
    assert!(rows.iter().all(|r| r.chars().count() <= 17), "{rows:?}");
}

#[test]
fn prefix_child_delete_chip_is_enabled_not_dimmed() {
    let mut tui = two();
    tui.handle_key(key(KeyCode::Enter));
    tui.handle_key(ctrl('x'));
    let buf = paint(&mut tui, 80, 24);
    let t = Theme::LIGHT;
    let row23 = row_text(&buf, 23, 80);
    let dx = hint_x(&row23, "d Delete");
    assert_eq!(buf[(dx, 23)].fg, t.comment, "{row23}");
    assert_eq!(buf[(dx, 23)].bg, t.bg_alt, "{row23}");
    assert!(!buf[(dx, 23)].modifier.contains(Modifier::DIM), "{row23}");
    let lx = hint_x(&row23, "Delete");
    assert_eq!(buf[(lx, 23)].fg, t.operator, "{row23}");
    assert_eq!(buf[(lx, 23)].bg, t.bg, "{row23}");
    assert!(!buf[(lx, 23)].modifier.contains(Modifier::BOLD), "{row23}");
}

#[test]
fn hint_bar_shows_c_and_d_on_header_child_and_empty() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    let header = bar(&mut tui);
    assert!(header.contains("c Clone"), "{header}");
    assert!(header.contains("d Delete"), "{header}");
    tui.handle_key(key(KeyCode::Esc));
    tui.handle_key(key(KeyCode::Enter));
    tui.handle_key(ctrl('x'));
    let child = bar(&mut tui);
    assert!(child.contains("d Delete"), "{child}");

    let mut empty = Tui::new(&[], ThemeMode::Light);
    empty.handle_key(ctrl('x'));
    let catalog = bar(&mut empty);
    assert!(catalog.contains("c Clone"), "{catalog}");
    assert!(catalog.contains("d Delete"), "{catalog}");
}

#[test]
fn hint_bar_space_expand_only_on_focused_header() {
    let mut tui = two();
    let header = bar(&mut tui);
    assert!(header.contains("Space Expand"), "{header}");
    assert!(!header.contains("Left Collapse"), "{header}");
    tui.handle_key(key(KeyCode::Enter));
    let child = bar(&mut tui);
    assert!(child.contains("Left Collapse"), "{child}");
    assert!(!child.contains("Space Expand"), "{child}");
}

#[test]
fn hint_bar_clone_form_typing_omits_stage_inspect_and_space() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    let last = bar(&mut tui);
    assert!(last.contains("Tab Next"), "{last}");
    assert!(!last.contains("Inspect"), "{last}");
    assert!(!last.contains("[/]"), "{last}");
    assert!(!last.contains("Space Toggle"), "{last}");
}

#[test]
fn hint_bar_inspect_enables_after_completed_stage_selected() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    tui.mark_dialog_child_started_for_test();
    let t = Theme::LIGHT;
    let buf = paint(&mut tui, 80, 24);
    let row = row_text(&buf, 23, 80);
    assert!(row.contains("i Inspect"), "{row}");
    let ix = hint_x(&row, "Inspect");
    assert_eq!(buf[(ix, 23)].fg, t.comment, "{row}");

    tui.handle_key(key(KeyCode::Char('[')));
    let buf = paint(&mut tui, 80, 24);
    let row = row_text(&buf, 23, 80);
    let ix = hint_x(&row, "Inspect");
    assert_eq!(buf[(ix, 23)].fg, t.operator, "{row}");
}

#[test]
fn hint_bar_running_ctrl_g_is_cancel_not_last() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    tui.mark_dialog_child_started_for_test();
    let last = bar(&mut tui);
    assert!(last.contains("Ctrl-G") && last.contains("Cancel"), "{last}");
    assert!(!last.contains("Last"), "{last}");
    assert!(!last.contains("Esc"), "{last}");
}

#[test]
fn hint_bar_closing_dialog_restores_picker_last() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    tui.handle_key(key(KeyCode::Esc));
    let last = bar(&mut tui);
    assert!(last.contains("Ctrl-G") && last.contains("Last"), "{last}");
    assert!(!last.contains("Cancel"), "{last}");
}

#[test]
fn hint_bar_awaiting_config_omits_esc_cancel() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    tui.set_dialog_awaiting_for_test(DialogAwaiting::Config);
    let last = bar(&mut tui);
    assert!(!last.contains("Esc"), "{last}");
    assert!(!last.contains("Cancel"), "{last}");
}

#[test]
fn hint_bar_awaiting_mutate_omits_esc_cancel() {
    let mut tui = two();
    tui.handle_key(key(KeyCode::Enter));
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('d')));
    tui.set_dialog_awaiting_for_test(DialogAwaiting::Mutate);
    let last = bar(&mut tui);
    assert!(!last.contains("Esc"), "{last}");
    assert!(!last.contains("Cancel"), "{last}");
}

#[test]
fn clone_opens_overlay_without_editing_query() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    assert!(tui.dialog_open());
    assert_eq!(tui.query(), "");
    tui.handle_key(key(KeyCode::Char('z')));
    assert_eq!(tui.query(), "", "dialog traps keys");
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    Widget::render(&mut tui, area, &mut buf);
    let text = buf_text(&buf);
    assert!(text.contains("Search:"), "{text}");
    assert!(text.contains("Clone"), "{text}");
}

#[test]
fn escape_closes_dialog_and_shows_cancel_toast() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('c')));
    tui.handle_key(key(KeyCode::Esc));
    assert!(!tui.dialog_open());
    assert_eq!(tui.query(), "");
    assert_eq!(tui.toast_message(), Some("cancelled"));
    let now = Instant::now();
    tui.pump(now + Duration::from_millis(1600));
    assert_eq!(tui.toast_message(), None);
}

#[test]
fn picker_ctrl_g_is_not_dialog_cancel_when_closed() {
    let mut tui = two();
    tui.handle_key(ctrl('g'));
    assert!(!tui.dialog_open());
}

#[test]
fn delete_opens_overlay_over_picker() {
    let mut tui = two();
    tui.handle_key(key(KeyCode::Enter));
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('d')));
    assert!(tui.dialog_open());
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    Widget::render(&mut tui, area, &mut buf);
    let text = buf_text(&buf);
    assert!(text.contains("Delete"), "{text}");
    assert!(text.contains("Search:"), "{text}");
}

#[test]
fn key_release_is_ignored() {
    let mut tui = two();
    let mut release = ctrl('x');
    release.kind = KeyEventKind::Release;
    tui.handle_key(release);
    assert!(!tui.dialog_open());
}

#[test]
fn source_has_no_restore_or_init() {
    let src = include_str!("mod.rs");
    assert!(
        !src.contains("ratatui::restore"),
        "TUI must not restore the terminal"
    );
    assert!(
        !src.contains("ratatui::init"),
        "TUI must not re-init the terminal"
    );
    assert!(!src.contains("fn run_clone"));
    assert!(!src.contains("fn clone_restored"));
    assert!(!src.contains("fn run_delete"));
    assert!(!src.contains("fn delete_restored"));
}

fn git(dir: &Path, args: &[&str]) {
    let out = GitCmd::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git subprocess failed to spawn");
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn init_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "topic"]);
    fs::write(dir.join("file.txt"), "one\n").unwrap();
    git(dir, &["add", "file.txt"]);
    git(
        dir,
        &[
            "-c",
            "user.email=contx@test",
            "-c",
            "user.name=contx",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "first",
        ],
    );
}

fn tui_for_repo(repo: &Path) -> Tui {
    let path = repo.display().to_string();
    let group = repo.parent().unwrap_or(repo).display().to_string();
    let cfg = ResolvedConfig {
        candidates: vec![SessionCandidate::new(path, group)],
        multiplexer: Multiplexer::Auto,
        command: Command::Picker,
        json: false,
        permanent_delete: false,
        clone: crate::config::CloneSettings::default(),
        config_path: "/tmp/contx-test.toml".into(),
        paths: vec![],
        git_from_home: false,
        config_existed: true,
    };
    Tui::from_config(cfg, ThemeMode::Light)
}

fn pump_until(tui: &mut Tui, timeout: Duration, pred: impl Fn(&Tui) -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        tui.pump(Instant::now());
        if pred(tui) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for delete dialog progress");
}

fn fetch_spawned(fake: &FakePty) -> bool {
    fake.spawns().iter().any(|(argv, _)| {
        argv.windows(3).any(|w| w == ["fetch", "--all", "--prune"])
    })
}

#[test]
fn delete_no_remote_standalone_skips_fetch_pty() {
    let d = TempDir::new();
    let repo = d.child("repo");
    init_repo(&repo);
    let path = repo.display().to_string();
    let mut tui = tui_for_repo(&repo);
    let fake = FakePty::new();
    tui.open_delete_for_test(path);
    tui.set_dialog_transport(fake.clone());
    tui.handle_key(key(KeyCode::Enter));
    assert!(!fetch_spawned(&fake), "no-remote must not spawn fetch PTY");
    pump_until(&mut tui, Duration::from_secs(5), |t| {
        !t.delete_findings().is_empty()
    });
    assert!(!fetch_spawned(&fake), "inspect must not spawn fetch PTY");
    let findings = tui.delete_findings().join("\n");
    assert!(
        findings.contains("no remotes") || findings.contains("HARD BLOCKER"),
        "expected preflight findings, got {findings:?}"
    );
}

#[test]
fn delete_standalone_with_remote_spawns_fetch_pty() {
    let d = TempDir::new();
    let repo = d.child("repo");
    init_repo(&repo);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            d.child("origin").to_str().unwrap(),
        ],
    );
    let path = repo.display().to_string();
    let mut tui = tui_for_repo(&repo);
    let fake = FakePty::new();
    tui.open_delete_for_test(path);
    tui.set_dialog_transport(fake.clone());
    tui.handle_key(key(KeyCode::Enter));
    pump_until(&mut tui, Duration::from_secs(5), |_| fetch_spawned(&fake));
    assert!(fetch_spawned(&fake));
    assert!(
        tui.delete_findings().is_empty(),
        "fetch PTY should run before findings"
    );
}

#[test]
fn new_dir_action_opens_destination_only_dialog() {
    let mut tui = two();
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('n')));
    assert!(tui.dialog_open());
    assert!(tui.dialog.as_ref().unwrap().is_new_dir());
    let text = buf_text(&paint(&mut tui, 80, 24));
    assert!(text.contains("New directory"), "{text}");
    assert!(text.contains("Destination"), "{text}");
    assert!(!text.contains("Source"), "{text}");
}

#[test]
fn new_dir_creates_directory_and_reports_created() {
    let d = TempDir::new();
    let group = d.child("group");
    let existing = group.join("existing");
    fs::create_dir_all(&existing).unwrap();
    let mut tui = tui_for_repo(&existing);
    tui.hold_refresh_for_test(true);
    tui.handle_key(ctrl('x'));
    tui.handle_key(key(KeyCode::Char('n')));
    for c in "alpha".chars() {
        tui.handle_key(key(KeyCode::Char(c)));
    }
    tui.handle_key(key(KeyCode::Enter));
    let created = group.join("alpha");
    assert!(created.is_dir(), "directory was not created");
    let generation = tui.dialog.as_ref().unwrap().generation();
    assert!(tui.refresh_in_flight);
    tui.refresh_tx
        .send(RefreshEvent {
            generation,
            kind: RefreshKind::Create {
                dest: created.display().to_string(),
                config_error: None,
            },
            result: Ok(vec![SessionCandidate::new(
                created.display().to_string(),
                group.display().to_string(),
            )]),
        })
        .unwrap();
    tui.pump(Instant::now());
    assert!(!tui.dialog_open(), "refresh completes the dialog");
    assert_eq!(
        tui.toast_message(),
        Some(format!("created `{}`", created.display()).as_str())
    );
}
