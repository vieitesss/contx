use super::{Intent, Selection};
use crate::fuzzy;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn alt(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::ALT)
}

fn selection(candidates: &[&str]) -> Selection {
    Selection::new(
        &candidates.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
}

fn entries(sel: &Selection) -> Vec<&str> {
    sel.matches().iter().map(|m| m.entry.as_str()).collect()
}

fn type_text(sel: &mut Selection, text: &str) {
    for c in text.chars() {
        assert_eq!(sel.handle_key(key(KeyCode::Char(c))), Intent::None);
    }
}

#[test]
fn typing_appends_and_refreshes_ranked_matches() {
    let mut sel = selection(&["/x/azb", "/y/ab"]);

    type_text(&mut sel, "ab");

    assert_eq!(sel.query(), "ab");
    assert_eq!(entries(&sel), vec!["/y/ab", "/x/azb"]);
    // Index stays on slot 1 as ranks change.
    assert_eq!(sel.selected_line(), 1);
}

#[test]
fn backspace_removes_one_character() {
    let mut sel = selection(&["/x/za", "/y/za"]);
    type_text(&mut sel, "ab");

    assert_eq!(sel.handle_key(key(KeyCode::Backspace)), Intent::None);

    assert_eq!(sel.query(), "a");
}

#[test]
fn word_deletion_removes_the_preceding_word() {
    let mut sel = selection(&["/x/za", "/y/za"]);
    type_text(&mut sel, "foo bar");

    assert_eq!(sel.handle_key(alt(KeyCode::Backspace)), Intent::None);
    assert_eq!(sel.query(), "foo ");

    let mut sel = selection(&["/x/za", "/y/za"]);
    type_text(&mut sel, "foo bar");

    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('w'))), Intent::None);
    assert_eq!(sel.query(), "foo ");

    let mut sel = selection(&["/x/za", "/y/za"]);
    type_text(&mut sel, "foo");

    assert_eq!(sel.handle_key(alt(KeyCode::Backspace)), Intent::None);
    assert_eq!(sel.query(), "");
}

#[test]
fn ctrl_c_exits_without_activating() {
    let mut sel = selection(&["/x/azb", "/y/ab"]);
    type_text(&mut sel, "ab");

    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('c'))), Intent::Exit);

    assert_eq!(sel.query(), "ab");
    assert_eq!(sel.selected_line(), 1);
    assert_eq!(entries(&sel), vec!["/y/ab", "/x/azb"]);
}

#[test]
fn navigation_moves_and_stops_at_limits() {
    let mut sel = selection(&["/a", "/b", "/c"]);

    assert_eq!(sel.handle_key(key(KeyCode::Down)), Intent::None);
    assert_eq!(sel.selected_line(), 2);
    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('j'))), Intent::None);
    assert_eq!(sel.selected_line(), 3);
    assert_eq!(sel.handle_key(key(KeyCode::Down)), Intent::None);
    assert_eq!(sel.selected_line(), 3);
    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('j'))), Intent::None);
    assert_eq!(sel.selected_line(), 3);

    assert_eq!(sel.handle_key(key(KeyCode::Up)), Intent::None);
    assert_eq!(sel.selected_line(), 2);
    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('k'))), Intent::None);
    assert_eq!(sel.selected_line(), 1);
    assert_eq!(sel.handle_key(key(KeyCode::Up)), Intent::None);
    assert_eq!(sel.selected_line(), 1);
    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('k'))), Intent::None);
    assert_eq!(sel.selected_line(), 1);

    assert_eq!(sel.query(), "");
}

#[test]
fn home_and_end_jump_to_first_and_last_results() {
    let mut sel = selection(&["/a", "/b", "/c"]);
    sel.handle_key(key(KeyCode::Down));
    assert_eq!(sel.selected_line(), 2);

    assert_eq!(sel.handle_key(key(KeyCode::Home)), Intent::None);
    assert_eq!(sel.selected_line(), 1);
    assert_eq!(sel.handle_key(key(KeyCode::End)), Intent::None);
    assert_eq!(sel.selected_line(), 3);
    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('t'))), Intent::None);
    assert_eq!(sel.selected_line(), 1);
    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('g'))), Intent::None);
    assert_eq!(sel.selected_line(), 3);
    assert_eq!(sel.handle_key(ctrl(KeyCode::Char('b'))), Intent::None);
    assert_eq!(sel.selected_line(), 3);
}

