use super::{
    DialogAwaiting, DialogHintState, HintChip, HintSurface, format_hint_rows,
    shortcut_hints,
};
use crate::tui::action::CancelState;

fn on(label: &'static str) -> HintChip {
    HintChip {
        label,
        enabled: true,
        selected: false,
    }
}

fn off(label: &'static str) -> HintChip {
    HintChip {
        label,
        enabled: false,
        selected: false,
    }
}

fn idle(expand: bool, collapse: bool) -> Vec<HintChip> {
    shortcut_hints(HintSurface::PickerIdle { expand, collapse })
}

fn prefix(delete_enabled: bool, selected: Option<usize>) -> Vec<HintChip> {
    shortcut_hints(HintSurface::PickerPrefix {
        delete_enabled,
        selected,
    })
}

fn dialog(state: DialogHintState) -> Vec<HintChip> {
    shortcut_hints(HintSurface::Dialog(state))
}

fn form(item_count: usize) -> DialogHintState {
    DialogHintState {
        item_count,
        ..DialogHintState::default()
    }
}

fn running(cancel: CancelState, item_count: usize) -> DialogHintState {
    DialogHintState {
        running: true,
        cancel,
        item_count,
        ..DialogHintState::default()
    }
}

fn labels(chips: &[HintChip]) -> Vec<&'static str> {
    chips.iter().map(|c| c.label).collect()
}

#[test]
fn picker_idle_inventory_omits_clone_delete_and_fold_keys() {
    let chips = idle(false, false);
    assert_eq!(
        chips,
        vec![
            on("Ctrl-X Actions"),
            on("Ctrl-J/K Move"),
            on("Ctrl-T First"),
            on("Ctrl-G/B Last"),
            on("Ctrl-W/Alt-BS Word"),
            on("Ctrl-C Quit"),
        ]
    );
    assert!(
        !labels(&chips)
            .iter()
            .any(|c| c.contains("Clone") || c.contains("Delete")),
        "{chips:?}"
    );
    assert!(
        !labels(&chips)
            .iter()
            .any(|c| c.contains("Space") || c.contains("Left")),
        "{chips:?}"
    );
}

#[test]
fn picker_idle_space_expand_only_when_expand() {
    let chips = idle(true, false);
    assert!(chips.contains(&on("Space Expand")), "{chips:?}");
    assert!(
        !labels(&chips).iter().any(|c| c.contains("Collapse")),
        "{chips:?}"
    );
}

#[test]
fn picker_idle_left_collapse_only_when_collapse() {
    let chips = idle(false, true);
    assert!(chips.contains(&on("Left Collapse")), "{chips:?}");
    assert!(
        !labels(&chips).iter().any(|c| c.contains("Expand")),
        "{chips:?}"
    );
}

