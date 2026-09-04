use super::SessionsList;
use crate::{fuzzy, theme::Theme, tui::message::Message};
use ratatui::{
    style::Style,
    text::{Line, Span},
};
use terminal_colorsaurus::ThemeMode;

const THEME_MODE: ThemeMode = ThemeMode::Light;
const HL: Style = Style::new().fg(Theme::LIGHT.accent);

#[test]
fn segments() {
    let idxs: Vec<usize> = vec![0, 1, 3, 5, 6, 7, 9, 11];
    let seg = SessionsList::get_hl_segments(&idxs);
    assert_eq![vec![(0, 1), (3, 3), (5, 7), (9, 9), (11, 11)], seg];

    let idxs: Vec<usize> = vec![2, 3, 4, 5, 6, 7, 9, 10];
    let seg = SessionsList::get_hl_segments(&idxs);
    assert_eq![vec![(2, 7), (9, 10)], seg];

    let idxs: Vec<usize> = vec![3, 6, 9];
    let seg = SessionsList::get_hl_segments(&idxs);
    assert_eq![vec![(3, 3), (6, 6), (9, 9)], seg];
}

#[test]
fn formatting_1() {
    let m = fuzzy::Match {
        entry: String::from("/user/vieites/opt/zerobrew"),
        match_indices: vec![18],
    };
    let sl = SessionsList::default();
    let l = sl.format_line(&m, false, THEME_MODE);
    assert_eq!(
        Line::from(vec![
            Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
            Span::from("z").style(HL),
            Span::from("erobrew").style(super::NORMAL_STYLE),
        ]),
        l
    );

    let m = fuzzy::Match {
        entry: String::from("/user/ze"),
        match_indices: vec![7],
    };
    let l = sl.format_line(&m, false, THEME_MODE);
    assert_eq!(
        Line::from(vec![
            Span::from("/user/z").style(super::NORMAL_STYLE),
            Span::from("e").style(HL),
            Span::default(),
        ]),
        l
    );

    let m = fuzzy::Match {
        entry: String::from("/user/ze"),
        match_indices: vec![0],
    };
    let l = sl.format_line(&m, false, THEME_MODE);
    assert_eq!(
        Line::from(vec![
            Span::default(),
            Span::from("/").style(HL),
            Span::from("user/ze").style(super::NORMAL_STYLE),
        ]),
        l
    );
}

#[test]
fn formatting_2() {
    let m = fuzzy::Match {
        entry: String::from("/user/vieites/opt/zerobrew"),
        match_indices: vec![18, 19],
    };

    let sl = SessionsList::default();
    let l = sl.format_line(&m, false, THEME_MODE);
    assert_eq!(
        Line::from(vec![
            Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
            Span::from("ze").style(HL),
            Span::from("robrew").style(super::NORMAL_STYLE),
        ]),
        l
    );

    let m = fuzzy::Match {
        entry: String::from("/user/vieites/opt/zerobrew"),
        match_indices: vec![18, 24],
    };
    let l = sl.format_line(&m, false, THEME_MODE);
    assert_eq!(
        Line::from(vec![
            Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
            Span::from("z").style(HL),
            Span::from("erobr").style(super::NORMAL_STYLE),
            Span::from("e").style(HL),
            Span::from("w").style(super::NORMAL_STYLE),
        ]),
        l
    );
}

fn list(paths: &[&str]) -> SessionsList {
    let owned: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
    SessionsList::new(&owned)
}

#[test]
fn selection_row_kept_across_reorder() {
    let mut sl = list(&["/x/azb", "/y/ab"]);
    sl.handle_message(Message::NextSession);
    assert_eq!(2, sl.selected_line);

    // "ab" ranks "/y/ab" (span 1) before "/x/azb" (span 2): the
    // numeric row must not follow the entry to its new position.
    sl.handle_message(Message::FilterSessions("ab".to_string()));
    assert_eq!(2, sl.selected_line);
}

#[test]
fn selection_clamps_when_nonzero_results_shrink() {
    let mut sl = list(&["/x/za", "/y/za", "/z/zb"]);
    sl.handle_message(Message::LastSession);
    assert_eq!(3, sl.selected_line);

    sl.handle_message(Message::FilterSessions("zb".to_string()));
    assert_eq!(1, sl.selected_line);
}

#[test]
fn selection_row_kept_at_zero_results() {
    let mut sl = list(&["/x/za", "/y/za"]);
    sl.handle_message(Message::LastSession);
    assert_eq!(2, sl.selected_line);

    sl.handle_message(Message::FilterSessions("qq".to_string()));
    assert_eq!(2, sl.selected_line);
}

#[test]
fn select_at_zero_results_is_noop() {
    let mut sl = list(&["/x/za", "/y/za"]);
    sl.handle_message(Message::LastSession);
    sl.handle_message(Message::FilterSessions("qq".to_string()));

    let res = sl.handle_message(Message::SelectSession);
    assert!(res.is_none());
    assert_eq!(2, sl.selected_line);
}

#[test]
fn selection_row_kept_or_clamped_when_results_return() {
    let mut sl = list(&["/x/za", "/y/zb"]);
    sl.handle_message(Message::LastSession);
    assert_eq!(2, sl.selected_line);

    // Zero results keep the stored row.
    sl.handle_message(Message::FilterSessions("qq".to_string()));
    assert_eq!(2, sl.selected_line);

    // Two results again: row 2 is still valid and kept.
    sl.handle_message(Message::FilterSessions("z".to_string()));
    assert_eq!(2, sl.selected_line);

    // One result: the stored row clamps down to it.
    sl.handle_message(Message::FilterSessions("zb".to_string()));
    assert_eq!(1, sl.selected_line);
}