#[test]
fn enter_selects_the_selected_candidate_without_touching_state() {
    let mut sel = selection(&["/x/azb", "/y/ab"]);
    type_text(&mut sel, "ab");
    sel.handle_key(key(KeyCode::Down));
    assert_eq!(sel.selected_line(), 2);

    let intent = sel.handle_key(key(KeyCode::Enter));

    assert_eq!(intent, Intent::Activate("/x/azb".to_string()));
    assert_eq!(sel.query(), "ab");
    assert_eq!(sel.selected_line(), 2);
    assert_eq!(entries(&sel), vec!["/y/ab", "/x/azb"]);
}

#[test]
fn enter_on_empty_results_is_a_noop() {
    let mut sel = selection(&["/x/za", "/y/za"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 2);
    type_text(&mut sel, "qq");
    assert!(sel.matches().is_empty());

    assert_eq!(sel.handle_key(key(KeyCode::Enter)), Intent::None);

    assert_eq!(sel.selected_line(), 2);
    assert_eq!(sel.query(), "qq");
}

#[test]
fn irrelevant_keys_and_chords_leave_everything_unchanged() {
    let mut sel = selection(&["/x/za", "/y/za"]);
    type_text(&mut sel, "a");
    let query = sel.query().to_string();
    let rows: Vec<String> =
        entries(&sel).iter().map(|s| s.to_string()).collect();
    let row = sel.selected_line();

    let released = KeyEvent {
        kind: KeyEventKind::Release,
        ..key(KeyCode::Char('a'))
    };
    for k in [
        alt(KeyCode::Char('x')),
        ctrl(KeyCode::Char('e')),
        ctrl(KeyCode::Char('a')),
        ctrl(KeyCode::Backspace),
        ctrl(KeyCode::Enter),
        ctrl(KeyCode::Down),
        key(KeyCode::Esc),
        key(KeyCode::Tab),
        key(KeyCode::F(5)),
        released,
    ] {
        assert_eq!(sel.handle_key(k), Intent::None);
    }

    assert_eq!(sel.query(), query);
    assert_eq!(
        entries(&sel),
        rows.iter().map(|s| s.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(sel.selected_line(), row);
}

#[test]
fn index_stays_on_slot_across_reorder() {
    let mut sel = selection(&["/x/azb", "/y/ab"]);
    type_text(&mut sel, "ab");
    assert_eq!(entries(&sel), vec!["/y/ab", "/x/azb"]);
    assert_eq!(sel.selected_line(), 1);
    assert_eq!(
        sel.handle_key(key(KeyCode::Enter)),
        Intent::Activate("/y/ab".to_string())
    );

    let mut sel = selection(&["/x/azb", "/y/ab"]);
    sel.handle_key(key(KeyCode::Down));
    assert_eq!(sel.selected_line(), 2);
    type_text(&mut sel, "ab");
    assert_eq!(entries(&sel), vec!["/y/ab", "/x/azb"]);
    assert_eq!(sel.selected_line(), 2);
    assert_eq!(
        sel.handle_key(key(KeyCode::Enter)),
        Intent::Activate("/x/azb".to_string())
    );
}

#[test]
fn selected_row_clamps_when_nonempty_results_shrink() {
    let mut sel = selection(&["/x/za", "/y/za", "/z/zb"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 3);

    type_text(&mut sel, "zb");
    assert_eq!(sel.selected_line(), 1);
}

#[test]
fn selected_row_is_stored_while_results_are_empty() {
    let mut sel = selection(&["/x/za", "/y/za"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 2);

    type_text(&mut sel, "qq");
    assert_eq!(sel.selected_line(), 2);
}

#[test]
fn selected_row_returns_clamped_or_unchanged_when_matches_return() {
    let mut sel = selection(&["/x/za", "/y/zb"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 2);

    // Zero results keep the stored row.
    type_text(&mut sel, "qq");
    assert_eq!(sel.selected_line(), 2);

    // Clearing the query brings two results back: row 2 is kept.
    sel.handle_key(key(KeyCode::Backspace));
    sel.handle_key(key(KeyCode::Backspace));
    assert_eq!(sel.query(), "");
    assert_eq!(sel.selected_line(), 2);

    // One result: the stored row clamps down to it.
    type_text(&mut sel, "zb");
    assert_eq!(sel.selected_line(), 1);
}

#[test]
fn index_stays_when_selected_path_is_gone() {
    let mut sel = selection(&["/a/foo", "/b/bar", "/c/baz"]);
    assert_eq!(sel.selected_line(), 1);

    type_text(&mut sel, "b");
    assert_eq!(entries(&sel), vec!["/b/bar", "/c/baz"]);
    assert_eq!(sel.selected_line(), 1);
}

#[test]
fn ranked_results_keep_fuzzy_order() {
    let candidates = ["/x/azb", "/y/ab"];
    let mut sel = selection(&candidates);
    type_text(&mut sel, "ab");

    let owned: Vec<String> = candidates.iter().map(|s| s.to_string()).collect();
    let expected: Vec<String> = fuzzy::search(&owned, "ab")
        .iter()
        .map(|m| m.entry.clone())
        .collect();
    let actual: Vec<String> =
        sel.matches().iter().map(|m| m.entry.clone()).collect();

    assert_eq!(actual, expected);
}

#[test]
fn select_entry_selects_the_match_carrying_it() {
    let mut sel = selection(&["/a", "/b", "/c"]);
    assert_eq!(sel.selected_line(), 1);

    sel.select_entry("/c");
    assert_eq!(sel.selected_line(), 3);
    sel.select_entry("/a");
    assert_eq!(sel.selected_line(), 1);
    // The query is untouched: this is selection only.
    assert_eq!(sel.query(), "");
}

#[test]
fn select_entry_leaves_selection_on_unknown_entries() {
    let mut sel = selection(&["/a", "/b"]);
    sel.select_entry("/b");
    assert_eq!(sel.selected_line(), 2);

    // Unknown entries and empty matches never move it, like a
    // move past either end.
    sel.select_entry("/missing");
    assert_eq!(sel.selected_line(), 2);

    let mut sel = selection(&[]);
    sel.select_entry("/a");
    assert_eq!(sel.selected_line(), 1);
}

#[test]
fn horizontal_arrows_are_noops_in_a_single_column() {
    // Production list renders one full-width column, so Left/Right
    // stay no-ops while Up/Down move linearly.
    let mut sel = selection(&["/a", "/b", "/c"]);
    assert_eq!(sel.selected_line(), 1);

    assert_eq!(sel.handle_key(key(KeyCode::Right)), Intent::None);
    assert_eq!(sel.selected_line(), 1);
    assert_eq!(sel.handle_key(key(KeyCode::Left)), Intent::None);
    assert_eq!(sel.selected_line(), 1);

    assert_eq!(sel.handle_key(key(KeyCode::Down)), Intent::None);
    assert_eq!(sel.selected_line(), 2);
    assert_eq!(sel.handle_key(key(KeyCode::Right)), Intent::None);
    assert_eq!(sel.selected_line(), 2);
    assert_eq!(sel.handle_key(key(KeyCode::Left)), Intent::None);
    assert_eq!(sel.selected_line(), 2);
}

#[test]
fn replace_candidates_keeps_query_and_refilters() {
    let mut sel = selection(&["/g1/a1", "/g1/a2", "/g2/b1"]);
    type_text(&mut sel, "a1");
    assert_eq!(entries(&sel), vec!["/g1/a1"]);
    sel.replace_candidates(&["/g1/a1".into(), "/g2/b1".into()]);
    assert_eq!(sel.query(), "a1");
    assert_eq!(entries(&sel), vec!["/g1/a1"]);
}