#[test]
fn prefix_always_shows_clone_new_dir_delete_cancel_and_quit() {
    let chips = prefix(false, None);
    assert_eq!(
        chips,
        vec![
            HintChip {
                label: "c Clone",
                enabled: true,
                selected: false,
            },
            HintChip {
                label: "n New directory",
                enabled: true,
                selected: false,
            },
            HintChip {
                label: "d Delete",
                enabled: false,
                selected: false,
            },
            on("Esc/Ctrl-X Cancel"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn prefix_without_navigation_highlights_neither_chip() {
    let chips = prefix(true, None);
    assert!(
        chips.iter().all(|c| !c.selected),
        "nothing is selected before an explicit navigation: {chips:?}"
    );
    let disabled = prefix(false, None);
    assert!(disabled.iter().all(|c| !c.selected), "{disabled:?}");
}

#[test]
fn prefix_clone_navigation_highlights_only_clone() {
    let chips = prefix(true, Some(0));
    assert_eq!(
        chips,
        vec![
            HintChip {
                label: "c Clone",
                enabled: true,
                selected: true,
            },
            HintChip {
                label: "n New directory",
                enabled: true,
                selected: false,
            },
            HintChip {
                label: "d Delete",
                enabled: true,
                selected: false,
            },
            on("Esc/Ctrl-X Cancel"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn prefix_new_dir_navigation_highlights_only_new_dir() {
    let chips = prefix(true, Some(1));
    assert_eq!(
        chips,
        vec![
            HintChip {
                label: "c Clone",
                enabled: true,
                selected: false,
            },
            HintChip {
                label: "n New directory",
                enabled: true,
                selected: true,
            },
            HintChip {
                label: "d Delete",
                enabled: true,
                selected: false,
            },
            on("Esc/Ctrl-X Cancel"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn prefix_delete_enabled_can_be_selected() {
    let chips = prefix(true, Some(2));
    assert_eq!(
        chips,
        vec![
            HintChip {
                label: "c Clone",
                enabled: true,
                selected: false,
            },
            HintChip {
                label: "n New directory",
                enabled: true,
                selected: false,
            },
            HintChip {
                label: "d Delete",
                enabled: true,
                selected: true,
            },
            on("Esc/Ctrl-X Cancel"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn prefix_omits_picker_ctrl_g_last_and_word() {
    let chips = prefix(true, Some(0));
    let names = labels(&chips);
    assert!(!names.iter().any(|c| c.contains("Last")), "{chips:?}");
    assert!(!names.iter().any(|c| c.contains("Word")), "{chips:?}");
    assert!(!names.iter().any(|c| c.contains("Ctrl-J")), "{chips:?}");
}

#[test]
fn prefix_does_not_select_disabled_delete() {
    let chips = prefix(false, Some(1));
    let delete = chips.iter().find(|c| c.label == "d Delete").unwrap();
    assert!(!delete.enabled);
    assert!(!delete.selected);
}

#[test]
fn form_with_two_or_more_items_shows_tab_and_shift_tab() {
    let chips = dialog(form(4));
    assert_eq!(
        chips,
        vec![
            on("Tab Next"),
            on("Shift-Tab Prev"),
            on("Esc Cancel"),
            on("[/] Stage"),
            off("i Inspect"),
            on("Ctrl-C Quit"),
        ]
    );
    let one = dialog(form(1));
    assert!(!labels(&one).iter().any(|c| c.contains("Tab")), "{one:?}");
}

#[test]
fn idle_form_esc_cancels_and_ctrl_g_is_not_last_or_cancel() {
    let chips = dialog(form(2));
    assert!(chips.contains(&on("Esc Cancel")), "{chips:?}");
    assert!(
        !labels(&chips).iter().any(|c| c.contains("Ctrl-G")),
        "{chips:?}"
    );
}

#[test]
fn running_idle_shows_ctrl_g_cancel_not_esc_or_last() {
    let chips = dialog(running(CancelState::Idle, 1));
    assert_eq!(
        chips,
        vec![
            on("Ctrl-G Cancel"),
            on("[/] Stage"),
            off("i Inspect"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn running_with_auth_fields_still_shows_tab() {
    let chips = dialog(running(CancelState::Idle, 3));
    assert_eq!(
        chips,
        vec![
            on("Tab Next"),
            on("Shift-Tab Prev"),
            on("Ctrl-G Cancel"),
            on("[/] Stage"),
            off("i Inspect"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn grace_omits_ctrl_g_esc_and_tab() {
    let chips = dialog(running(CancelState::Grace, 0));
    assert_eq!(
        chips,
        vec![on("[/] Stage"), off("i Inspect"), on("Ctrl-C Quit"),]
    );
}

#[test]
fn force_ready_omits_ignored_keys() {
    let chips = dialog(running(CancelState::ForceReady, 1));
    assert_eq!(
        chips,
        vec![on("[/] Stage"), off("i Inspect"), on("Ctrl-C Quit"),]
    );
}

#[test]
fn refresh_pending_omits_esc_cancel() {
    let chips = dialog(DialogHintState {
        item_count: 1,
        refresh_pending: true,
        ..DialogHintState::default()
    });
    assert!(
        !labels(&chips).iter().any(|c| c.contains("Esc")),
        "a pending refresh has nothing to cancel: {chips:?}"
    );
    assert!(chips.contains(&on("Ctrl-C Quit")), "{chips:?}");
}

#[test]
fn sticky_omits_esc_and_tab() {
    let chips = dialog(DialogHintState {
        item_count: 1,
        sticky: true,
        ..DialogHintState::default()
    });
    assert_eq!(
        chips,
        vec![on("[/] Stage"), off("i Inspect"), on("Ctrl-C Quit"),]
    );
}

#[test]
fn awaiting_config_and_mutate_omit_esc_cancel() {
    for awaiting in [DialogAwaiting::Config, DialogAwaiting::Mutate] {
        let chips = dialog(DialogHintState {
            awaiting,
            ..DialogHintState::default()
        });
        assert_eq!(
            chips,
            vec![on("[/] Stage"), off("i Inspect"), on("Ctrl-C Quit"),],
            "{awaiting:?}"
        );
    }
}

#[test]
fn awaiting_inspect_esc_still_cancels() {
    let chips = dialog(DialogHintState {
        awaiting: DialogAwaiting::Inspect,
        ..DialogHintState::default()
    });
    assert_eq!(
        chips,
        vec![
            on("Esc Cancel"),
            on("[/] Stage"),
            off("i Inspect"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn typing_omits_stage_inspect_and_space() {
    let chips = dialog(DialogHintState {
        item_count: 4,
        typing: true,
        inspect_enabled: true,
        add_parent_focused: true,
        ..DialogHintState::default()
    });
    assert_eq!(
        chips,
        vec![
            on("Tab Next"),
            on("Shift-Tab Prev"),
            on("Esc Cancel"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn inspect_enabled_uses_space_alias_unless_add_parent_focused() {
    let open = dialog(DialogHintState {
        item_count: 2,
        inspect_enabled: true,
        ..DialogHintState::default()
    });
    assert!(open.contains(&on("i Inspect")), "{open:?}");
    let parent = dialog(DialogHintState {
        item_count: 2,
        inspect_enabled: true,
        add_parent_focused: true,
        ..DialogHintState::default()
    });
    assert_eq!(
        parent,
        vec![
            on("Tab Next"),
            on("Shift-Tab Prev"),
            on("Esc Cancel"),
            on("[/] Stage"),
            on("i Inspect"),
            on("Space Toggle"),
            on("Ctrl-C Quit"),
        ]
    );
}

#[test]
fn focused_selection_and_toggle_have_accurate_hints() {
    let focused = dialog(DialogHintState {
        inspect_enabled: true,
        selection_focused: true,
        presets_toggle_focused: true,
        ..DialogHintState::default()
    });
    assert!(focused.contains(&on("i Inspect")), "{focused:?}");
    assert!(focused.contains(&on("Space Toggle")), "{focused:?}");
    assert!(
        !labels(&focused)
            .iter()
            .any(|label| label.contains("i/Space"))
    );

    let toggle = dialog(DialogHintState {
        inspect_enabled: true,
        add_parent_focused: true,
        ..DialogHintState::default()
    });
    assert!(toggle.contains(&on("i Inspect")), "{toggle:?}");
    assert!(toggle.contains(&on("Space Toggle")), "{toggle:?}");
}

#[test]
fn every_surface_includes_ctrl_c_quit() {
    for chips in [
        idle(false, false),
        idle(true, true),
        prefix(false, None),
        prefix(true, Some(1)),
        dialog(form(0)),
        dialog(running(CancelState::Grace, 0)),
    ] {
        assert!(chips.contains(&on("Ctrl-C Quit")), "{chips:?}");
    }
}

fn row_blob(
    chips: &[HintChip],
    width: usize,
    max_rows: usize,
) -> (Vec<String>, String) {
    let rows = format_hint_rows(chips, width, max_rows);
    let texts: Vec<String> = rows.iter().map(|r| r.text(width)).collect();
    let blob = texts.join(" ");
    (texts, blob)
}

#[test]
fn format_hint_rows_80_keeps_every_idle_prefix_dialog_chip() {
    for chips in [idle(false, false), prefix(true, Some(0)), dialog(form(4))] {
        let (texts, blob) = row_blob(&chips, 80, 8);
        for chip in &chips {
            assert!(blob.contains(chip.label), "{chip:?} missing in {texts:?}");
        }
        assert!(texts.len() <= 2, "{texts:?}");
        for t in &texts {
            assert!(t.chars().count() <= 80, "{t:?}");
            assert!(!t.contains('…'), "{t:?}");
        }
    }
}

#[test]
fn format_hint_rows_width_10_ellipsizes_overlong_chip() {
    let chips = idle(false, false);
    let rows = format_hint_rows(&chips, 10, 8);
    assert!(!rows.is_empty());
    let first = rows[0].text(10);
    assert!(first.ends_with('…'), "{first:?}");
    assert_eq!(first.chars().count(), 10);
    assert_eq!(rows[0].chips.len(), 1);
}

#[test]
fn format_hint_rows_width_20_keeps_complete_chips() {
    let chips = idle(false, false);
    let rows = format_hint_rows(&chips, 20, 8);
    for row in &rows {
        let t = row.text(20);
        assert!(t.chars().count() <= 20, "{t:?}");
        if row.chips.len() == 1 && row.chips[0].label.chars().count() > 20 {
            assert!(t.ends_with('…'), "{t:?}");
        } else {
            for chip in &row.chips {
                assert!(t.contains(chip.label), "{t:?} {chip:?}");
            }
            assert!(!t.contains('…'), "{t:?}");
        }
    }
}

#[test]
fn format_hint_rows_last_row_ellipsizes_when_capped() {
    let chips = idle(false, false);
    let rows = format_hint_rows(&chips, 80, 1);
    assert_eq!(rows.len(), 1);
    let t = rows[0].text(80);
    assert!(t.ends_with('…'), "{t:?}");
    assert!(t.contains("Ctrl-X Actions"), "{t:?}");
    assert!(!t.contains("Ctrl-C Quit"), "{t:?}");
    assert!(t.chars().count() <= 80, "{t:?}");
}

#[test]
fn hint_chip_width_counts_badge_padding() {
    assert_eq!(on("c Clone").width(), 8);
    assert_eq!(on("Ctrl-W/Alt-BS Word").width(), 19);
}

#[test]
fn format_hint_rows_wraps_on_badge_padding() {
    let chips = prefix(false, None);
    // Plain labels "c Clone d Delete" fit in 17 cells, but the
    // padded key badges need 18, so the pair splits across rows.
    let rows = format_hint_rows(&chips, 17, 8);
    assert_eq!(rows[0].chips, vec![chips[0]]);
    assert_eq!(rows[0].text(17), " c Clone");
    assert_eq!(rows[1].chips, vec![chips[1]]);
    assert!(rows.iter().all(|r| r.text(17).chars().count() <= 17));
}

#[test]
fn format_hint_rows_zero_width_or_rows_is_empty() {
    let chips = idle(false, false);
    assert!(format_hint_rows(&chips, 0, 4).is_empty());
    assert!(format_hint_rows(&chips, 80, 0).is_empty());
}
