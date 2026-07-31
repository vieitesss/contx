use super::SessionsList;
use crate::{fuzzy, theme::Theme};
use ratatui::{
    style::Style,
    text::{Span, Line},
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
