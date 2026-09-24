use super::{
    GIT_INDENT, ITEM_ROWS, PATH_INDENT, SessionsListState, VisualMotion,
    VisualTarget, apply_query_folds, apply_visual_motion, collapse_group,
    expand_group, git_spans, visual_entries,
};
use crate::{
    theme::Theme,
    tui::{
        git::{
            CandidateState, Head, PullRequest, PullRequestState, Upstream,
            WorkState,
        },
        selection::{Intent, Selection},
    },
};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    style::{Color, Modifier},
    widgets::{StatefulWidget, Widget},
};
use terminal_colorsaurus::ThemeMode;

fn selection(paths: &[&str]) -> Selection {
    Selection::new(&paths.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

fn render_list(
    sel: &Selection,
    state: &mut SessionsListState,
    w: u16,
    h: u16,
) -> Buffer {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    sel.render(area, &mut buf, state);
    buf
}

fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
    (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn list_layout_constants_single_column_two_rows() {
    // Production list: two rows per item, path indent 2 with no
    // selection arrow, git line subordinate by two more cells.
    assert_eq!(ITEM_ROWS, 2);
    assert_eq!(PATH_INDENT, 2);
    assert_eq!(GIT_INDENT, 4);

    let sel = selection(&["/alpha", "/beta"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    let _ = render_list(&sel, &mut state, 64, 4);
}

#[test]
fn selected_item_uses_full_width_tint_without_borders() {
    let sel = selection(&["/alpha", "/beta"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    // Two items, two rows each.
    let buf = render_list(&sel, &mut state, 40, 4);
    let bg_alt = Color::Rgb(0xEA, 0xE7, 0xE1);
    let bg = Color::Rgb(0xF5, 0xF5, 0xF5);
    let fg = Color::Rgb(0x28, 0x28, 0x28);
    // Selected first item: full-width tint on both rows, indented
    // path, no selection arrow.
    assert_eq!(buf[(0, 0)].bg, bg_alt);
    assert_eq!(buf[(39, 0)].bg, bg_alt);
    assert_eq!(buf[(39, 1)].bg, bg_alt);
    assert_eq!(buf[(0, 0)].symbol(), " ");
    assert_eq!(buf[(1, 0)].symbol(), " ");
    assert_eq!(buf[(1, 1)].fg, fg);
    // Unselected second item keeps the base background.
    assert_eq!(buf[(0, 2)].bg, bg);
    assert_eq!(buf[(39, 3)].bg, bg);
    // No item frames anywhere in the list area. The tree spine
    // (`├`/`└`/`│`) is allowed in indent columns once grouping
    // lands; frames (`┌┐┘─` and heavy corners) never are.
    for y in 0..4 {
        let row = row_text(&buf, y, 40);
        assert!(
            !row.chars().any(|c| matches!(
                c,
                '┌' | '┐'
                    | '┘'
                    | '─'
                    | '┏'
                    | '┓'
                    | '┗'
                    | '┛'
                    | '━'
                    | '┃'
            )),
            "no frames: {row}"
        );
    }
}

#[test]
fn git_spans_nonrepo_measurable_and_weight() {
    let theme = Theme::LIGHT;
    let nonrepo = CandidateState {
        root: None,
        linked: false,
        primary: None,
        state: WorkState::Clean,
        head: Head::Absent,
        upstream: Upstream::Absent,
        pull_request: None,
        pull_request_checked: false,
    };
    assert!(git_spans(Some(&nonrepo), 20, theme).is_empty());

    let meas = CandidateState {
        root: Some("/r".to_string()),
        linked: false,
        primary: None,
        state: WorkState::Measurable {
            added: 3,
            deleted: 4,
        },
        head: Head::Absent,
        upstream: Upstream::Absent,
        pull_request: None,
        pull_request_checked: false,
    };
    let spans = git_spans(Some(&meas), 20, theme);
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains('+') && text.contains('−'));
    assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    for span in spans.iter().skip(1) {
        assert!(
            !span.style.add_modifier.contains(Modifier::BOLD),
            "counts stay normal weight: {}",
            span.content
        );
    }

    let glyph_only = git_spans(Some(&meas), 2, theme);
    let glyph: String = glyph_only.iter().map(|s| s.content.as_ref()).collect();
    assert!(!glyph.contains('+') && !glyph.contains('−'));
    assert!(glyph_only[0].style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn tiny_areas_do_not_panic() {
    let sel = selection(&["/a", "/b", "/c"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 10));
    for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (10, 2), (2, 10), (5, 5)] {
        sel.render(Rect::new(0, 0, w, h), &mut buf, &mut state);
    }
}

#[test]
fn scroll_moves_in_item_row_steps() {
    let mut sel = selection(&["/a", "/b", "/c", "/d", "/e"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 5);
    let mut state = SessionsListState::new(ThemeMode::Light);
    // Two item-rows visible (height 4, two rows per item).
    let buf = render_list(&sel, &mut state, 40, 4);
    assert_eq!(state.scroll, 3);
    assert_eq!(ITEM_ROWS, 2);
    let top = row_text(&buf, 0, 40);
    let next = row_text(&buf, 2, 40);
    assert!(top.contains("/d"), "scrolled to /d: {top}");
    assert!(next.contains("/e"), "next item-row is /e: {next}");
    assert!(!top.contains("/a"));
}

#[test]
fn selected_item_above_viewport_scrolls_back_into_view() {
    let mut sel = selection(&["/a", "/b", "/c", "/d", "/e"]);
    // Same persistent state across renders, like the TUI event loop.
    let mut state = SessionsListState::new(ThemeMode::Light);
    // Scroll to the bottom first so the first item leaves the viewport.
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 5);
    let buf = render_list(&sel, &mut state, 40, 4);
    let top = row_text(&buf, 0, 40);
    assert!(top.contains("/d"), "scrolled to /d: {top}");
    // Jump home: the selected item above the viewport must come
    // back into view, proven by visible path text only.
    sel.handle_key(key(KeyCode::Home));
    assert_eq!(sel.selected_line(), 1);
    let buf = render_list(&sel, &mut state, 40, 4);
    let top = row_text(&buf, 0, 40);
    let next = row_text(&buf, 2, 40);
    assert!(top.contains("/a"), "scrolled back to /a: {top}");
    assert!(next.contains("/b"), "next item-row is /b: {next}");
}

#[test]
fn filter_label_is_visible_on_the_top_row() {
    let mut tui = crate::tui::Tui::new(&[], ThemeMode::Light);
    let area = Rect::new(0, 0, 40, 5);
    let mut buf = Buffer::empty(area);
    Widget::render(&mut tui, area, &mut buf);
    let top = row_text(&buf, 0, 40);
    assert!(
        top.starts_with("Search: "),
        "top row is search chrome: {top}"
    );
    assert!(top.contains("█"), "cursor is visible: {top}");
    assert!(!top.contains("filter:"), "old prompt is gone: {top}");
}

#[test]
fn rendered_path_hits_paint_yellow_in_query_order() {
    let mut sel = selection(&["/alpha", "/beta"]);
    for c in "alp".chars() {
        sel.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(sel.matches().len(), 1);
    let mut state = SessionsListState::new(ThemeMode::Light);
    let buf = render_list(&sel, &mut state, 40, 2);
    let yellow = Theme::LIGHT.accent;
    let fg = Color::Rgb(0x28, 0x28, 0x28);
    // Yellow cells on the path row spell the query; neighbours stay plain.
    let hit_text: String = (0..40)
        .filter(|&x| buf[(x, 0)].fg == yellow)
        .map(|x| buf[(x, 0)].symbol().to_string())
        .collect();
    assert_eq!(hit_text, "alp");
    for x in 0..40 {
        if buf[(x, 0)].fg != yellow {
            assert_eq!(buf[(x, 0)].fg, fg, "plain cell at x={x}");
        }
    }
}

#[test]
fn empty_catalog_states_no_candidates() {
    let sel = selection(&[]);
    assert!(!sel.has_candidates());
    let mut state = SessionsListState::new(ThemeMode::Light);
    let buf = render_list(&sel, &mut state, 40, 4);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("no session candidates"), "empty copy: {row}");
    assert!(!row.contains("no matches"), "not a filter miss: {row}");
    let comment = Color::Rgb(0x45, 0x55, 0x4D);
    assert_eq!(buf[(4, 0)].fg, comment);
}

#[test]
fn filter_miss_states_no_matches_with_query() {
    let mut sel = selection(&["/alpha", "/beta"]);
    assert!(sel.has_candidates());
    sel.handle_key(key(KeyCode::Char('q')));
    sel.handle_key(key(KeyCode::Char('q')));
    assert!(sel.matches().is_empty());
    let mut state = SessionsListState::new(ThemeMode::Light);
    let buf = render_list(&sel, &mut state, 40, 4);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("no matches for \"qq\""), "miss copy: {row}");
    let comment = Color::Rgb(0x45, 0x55, 0x4D);
    assert_eq!(buf[(4, 0)].fg, comment);
}

#[test]
fn empty_tui_keeps_search_chrome_with_cursor() {
    let mut tui = crate::tui::Tui::new(&[], ThemeMode::Light);
    let area = Rect::new(0, 0, 40, 5);
    let mut buf = Buffer::empty(area);
    Widget::render(&mut tui, area, &mut buf);
    let top = row_text(&buf, 0, 40);
    assert!(top.starts_with("Search: "), "search chrome: {top}");
    assert!(top.contains("█"), "cursor is visible: {top}");
    let list_row = row_text(&buf, 1, 40);
    assert!(
        list_row.contains("no session candidates"),
        "empty copy in list area: {list_row}"
    );
}

#[test]
fn git_line_renders_measurable_pair_on_line_two() {
    let sel = selection(&["/alpha"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.git.apply(vec![(
        "/alpha".to_string(),
        CandidateState {
            root: Some("/alpha".to_string()),
            linked: false,
            primary: None,
            state: WorkState::Measurable {
                added: 3,
                deleted: 12,
            },
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )]);
    let buf = render_list(&sel, &mut state, 40, 2);
    let path_row = row_text(&buf, 0, 40);
    let git_row = row_text(&buf, 1, 40);
    assert!(path_row.contains("/alpha"), "path on line 1: {path_row}");
    assert!(git_row.starts_with("    "), "git indent: {git_row}");
    assert!(git_row.contains("+3 −12"), "paired counts: {git_row}");
    // Counts keep their own colors on the tinted row.
    let green = Color::Rgb(0x82, 0xA7, 0x62);
    let red = Color::Rgb(0x98, 0x22, 0x2A);
    let plus = (0..40).find(|&x| buf[(x, 1)].symbol() == "+").unwrap();
    let minus = (0..40).find(|&x| buf[(x, 1)].symbol() == "−").unwrap();
    assert_eq!(buf[(plus, 1)].fg, green);
    assert_eq!(buf[(minus, 1)].fg, red);
}

#[test]
fn git_line_states_loading_failed_marker_nonrepo() {
    let sel = selection(&["/a", "/b", "/c", "/d"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    // "/a" stays unknown: still resolving.
    state.git.apply(vec![
        (
            "/b".to_string(),
            CandidateState {
                root: None,
                linked: false,
                primary: None,
                state: WorkState::Failed,
                head: Head::Absent,
                upstream: Upstream::Absent,
                pull_request: None,
                pull_request_checked: false,
            },
        ),
        (
            "/c".to_string(),
            CandidateState {
                root: Some("/c".to_string()),
                linked: false,
                primary: None,
                state: WorkState::Marker,
                head: Head::Absent,
                upstream: Upstream::Absent,
                pull_request: None,
                pull_request_checked: false,
            },
        ),
        (
            "/d".to_string(),
            CandidateState {
                root: None,
                linked: false,
                primary: None,
                state: WorkState::Clean,
                head: Head::Absent,
                upstream: Upstream::Absent,
                pull_request: None,
                pull_request_checked: false,
            },
        ),
    ]);
    let buf = render_list(&sel, &mut state, 40, 8);
    let loading = row_text(&buf, 1, 40);
    let failed = row_text(&buf, 3, 40);
    let marker = row_text(&buf, 5, 40);
    let nonrepo = row_text(&buf, 7, 40);
    assert!(loading.contains("…"), "loading ellipsis: {loading}");
    assert!(failed.contains(""), "failure glyph: {failed}");
    assert!(!failed.contains('+') && !failed.contains('−'));
    assert!(marker.contains("•"), "dirty marker: {marker}");
    assert!(nonrepo.trim().is_empty(), "nonrepo blank: {nonrepo}");
}

#[test]
fn narrow_width_keeps_basename_and_drops_counts_together() {
    let sel = selection(&["/p/very-long-parent/ab"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.git.apply(vec![(
        "/p/very-long-parent/ab".to_string(),
        CandidateState {
            root: Some("/p/very-long-parent/ab".to_string()),
            linked: false,
            primary: None,
            state: WorkState::Measurable {
                added: 12_345_678,
                deleted: 9,
            },
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )]);
    // Wide: full path with the paired counts.
    let buf = render_list(&sel, &mut state, 40, 2);
    let git_row = row_text(&buf, 1, 40);
    assert!(git_row.contains('+') && git_row.contains('−'));
    // Narrow: parents shorten before the basename, and the counts
    // drop as a pair (glyph only, never plus-only).
    let buf = render_list(&sel, &mut state, 10, 2);
    let path_row = row_text(&buf, 0, 10);
    let git_row = row_text(&buf, 1, 10);
    assert_eq!(path_row, "  /p/ve…ab");
    assert!(!git_row.contains('+') && !git_row.contains('−'));
}

fn upstream_state(state: WorkState, upstream: Upstream) -> CandidateState {
    CandidateState {
        root: Some("/r".to_string()),
        linked: false,
        primary: None,
        state,
        head: Head::Absent,
        upstream,
        pull_request: None,
        pull_request_checked: false,
    }
}

fn spans_text(state: &CandidateState, w: usize) -> String {
    git_spans(Some(state), w, Theme::LIGHT)
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn named_state(
    state: WorkState,
    name: &str,
    upstream: Upstream,
) -> CandidateState {
    CandidateState {
        root: Some("/r".to_string()),
        linked: false,
        primary: None,
        state,
        head: Head::Named(name.to_string()),
        upstream,
        pull_request: None,
        pull_request_checked: false,
    }
}

#[test]
fn git_spans_shows_associated_pr_number_and_state() {
    let mut state = named_state(WorkState::Clean, "topic", Upstream::Absent);
    state.pull_request = Some(PullRequest {
        number: 123,
        state: PullRequestState::Open,
    });

    assert!(spans_text(&state, 40).contains("#123 OPEN"));
}

#[test]
fn git_spans_omits_pr_when_no_pr_or_insufficient_width() {
    let state = named_state(WorkState::Clean, "topic", Upstream::Absent);
    assert!(!spans_text(&state, 40).contains("#"));

    let mut state = state;
    state.pull_request = Some(PullRequest {
        number: 123,
        state: PullRequestState::Closed,
    });
    assert!(!spans_text(&state, 8).contains("#123"));
}

#[test]
fn git_spans_shows_branch_after_icon_before_dirty() {
    // Clean + branch: icon, name, nothing else.
    let clean = named_state(WorkState::Clean, "topic", Upstream::Absent);
    let spans = git_spans(Some(&clean), 20, Theme::LIGHT);
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "\u{ec6f} topic");
    // Pair order: icon, name, local counts, then arrows.
    let diverged = named_state(
        WorkState::Measurable {
            added: 3,
            deleted: 12,
        },
        "topic",
        Upstream::Counts {
            ahead: 2,
            behind: 1,
        },
    );
    assert_eq!(spans_text(&diverged, 30), "\u{ec6f} topic +3 −12 ↓1 ↑2");
    // Marker order: icon, name, marker, then arrows.
    let marked = named_state(
        WorkState::Marker,
        "topic",
        Upstream::Counts {
            ahead: 0,
            behind: 3,
        },
    );
    assert_eq!(spans_text(&marked, 20), "\u{ec6f} topic • ↓3");
    // The name paints `git_icon`, never bold like the icon.
    let name = spans.iter().find(|s| s.content == "topic").expect("name");
    assert_eq!(name.style.fg, Some(Theme::LIGHT.git_icon));
    assert!(
        !name.style.add_modifier.contains(Modifier::BOLD),
        "name stays normal weight",
    );
}

#[test]
fn git_spans_detached_shows_short_sha() {
    let detached = CandidateState {
        root: Some("/r".to_string()),
        linked: false,
        primary: None,
        state: WorkState::Clean,
        head: Head::Detached {
            short: "a1b2c3d".to_string(),
        },
        upstream: Upstream::Absent,
        pull_request: None,
        pull_request_checked: false,
    };
    let text = spans_text(&detached, 20);
    assert_eq!(text, "\u{ec6f} a1b2c3d");
    assert!(!text.contains("HEAD") && !text.contains('('));
}

#[test]
fn git_spans_omits_head_on_failed_resolve_and_loading() {
    // A failed resolution has no confirmed root and cannot claim a head.
    let mut failed = named_state(WorkState::Failed, "topic", Upstream::Absent);
    failed.root = None;
    let text = spans_text(&failed, 20);
    assert!(!text.contains("topic"), "failed resolution: {text}");
    // Loading stays the ellipsis, never a name.
    let text: String = git_spans(None, 20, Theme::LIGHT)
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(!text.contains("topic"));
}

#[test]
fn narrow_width_drops_upstream_then_dirty_then_head() {
    let row = named_state(
        WorkState::Measurable {
            added: 3,
            deleted: 4,
        },
        "topic",
        Upstream::Counts {
            ahead: 0,
            behind: 1,
        },
    );
    // Full line is 16 cells: icon, name, pair, arrow.
    assert_eq!(spans_text(&row, 16), "\u{ec6f} topic +3 −4 ↓1");
    // Arrow drops first; the local pair stays exact.
    assert_eq!(spans_text(&row, 13), "\u{ec6f} topic +3 −4");
    // Pair drops as a pair; the full name stands alone.
    assert_eq!(spans_text(&row, 12), "\u{ec6f} topic");
    // Name truncates prefix-kept, never beside dirty.
    assert_eq!(spans_text(&row, 6), "\u{ec6f} top…");
    assert_eq!(spans_text(&row, 4), "\u{ec6f} t…");
    // No room for even `t…`: bare icon, never a lone `…`.
    assert_eq!(spans_text(&row, 3), "\u{ec6f}");
    for w in 0..25 {
        let text = spans_text(&row, w);
        let has_arrows = text.contains('↓') || text.contains('↑');
        assert!(
            !has_arrows || text.contains('+'),
            "arrows without pair at {w}: {text}",
        );
        assert!(
            !text.contains('+') || text.contains("topic"),
            "pair without full name at {w}: {text}",
        );
        assert!(
            !text.contains('…') || !(has_arrows || text.contains('+')),
            "truncated name beside dirty/upstream at {w}: {text}",
        );
        if w > 0 {
            assert!(text.starts_with("\u{ec6f}"), "icon first at {w}: {text}",);
        }
    }
}

#[test]
fn git_line_renders_branch_on_clean_and_dirty_rows() {
    let sel = selection(&["/main", "/side"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.git.apply(vec![
        (
            "/main".to_string(),
            CandidateState {
                root: Some("/main".to_string()),
                linked: false,
                primary: None,
                state: WorkState::Clean,
                head: Head::Named("topic".to_string()),
                upstream: Upstream::Absent,
                pull_request: None,
                pull_request_checked: false,
            },
        ),
        (
            "/side".to_string(),
            CandidateState {
                root: Some("/side".to_string()),
                linked: true,
                primary: None,
                state: WorkState::Measurable {
                    added: 3,
                    deleted: 4,
                },
                head: Head::Named("side".to_string()),
                upstream: Upstream::Absent,
                pull_request: None,
                pull_request_checked: false,
            },
        ),
    ]);
    let buf = render_list(&sel, &mut state, 40, 4);
    let first = row_text(&buf, 1, 40);
    let second = row_text(&buf, 3, 40);
    assert!(first.contains("topic"), "clean branch row: {first}");
    assert!(!first.contains('+'));
    assert!(second.contains("side +3 −4"), "linked row: {second}");
}

#[test]
fn git_spans_upstream_arrows_beside_local_state() {
    let red = Color::Rgb(0x98, 0x22, 0x2A);
    let green = Color::Rgb(0x82, 0xA7, 0x62);
    // Clean + behind: icon and one red arrow, no counts.
    let behind = upstream_state(
        WorkState::Clean,
        Upstream::Counts {
            ahead: 0,
            behind: 1,
        },
    );
    let spans = git_spans(Some(&behind), 20, Theme::LIGHT);
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("↓1"), "behind arrow: {text}");
    assert!(!text.contains('+') && !text.contains('−'));
    assert!(!text.contains('↑'));
    let arrow = spans.iter().find(|s| s.content.contains('↓')).unwrap();
    assert_eq!(arrow.style.fg, Some(red));
    assert!(
        !arrow.style.add_modifier.contains(Modifier::BOLD),
        "arrows stay normal weight",
    );
    // Ahead only: green arrow, no behind arrow.
    let ahead = upstream_state(
        WorkState::Clean,
        Upstream::Counts {
            ahead: 2,
            behind: 0,
        },
    );
    let text = spans_text(&ahead, 20);
    assert!(text.contains("↑2"), "ahead arrow: {text}");
    assert!(!text.contains('↓'));
    let spans = git_spans(Some(&ahead), 20, Theme::LIGHT);
    let arrow = spans.iter().find(|s| s.content.contains('↑')).unwrap();
    assert_eq!(arrow.style.fg, Some(green));
    // Marker + behind: marker keeps its accent, arrow follows.
    let marked = upstream_state(
        WorkState::Marker,
        Upstream::Counts {
            ahead: 0,
            behind: 3,
        },
    );
    let text = spans_text(&marked, 20);
    assert!(text.contains("•") && text.contains("↓3"), "{text}");
    // Dirty + diverged: local pair first, then both arrows.
    let diverged = upstream_state(
        WorkState::Measurable {
            added: 3,
            deleted: 12,
        },
        Upstream::Counts {
            ahead: 2,
            behind: 1,
        },
    );
    let text = spans_text(&diverged, 30);
    assert!(text.contains("+3 −12 ↓1 ↑2"), "pair then arrows: {text}",);
}

#[test]
fn git_spans_omit_zero_absent_and_failed_upstream() {
    // Equal and absent paint no arrows.
    for upstream in [
        Upstream::Counts {
            ahead: 0,
            behind: 0,
        },
        Upstream::Absent,
    ] {
        let text = spans_text(&upstream_state(WorkState::Clean, upstream), 20);
        assert!(
            !text.contains('↓') && !text.contains('↑'),
            "no arrows: {text}",
        );
    }
    // Failed stays the failure glyph even with divergence.
    let failed = upstream_state(
        WorkState::Failed,
        Upstream::Counts {
            ahead: 1,
            behind: 1,
        },
    );
    let text = spans_text(&failed, 20);
    assert!(
        !text.contains('↓') && !text.contains('↑'),
        "failed glyph only: {text}",
    );
    assert!(!text.contains('+') && !text.contains('−'));
}

#[test]
fn narrow_width_drops_upstream_before_local_counts() {
    let diverged = upstream_state(
        WorkState::Measurable {
            added: 12_345_678,
            deleted: 9,
        },
        Upstream::Counts {
            ahead: 0,
            behind: 1,
        },
    );
    // Exact local `icon +12345678 −9` is 14 cells; `↓1` needs 3
    // more. At 16 the arrows drop while the local pair stays
    // exact — upstream never forces local compaction.
    let text = spans_text(&diverged, 16);
    assert!(text.contains("+12345678"), "local stays exact: {text}");
    assert!(!text.contains('↓'), "arrows drop first: {text}");
    // Room for both: arrows return beside the exact pair.
    let text = spans_text(&diverged, 17);
    assert!(text.contains("+12345678 −9 ↓1"), "arrows return: {text}",);
    // Tighter still: the local pair drops as a pair too.
    let text = spans_text(&diverged, 2);
    assert!(!text.contains('+') && !text.contains('↓'));
}

#[test]
fn narrow_band_never_reattaches_upstream_after_local_drop() {
    // Diverged measurable across the full narrow band: arrows
    // imply the local pair, and the diverged pair stays
    // both-or-neither at every width. (At width 4 the exact
    // pair is long gone while `icon ↓7` would still fit: that
    // bare-icon reattach is the inversion this pins.)
    let diverged = upstream_state(
        WorkState::Measurable {
            added: 12_345_678,
            deleted: 9,
        },
        Upstream::Counts {
            ahead: 5,
            behind: 7,
        },
    );
    for w in 0..30 {
        let text = spans_text(&diverged, w);
        let has_arrows = text.contains('↓') || text.contains('↑');
        assert!(
            !has_arrows || text.contains('+'),
            "arrows without pair at {w}: {text}",
        );
        assert_eq!(
            text.contains('↓'),
            text.contains('↑'),
            "split diverged pair at {w}: {text}",
        );
    }
    // Marker + upstream: a dropped marker keeps arrows dropped.
    let marked = upstream_state(
        WorkState::Marker,
        Upstream::Counts {
            ahead: 0,
            behind: 3,
        },
    );
    for w in 0..20 {
        let text = spans_text(&marked, w);
        assert!(
            !text.contains('↓') || text.contains('•'),
            "arrow without marker at {w}: {text}",
        );
    }
    // Clean + behind: arrows may sit beside the bare icon, and
    // the fit is monotonic — once dropped, they stay dropped.
    let behind = upstream_state(
        WorkState::Clean,
        Upstream::Counts {
            ahead: 0,
            behind: 1,
        },
    );
    let mut seen_drop = false;
    for w in (0..20).rev() {
        let text = spans_text(&behind, w);
        if text.contains('↓') {
            assert!(!seen_drop, "arrow returns after dropping at {w}: {text}",);
        } else {
            seen_drop = true;
        }
    }
}

#[test]
fn git_line_renders_upstream_on_clean_and_dirty_rows() {
    let sel = selection(&["/behind", "/diverged"]);
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.git.apply(vec![
        (
            "/behind".to_string(),
            CandidateState {
                root: Some("/behind".to_string()),
                linked: false,
                primary: None,
                state: WorkState::Clean,
                head: Head::Absent,
                upstream: Upstream::Counts {
                    ahead: 0,
                    behind: 1,
                },
                pull_request: None,
                pull_request_checked: false,
            },
        ),
        (
            "/diverged".to_string(),
            CandidateState {
                root: Some("/diverged".to_string()),
                linked: true,
                primary: None,
                state: WorkState::Measurable {
                    added: 3,
                    deleted: 4,
                },
                head: Head::Absent,
                upstream: Upstream::Counts {
                    ahead: 2,
                    behind: 1,
                },
                pull_request: None,
                pull_request_checked: false,
            },
        ),
    ]);
    let buf = render_list(&sel, &mut state, 40, 4);
    let first = row_text(&buf, 1, 40);
    let second = row_text(&buf, 3, 40);
    assert!(first.contains("↓1"), "clean-behind row: {first}");
    assert!(!first.contains('+'));
    assert!(
        second.contains("+3 −4 ↓1 ↑2"),
        "dirty-diverged row: {second}",
    );
}

/// A home that matches none of the fixed shaping paths, so those
/// cases stay deterministic without reading the real `$HOME`.
const FAR_HOME: &str = "/test-home";

/// Type `query` into a fresh selection through the real key
/// pipeline, so `matches` carry original fuzzy byte ranges.
fn typed_selection(paths: &[&str], query: &str) -> Selection {
    let mut sel = selection(paths);
    for c in query.chars() {
        sel.handle_key(key(KeyCode::Char(c)));
    }
    sel
}

/// Render through the real pipeline with a deterministic home.
/// Production still defaults from `$HOME` in
/// `SessionsListState::new`; tests assign the fake home directly
/// (same pattern as `state.git`) so parallel tests never touch
/// the process environment.
fn render_case(
    paths: &[&str],
    home: Option<&str>,
    query: &str,
    w: u16,
    h: u16,
) -> Buffer {
    let sel = typed_selection(paths, query);
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.home = home.map(str::to_string);
    render_list(&sel, &mut state, w, h)
}

/// Visible characters painted with the hit style on row `y`.
fn hit_spelling(buf: &Buffer, y: u16, w: u16) -> String {
    let yellow = Theme::LIGHT.accent;
    (0..w)
        .filter(|&x| buf[(x, y)].fg == yellow)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

fn hit_count(buf: &Buffer, y: u16, w: u16) -> usize {
    let yellow = Theme::LIGHT.accent;
    (0..w).filter(|&x| buf[(x, y)].fg == yellow).count()
}

#[test]
fn rendered_short_path_renders_in_full_unhighlighted() {
    // Empty query: every session candidate, no hits.
    let buf = render_case(&["/alpha"], Some(FAR_HOME), "", 40, 2);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("/alpha"), "full path: {row}");
    assert_eq!(hit_count(&buf, 0, 40), 0, "no hits: {row}");
}

#[test]
fn rendered_prefix_basename_shortening_keeps_basename_hit() {
    // Width 12 leaves path budget 10: "/x/ve" + `…` + "name".
    let buf =
        render_case(&["/x/very-long-parent/name"], Some(FAR_HOME), "n", 12, 2);
    let row = row_text(&buf, 0, 12);
    assert_eq!(row, "  /x/ve…name");
    // The surviving basename hit still paints ...
    assert_eq!(hit_spelling(&buf, 0, 12), "n");
    // ... while the inserted shortening ellipsis never paints.
    let ell = (0..12).find(|&x| buf[(x, 0)].symbol() == "…").unwrap();
    assert_ne!(buf[(ell, 0)].fg, Theme::LIGHT.accent);
}

#[test]
fn rendered_dropped_middle_hit_disappears() {
    // Same shape, but `g` lives only in the shortened middle.
    let buf =
        render_case(&["/x/very-long-parent/name"], Some(FAR_HOME), "g", 12, 2);
    let row = row_text(&buf, 0, 12);
    assert_eq!(row, "  /x/ve…name");
    assert_eq!(hit_count(&buf, 0, 12), 0, "middle hit gone: {row}");
}

#[test]
fn rendered_tail_form_keeps_only_tail_hits() {
    // Width 6 leaves path budget 4: no basename room, `…` + tail.
    let buf = render_case(&["abcdefgh"], Some(FAR_HOME), "h", 6, 2);
    assert_eq!(row_text(&buf, 0, 6), "  …fgh");
    assert_eq!(hit_spelling(&buf, 0, 6), "h");
    // The dropped prefix hit never paints onto the tail.
    let buf = render_case(&["abcdefgh"], Some(FAR_HOME), "a", 6, 2);
    assert_eq!(row_text(&buf, 0, 6), "  …fgh");
    assert_eq!(hit_count(&buf, 0, 6), 0);
}

#[test]
fn rendered_exact_home_becomes_tilde() {
    let buf = render_case(&["/home/me"], Some("/home/me"), "", 40, 2);
    let row = row_text(&buf, 0, 40);
    assert!(row.starts_with("  ~"), "exact home: {row}");
    assert!(!row.contains("home"), "no raw home: {row}");
    assert_eq!(hit_count(&buf, 0, 40), 0);
}

#[test]
fn rendered_home_descendant_abbreviates() {
    let buf = render_case(&["/home/me/a/b"], Some("/home/me"), "", 40, 2);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("~/a/b"), "descendant: {row}");
    assert!(!row.contains("/home/me"), "no raw home: {row}");
}

#[test]
fn rendered_home_sibling_missing_and_empty_leave_path() {
    // A sibling that merely shares a string prefix keeps its path.
    let buf = render_case(&["/home/me2/x"], Some("/home/me"), "", 40, 2);
    assert!(row_text(&buf, 0, 40).contains("/home/me2/x"));
    // Missing or empty home invents no tilde.
    let buf = render_case(&["/home/me/proj"], None, "", 40, 2);
    assert!(row_text(&buf, 0, 40).contains("/home/me/proj"));
    let buf = render_case(&["/home/me/proj"], Some(""), "", 40, 2);
    assert!(row_text(&buf, 0, 40).contains("/home/me/proj"));
}

#[test]
fn rendered_home_collapse_keeps_tail_hits_drops_prefix_hits() {
    let entry = "/home/me/projects/alpha";
    // `l` sits in the surviving tail past the collapsed prefix.
    let buf = render_case(&[entry], Some("/home/me"), "l", 40, 2);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("~/projects/alpha"), "shaped: {row}");
    assert_eq!(hit_spelling(&buf, 0, 40), "l");
    // `m` sits only inside the collapsed `$HOME` prefix.
    let buf = render_case(&[entry], Some("/home/me"), "m", 40, 2);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("~/projects/alpha"), "shaped: {row}");
    assert_eq!(hit_count(&buf, 0, 40), 0, "prefix hit gone");
    // The inserted `~` itself is never painted as a hit.
    assert_ne!(buf[(2, 0)].fg, Theme::LIGHT.accent);
}

#[test]
fn rendered_unicode_highlights_survive_abbreviation() {
    let buf =
        render_case(&["/home/me/café/naïve"], Some("/home/me"), "é", 40, 2);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("~/café/naïve"), "shaped: {row}");
    assert_eq!(hit_spelling(&buf, 0, 40), "é");
}

#[test]
fn rendered_unicode_highlights_survive_shortening() {
    // Width 12 leaves path budget 10: "/x/vé" + `…` + "nâme".
    let buf =
        render_case(&["/x/véry-long-parent/nâme"], Some(FAR_HOME), "â", 12, 2);
    let row = row_text(&buf, 0, 12);
    assert_eq!(row, "  /x/vé…nâme");
    assert_eq!(hit_spelling(&buf, 0, 12), "â");
    // A multibyte neighbour in the dropped middle paints nothing.
    let buf =
        render_case(&["/x/véry-long-parent/nâme"], Some(FAR_HOME), "g", 12, 2);
    assert_eq!(hit_count(&buf, 0, 12), 0);
}

#[test]
fn rendered_unicode_abbreviation_and_shortening_together() {
    let entry = "/home/me/prójects/lóngname";
    // Width 14 leaves path budget 12: "~/p" + `…` + "lóngname".
    let buf = render_case(&[entry], Some("/home/me"), "ó", 14, 2);
    let row = row_text(&buf, 0, 14);
    assert_eq!(row, "  ~/p…lóngname");
    assert_eq!(hit_spelling(&buf, 0, 14), "ó");
    let buf = render_case(&[entry], Some("/home/me"), "j", 14, 2);
    assert_eq!(hit_count(&buf, 0, 14), 0, "dropped middle: {row}");
}

#[test]
fn rendered_literal_leading_ellipsis_keeps_prefix_hits() {
    // The real path starts with `…` and shortens to prefix form
    // "…/" + `…` + "bbb". The `…` hit sits in the retained
    // prefix, the `b` hit in the retained basename, and the `a`
    // hit in the dropped middle — so the row must paint exactly
    // "…b". Inferring tail form from the leading `…` instead
    // paints "/b", and merely dropping mismatched hits would
    // still lose the retained prefix hit and read "b".
    let buf = render_case(&["…/aaa/bbb"], None, "…ab", 8, 2);
    let row = row_text(&buf, 0, 8);
    assert_eq!(row, "  …/…bbb");
    assert_eq!(hit_spelling(&buf, 0, 8), "…b");
    let xs: Vec<u16> = (0..8)
        .filter(|&x| buf[(x, 0)].fg == Theme::LIGHT.accent)
        .collect();
    assert_eq!(xs, vec![2, 5], "prefix and basename cells: {row}");
    // The inserted `…` (visible index 2) is never a hit.
    assert_ne!(buf[(4, 0)].fg, Theme::LIGHT.accent);
}

#[test]
fn rendered_resize_reshapes_text_and_hits_together() {
    let entry = "/x/very-long-parent/name";
    let buf = render_case(&[entry], Some(FAR_HOME), "n", 40, 2);
    let wide = row_text(&buf, 0, 40);
    assert!(wide.contains(entry), "wide keeps full: {wide}");
    assert_eq!(hit_spelling(&buf, 0, 40), "n");
    let buf = render_case(&[entry], Some(FAR_HOME), "n", 12, 2);
    let narrow = row_text(&buf, 0, 12);
    assert_eq!(narrow, "  /x/ve…name");
    assert_eq!(hit_spelling(&buf, 0, 12), "n");
}

#[test]
fn rendered_selected_row_tint_keeps_hits() {
    let buf = render_case(&["/alpha", "/beta"], Some(FAR_HOME), "a", 40, 4);
    // First item is selected: full-width tint plus the hit.
    assert_eq!(hit_spelling(&buf, 0, 40), "a");
    let x = (0..40)
        .find(|&x| buf[(x, 0)].fg == Theme::LIGHT.accent)
        .expect("hit on selected row");
    assert_eq!(buf[(x, 0)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(39, 0)].bg, Theme::LIGHT.bg_alt);
    // Second item keeps the base background with its own hit.
    assert_eq!(hit_spelling(&buf, 2, 40), "a");
    assert_eq!(buf[(39, 2)].bg, Theme::LIGHT.bg);
}

#[test]
fn rendered_adjacent_hits_read_continuous() {
    let buf = render_case(&["/alpha"], Some(FAR_HOME), "alp", 40, 2);
    assert_eq!(hit_spelling(&buf, 0, 40), "alp");
    let xs: Vec<u16> = (0..40)
        .filter(|&x| buf[(x, 0)].fg == Theme::LIGHT.accent)
        .collect();
    assert_eq!(xs.len(), 3);
    assert_eq!(xs[2] - xs[0], 2, "contiguous: {xs:?}");
}

#[test]
fn rendered_disjoint_hits_stay_separate() {
    let buf = render_case(&["/axbxc"], Some(FAR_HOME), "abc", 40, 2);
    assert_eq!(hit_spelling(&buf, 0, 40), "abc");
    let xs: Vec<u16> = (0..40)
        .filter(|&x| buf[(x, 0)].fg == Theme::LIGHT.accent)
        .collect();
    assert_eq!(xs.len(), 3);
    assert!(xs[2] - xs[0] > 2, "gaps survive: {xs:?}");
}

#[test]
fn rendered_git_line_unaffected_by_path_shaping() {
    let sel = typed_selection(&["/x/very-long-parent/name"], "n");
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.home = Some(FAR_HOME.to_string());
    state.git.apply(vec![(
        "/x/very-long-parent/name".to_string(),
        CandidateState {
            root: Some("/x/very-long-parent/name".to_string()),
            linked: false,
            primary: None,
            state: WorkState::Measurable {
                added: 3,
                deleted: 12,
            },
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )]);
    let buf = render_list(&sel, &mut state, 12, 2);
    let path_row = row_text(&buf, 0, 12);
    let git_row = row_text(&buf, 1, 12);
    assert_eq!(path_row, "  /x/ve…name");
    assert_eq!(hit_spelling(&buf, 0, 12), "n");
    assert!(git_row.contains("+3 −12"), "git intact: {git_row}");
}

#[test]
fn rendered_shaping_leaves_ranking_and_query_alone() {
    let sel = typed_selection(&["/x/very-long-parent/name", "/other"], "name");
    assert_eq!(sel.query(), "name");
    assert_eq!(sel.matches().len(), 1);
    // Selection still holds the full unshaped candidate.
    assert_eq!(sel.matches()[0].entry, "/x/very-long-parent/name");
}

#[test]
fn rendered_tiny_areas_with_query_do_not_panic() {
    let sel = typed_selection(&["/a", "/b", "/c"], "a");
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.home = Some(FAR_HOME.to_string());
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 10));
    for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (10, 2), (2, 10), (5, 5)] {
        sel.render(Rect::new(0, 0, w, h), &mut buf, &mut state);
    }
}

/// Grouping B + tree spine (locked): config-group headers,
/// remainder-only children, nested linked worktrees, blanks
/// between groups. These tests target production `render_list`
/// with `state.groups` set directly: grouping is presentation
/// over `Selection` matches, so no config or git polling is
/// involved. Layout/nest/hit/teal/scroll cases fail until the
/// grouping render lands; navigation locks hold throughout.
const GROUP_TEAL: Color = Color::Rgb(0x3D, 0x7A, 0x6F);

/// SessionsListState with config groups assigned directly
/// (same pattern as `state.git` / `state.home` above), so
/// grouping stays presentation over matches.
fn grouped_state(groups: &[(&str, &str)]) -> SessionsListState {
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.home = Some(FAR_HOME.to_string());
    for (path, group) in groups {
        state.groups.insert(path.to_string(), group.to_string());
    }
    state
}

/// Clean git state with an explicit nest link for grouped
/// fixtures: `primary` names the main worktree root this
/// linked checkout nests under, if any.
fn grouped_git(
    root: &str,
    linked: bool,
    primary: Option<&str>,
) -> (String, CandidateState) {
    (
        root.to_string(),
        CandidateState {
            root: Some(root.to_string()),
            linked,
            primary: primary.map(str::to_string),
            state: WorkState::Clean,
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )
}

#[test]
fn grouped_headers_render_comment_without_tint_or_glyph() {
    let sel = selection(&["/parent/alpha", "/other/beta"]);
    let mut state = grouped_state(&[
        ("/parent/alpha", "/parent"),
        ("/other/beta", "/other"),
    ]);
    let buf = render_list(&sel, &mut state, 40, 20);
    let comment = Theme::LIGHT.comment;
    let bg = Theme::LIGHT.bg;
    // First group header: indented, comment, base background.
    let header = row_text(&buf, 0, 40);
    assert!(header.starts_with("  \u{25be} /parent"), "header: {header}");
    assert_eq!(buf[(2, 0)].fg, comment);
    assert_eq!(buf[(0, 0)].bg, bg);
    assert_eq!(buf[(39, 0)].bg, bg);
    // Headers carry no tree glyph.
    assert!(
        !header.contains('\u{251c}')
            && !header.contains('\u{2514}')
            && !header.contains('\u{2502}'),
        "header has no glyph: {header}"
    );
    // Blank row between groups, then the second header.
    let blank = row_text(&buf, 3, 40);
    assert!(blank.trim().is_empty(), "blank between: {blank}");
    assert_eq!(buf[(0, 3)].bg, bg);
    let second = row_text(&buf, 4, 40);
    assert!(second.starts_with("  \u{25be} /other"), "second: {second}");
}

#[test]
fn grouped_folded_header_shows_glyph_count_and_tint() {
    let sel = selection(&["/parent/alpha", "/parent/beta", "/other/gamma"]);
    let mut state = grouped_state(&[
        ("/parent/alpha", "/parent"),
        ("/parent/beta", "/parent"),
        ("/other/gamma", "/other"),
    ]);
    state.group_order = vec!["/parent".to_string(), "/other".to_string()];
    state.folded.insert(("/parent".to_string(), 0));
    state.active_header = Some((0, 0));
    let buf = render_list(&sel, &mut state, 40, 20);
    let header = row_text(&buf, 0, 40);
    assert!(
        header.starts_with("  \u{25b8} /parent (2)"),
        "folded: {header}"
    );
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(39, 0)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(2, 0)].fg, Theme::LIGHT.comment);
    let all: String = (0..8)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!all.contains("alpha"), "hidden children: {all}");
    assert!(!all.contains("beta"), "hidden children: {all}");
    // Folded header, blank, unfolded header, child rows.
    let other = row_text(&buf, 2, 40);
    assert!(other.starts_with("  \u{25be} /other"), "open: {other}");
    assert!(!other.contains('('), "no count: {other}");
    assert_eq!(buf[(0, 2)].bg, Theme::LIGHT.bg);
}

#[test]
fn grouped_children_show_remainder_only_with_tree_spine() {
    let sel = selection(&["/parent/alpha", "/parent/beta"]);
    let mut state = grouped_state(&[
        ("/parent/alpha", "/parent"),
        ("/parent/beta", "/parent"),
    ]);
    let buf = render_list(&sel, &mut state, 40, 20);
    let comment = Theme::LIGHT.comment;
    // Non-last child branches; last child corners.
    let first = row_text(&buf, 1, 40);
    assert!(first.starts_with("  \u{251c} alpha"), "first: {first}");
    assert!(!first.contains("/parent"), "remainder: {first}");
    assert_eq!(buf[(2, 1)].fg, comment);
    // The spine continues through that child's git row.
    assert_eq!(buf[(2, 2)].symbol(), "\u{2502}");
    assert_eq!(buf[(2, 2)].fg, comment);
    let last = row_text(&buf, 3, 40);
    assert!(last.starts_with("  \u{2514} beta"), "last: {last}");
    assert!(!last.contains("/parent"), "remainder: {last}");
    // A last child's git row carries no continuation.
    assert!(row_text(&buf, 4, 40).starts_with("      "));
}

#[test]
fn grouped_empty_groups_hide_without_leaving_gaps() {
    let sel = typed_selection(&["/a/apple", "/b/cherry", "/c/apricot"], "ap");
    assert_eq!(sel.matches().len(), 2);
    let mut state = grouped_state(&[
        ("/a/apple", "/a"),
        ("/b/cherry", "/b"),
        ("/c/apricot", "/c"),
    ]);
    let buf = render_list(&sel, &mut state, 40, 20);
    // Seven content rows: header, two child rows, blank,
    // header, two child rows.
    let rows: Vec<String> = (0..7).map(|y| row_text(&buf, y, 40)).collect();
    let all = rows.join("\n");
    assert!(!all.contains("cherry"), "filtered out: {all}");
    assert!(!all.contains("/b"), "hidden header: {all}");
    let headers: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.starts_with("  \u{25be} /").then_some(i))
        .collect();
    assert_eq!(headers.len(), 2, "two headers: {all}");
    let (h0, h1) = (headers[0], headers[1]);
    // Each visible group shows a lone cornered child, with one
    // blank row between the groups and none elsewhere.
    assert!(rows[h0 + 1].starts_with("  \u{2514} "), "{all}");
    assert!(rows[h1].starts_with("  \u{25be} /"), "{all}");
    assert!(rows[h1 + 1].starts_with("  \u{2514} "), "{all}");
    let blanks: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.trim().is_empty().then_some(i))
        .collect();
    assert_eq!(blanks, vec![h0 + 3], "single separator: {all}");
    assert_eq!(h1, h0 + 4, "groups stay packed: {all}");
}

#[test]
fn grouped_selection_tint_covers_selected_child_only() {
    let mut sel = selection(&["/parent/alpha", "/parent/beta"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 2);
    let mut state = grouped_state(&[
        ("/parent/alpha", "/parent"),
        ("/parent/beta", "/parent"),
    ]);
    let buf = render_list(&sel, &mut state, 40, 20);
    let bg = Theme::LIGHT.bg;
    let bg_alt = Theme::LIGHT.bg_alt;
    // Header never tinted; unselected child keeps the base.
    assert!(row_text(&buf, 0, 40).starts_with("  \u{25be} /parent"));
    assert_eq!(buf[(0, 0)].bg, bg);
    assert_eq!(buf[(39, 0)].bg, bg);
    assert_eq!(buf[(0, 1)].bg, bg);
    assert_eq!(buf[(39, 2)].bg, bg);
    // Selected child tints both rows, full width.
    assert_eq!(buf[(0, 3)].bg, bg_alt);
    assert_eq!(buf[(39, 3)].bg, bg_alt);
    assert_eq!(buf[(0, 4)].bg, bg_alt);
    assert_eq!(buf[(39, 4)].bg, bg_alt);
}

#[test]
fn grouped_hits_paint_yellow_on_remainder_only() {
    // A multibyte hit inside the remainder still paints.
    let sel = typed_selection(&["/parent/caf\u{e9}"], "\u{e9}");
    let mut state = grouped_state(&[("/parent/caf\u{e9}", "/parent")]);
    let buf = render_list(&sel, &mut state, 40, 20);
    assert_eq!(hit_spelling(&buf, 1, 40), "\u{e9}");
}

#[test]
fn grouped_linked_icon_is_teal_ordinary_stays_blue() {
    let sel = selection(&["/parent/main", "/parent/side"]);
    let mut state = grouped_state(&[
        ("/parent/main", "/parent"),
        ("/parent/side", "/parent"),
    ]);
    state.git.apply(vec![
        grouped_git("/parent/main", false, None),
        grouped_git("/parent/side", true, None),
    ]);
    let buf = render_list(&sel, &mut state, 40, 20);
    let git_icon = Theme::LIGHT.git_icon;
    // Ordinary icon stays git_icon blue ...
    let ox = (0..40)
        .find(|&x| buf[(x, 2)].symbol() == "\u{ec6f}")
        .expect("ordinary icon on row 2");
    assert_eq!(buf[(ox, 2)].fg, git_icon);
    // ... while the linked icon is a distinct teal.
    let lx = (0..40)
        .find(|&x| buf[(x, 4)].symbol() == "\u{ec7d}")
        .expect("linked icon on row 4");
    assert_eq!(buf[(lx, 4)].fg, GROUP_TEAL);
}

#[test]
fn grouped_linked_worktree_nests_under_main_with_second_column() {
    let sel =
        selection(&["/p/portal", "/p/portal-wt1", "/p/portal-wt2", "/p/zeta"]);
    let mut state = grouped_state(&[
        ("/p/portal", "/p"),
        ("/p/portal-wt1", "/p"),
        ("/p/portal-wt2", "/p"),
        ("/p/zeta", "/p"),
    ]);
    state.git.apply(vec![
        grouped_git("/p/portal", false, None),
        grouped_git("/p/portal-wt1", true, Some("/p/portal")),
        grouped_git("/p/portal-wt2", true, Some("/p/portal")),
        grouped_git("/p/zeta", false, None),
    ]);
    let buf = render_list(&sel, &mut state, 40, 20);
    // Rows: 0 header, 1-2 main, 3-4 first nest, 5-6 second
    // nest, 7-8 last sibling.
    assert!(row_text(&buf, 1, 40).starts_with("  \u{251c} portal"));
    let wt1 = row_text(&buf, 3, 40);
    assert!(wt1.starts_with("  \u{2502} \u{251c} portal-wt1"), "{wt1}");
    assert!(!wt1.contains("/p/"), "basename only: {wt1}");
    // The second tree column continues through the nested git
    // row.
    assert_eq!(buf[(2, 4)].symbol(), "\u{2502}");
    assert_eq!(buf[(4, 4)].symbol(), "\u{2502}");
    let wt2 = row_text(&buf, 5, 40);
    assert!(wt2.starts_with("  \u{2502} \u{2514} portal-wt2"), "{wt2}");
    assert!(row_text(&buf, 7, 40).starts_with("  \u{2514} zeta"));
    // Remainder hits still map onto the nested basename: `p`
    // sits at the remainder start on both rows.
    let sel = typed_selection(&["/p/portal", "/p/portal-wt1"], "p");
    assert_eq!(sel.matches().len(), 2);
    let mut state =
        grouped_state(&[("/p/portal", "/p"), ("/p/portal-wt1", "/p")]);
    let buf = render_list(&sel, &mut state, 40, 20);
    assert_eq!(hit_spelling(&buf, 1, 40), "p");
    assert_eq!(hit_spelling(&buf, 3, 40), "p");
}

#[test]
fn grouped_orphan_worktree_stays_flat_without_its_main() {
    // The main is filtered out: the linked child renders as a
    // flat group child, never a hanging nest.
    let sel = typed_selection(&["/p/portal", "/p/solo-wt"], "solo");
    assert_eq!(sel.matches().len(), 1);
    assert_eq!(sel.matches()[0].entry, "/p/solo-wt");
    let mut state = grouped_state(&[("/p/portal", "/p"), ("/p/solo-wt", "/p")]);
    state
        .git
        .apply(vec![grouped_git("/p/solo-wt", true, Some("/p/portal"))]);
    let buf = render_list(&sel, &mut state, 40, 20);
    let header = row_text(&buf, 0, 40);
    assert!(header.starts_with("  \u{25be} /p"), "header: {header}");
    let path = row_text(&buf, 1, 40);
    assert!(path.starts_with("  \u{2514} solo-wt"), "flat: {path}");
    assert!(!path.contains('\u{2502}'), "no nest column: {path}");
}

#[test]
fn grouped_navigation_skips_headers_and_blanks() {
    let mut sel = selection(&["/g1/a1", "/g1/a2", "/g2/b1"]);
    let groups = [("/g1/a1", "/g1"), ("/g1/a2", "/g1"), ("/g2/b1", "/g2")];
    // Down walks children only: headers and blanks never take
    // the selection, and the last child holds.
    sel.handle_key(key(KeyCode::Down));
    assert_eq!(sel.selected_line(), 2);
    sel.handle_key(key(KeyCode::Down));
    assert_eq!(sel.selected_line(), 3);
    sel.handle_key(key(KeyCode::Down));
    assert_eq!(sel.selected_line(), 3);
    let mut state = grouped_state(&groups);
    let buf = render_list(&sel, &mut state, 40, 20);
    // Tint sits on the last child's two rows; headers stay base.
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg);
    assert_eq!(buf[(0, 7)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(0, 8)].bg, Theme::LIGHT.bg_alt);
    // Home/End land on the first/last child.
    sel.handle_key(key(KeyCode::Home));
    assert_eq!(sel.selected_line(), 1);
    sel.handle_key(key(KeyCode::Up));
    assert_eq!(sel.selected_line(), 1);
    let mut state = grouped_state(&groups);
    let buf = render_list(&sel, &mut state, 40, 20);
    assert_eq!(buf[(0, 1)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg);
}

#[test]
fn grouped_selected_child_and_git_row_stay_in_view() {
    let mut sel = selection(&["/g1/a1", "/g1/a2", "/g2/b1"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(sel.selected_line(), 3);
    let mut state = grouped_state(&[
        ("/g1/a1", "/g1"),
        ("/g1/a2", "/g1"),
        ("/g2/b1", "/g2"),
    ]);
    let buf = render_list(&sel, &mut state, 40, 5);
    // The scrolled window holds the second header plus the
    // whole selected child block: header, path, and git rows.
    let visible: String = (0..5)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        visible.contains("  \u{25be} /g2"),
        "header in view: {visible}"
    );
    assert!(visible.contains("b1"), "path in view: {visible}");
    // The git row itself is in view: child git indent, not a
    // header or a path row (no git state is set, so the row
    // shows the loading ellipsis).
    assert!(
        row_text(&buf, 4, 40).starts_with("      "),
        "git row in view: {visible}"
    );
    assert!(row_text(&buf, 3, 40).contains("b1"));
}

#[test]
fn grouped_spine_glyphs_allowed_but_frames_forbidden() {
    let sel = selection(&["/parent/alpha", "/parent/beta"]);
    let mut state = grouped_state(&[
        ("/parent/alpha", "/parent"),
        ("/parent/beta", "/parent"),
    ]);
    let buf = render_list(&sel, &mut state, 40, 20);
    // The spine reads in the indent columns, comment-colored.
    assert_eq!(buf[(2, 1)].symbol(), "\u{251c}");
    assert_eq!(buf[(2, 1)].fg, Theme::LIGHT.comment);
    // Item frames never appear, spine or not.
    for y in 0..5 {
        let row = row_text(&buf, y, 40);
        assert!(
            !row.chars().any(|c| matches!(
                c,
                '\u{250c}'
                    | '\u{2510}'
                    | '\u{2518}'
                    | '\u{2500}'
                    | '\u{250f}'
                    | '\u{2513}'
                    | '\u{2517}'
                    | '\u{251b}'
                    | '\u{2501}'
                    | '\u{2503}'
            )),
            "no frames: {row}"
        );
    }
}

#[test]
fn grouped_enter_activates_full_candidate_path() {
    // Headers never enter Selection: Enter activates the full
    // candidate path, never a remainder.
    let mut sel = selection(&["/parent/alpha", "/parent/beta"]);
    sel.handle_key(key(KeyCode::End));
    assert_eq!(
        sel.handle_key(key(KeyCode::Enter)),
        Intent::Activate("/parent/beta".to_string())
    );
}

#[test]
fn grouped_header_abbreviates_home() {
    let sel = selection(&["/home/me/work/proj"]);
    let mut state = grouped_state(&[("/home/me/work/proj", "/home/me/work")]);
    state.home = Some("/home/me".to_string());
    let buf = render_list(&sel, &mut state, 40, 20);
    let header = row_text(&buf, 0, 40);
    assert!(header.starts_with("  \u{25be} ~/work"), "header: {header}");
    assert!(!header.contains("/home/me"), "no raw home: {header}");
    assert_eq!(hit_count(&buf, 0, 40), 0, "empty query: {header}");
    // The remainder is not double-abbreviated.
    let path = row_text(&buf, 1, 40);
    assert!(path.contains("proj"), "remainder: {path}");
    assert!(!path.contains('~'), "no tilde in child: {path}");
}

#[test]
fn grouped_header_paints_prefix_hits_that_survive_abbreviation() {
    // Hits that still sit on `~/work/pre-vieitesss` paint;
    // remainder `claims` paints on the child, not the header.
    let entry = "/home/me/work/pre-vieitesss/claims";
    let mut sel = typed_selection(&[entry], "pre-vieitesssclaims");
    let mut state = grouped_state(&[(entry, "/home/me/work/pre-vieitesss")]);
    state.home = Some("/home/me".to_string());
    apply_query_folds(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    let header = row_text(&buf, 0, 40);
    assert!(
        header.starts_with("  \u{25be} ~/work/pre-vieitesss"),
        "header: {header}"
    );
    assert_eq!(hit_spelling(&buf, 0, 40), "pre-vieitesss");
    assert_ne!(buf[(2, 0)].fg, Theme::LIGHT.accent, "glyph: {header}");
    let tilde = (0..40).find(|&x| buf[(x, 0)].symbol() == "~").unwrap();
    assert_ne!(buf[(tilde, 0)].fg, Theme::LIGHT.accent, "~ : {header}");
    let path = row_text(&buf, 1, 40);
    assert!(path.contains("claims"), "remainder: {path}");
    assert_eq!(hit_spelling(&buf, 1, 40), "claims");

    // A hit that lives only in collapsed `$HOME` drops; the
    // remainder still paints on the child.
    let mut sel = typed_selection(&[entry], "hclaims");
    apply_query_folds(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    let header = row_text(&buf, 0, 40);
    assert!(
        header.starts_with("  \u{25be} ~/work/pre-vieitesss"),
        "header: {header}"
    );
    assert_eq!(hit_spelling(&buf, 0, 40), "", "home hit gone: {header}");
    let tilde = (0..40).find(|&x| buf[(x, 0)].symbol() == "~").unwrap();
    assert_ne!(buf[(tilde, 0)].fg, Theme::LIGHT.accent);
    assert_eq!(hit_spelling(&buf, 1, 40), "claims");
}

#[test]
fn grouped_query_orders_groups_by_best_match() {
    // "ab" ranks the shorter g2 entry first; with a non-empty
    // query the group containing that best match renders first,
    // even though resolved order saw g1 first. Within a group,
    // children stay in matches() rank order (one child each).
    let mut sel = typed_selection(&["/g1/xab", "/g2/ab"], "ab");
    assert_eq!(sel.matches()[0].entry, "/g2/ab");
    assert_eq!(sel.matches()[1].entry, "/g1/xab");
    let mut state = grouped_state(&[("/g1/xab", "/g1"), ("/g2/ab", "/g2")]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/g2/ab".to_string()),
            VisualTarget::Child("/g1/xab".to_string()),
        ]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    assert!(row_text(&buf, 0, 40).starts_with("  \u{25be} /g2"));
    // Unfolded headers never enter Selection and never tint.
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg);
    assert_eq!(buf[(39, 0)].bg, Theme::LIGHT.bg);
    assert_eq!(buf[(0, 1)].bg, Theme::LIGHT.bg_alt);
    let second = (0..20)
        .map(|y| row_text(&buf, y, 40))
        .find(|r| r.starts_with("  \u{25be} /g1"))
        .expect("g1 still visible");
    assert!(second.starts_with("  \u{25be} /g1"));
}

#[test]
fn grouped_query_interleaves_remainder_hits_by_rank() {
    // Screenshot shape: two groups, a strong remainder hit in
    // each and a weaker remainder hit in the first. Rank is
    // strong-A, strong-B, weak-A; clustering group A would put
    // weak-A above strong-B. Each consecutive run of a group
    // key still gets its own header, so /g1 appears twice.
    let mut sel = typed_selection(&["/g1/azb", "/g1/ab", "/g2/ab2"], "ab");
    assert_eq!(sel.matches()[0].entry, "/g1/ab");
    assert_eq!(sel.matches()[1].entry, "/g2/ab2");
    assert_eq!(sel.matches()[2].entry, "/g1/azb");
    let mut state = grouped_state(&[
        ("/g1/azb", "/g1"),
        ("/g1/ab", "/g1"),
        ("/g2/ab2", "/g2"),
    ]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/g1/ab".to_string()),
            VisualTarget::Child("/g2/ab2".to_string()),
            VisualTarget::Child("/g1/azb".to_string()),
        ]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    let rows: Vec<String> = (0..20).map(|y| row_text(&buf, y, 40)).collect();
    let headers: Vec<&str> = rows
        .iter()
        .filter(|r| r.contains('\u{25be}'))
        .map(|r| r.trim_end())
        .collect();
    assert_eq!(headers.len(), 3, "header per run: {rows:?}");
    assert!(headers[0].starts_with("  \u{25be} /g1"), "{headers:?}");
    assert!(headers[1].starts_with("  \u{25be} /g2"), "{headers:?}");
    assert!(headers[2].starts_with("  \u{25be} /g1"), "{headers:?}");
    let children: Vec<&str> = rows
        .iter()
        .filter(|r| r.contains('\u{251c}') || r.contains('\u{2514}'))
        .map(|r| r.as_str())
        .collect();
    assert_eq!(children.len(), 3, "{children:?}");
    assert!(children[0].contains("ab") && !children[0].contains("azb"));
    assert!(children[1].contains("ab2"), "{children:?}");
    assert!(children[2].contains("azb"), "{children:?}");
}

#[test]
fn collapse_one_search_run_leaves_the_other_same_key_run_open() {
    // Rank interleaves two /g1 remainder runs around /g2. Folding
    // the run under the cursor must be occurrence-local: the other
    // /g1 header keeps its own fold state.
    let mut sel = typed_selection(&["/g1/azb", "/g1/ab", "/g2/ab2"], "ab");
    let mut state = grouped_state(&[
        ("/g1/azb", "/g1"),
        ("/g1/ab", "/g1"),
        ("/g2/ab2", "/g2"),
    ]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    apply_query_folds(&mut state, &mut sel);
    sel.select_entry("/g1/azb");
    collapse_group(&mut state, "/g1", 1);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/g1/ab".to_string()),
            VisualTarget::Child("/g2/ab2".to_string()),
            VisualTarget::Header(0, 1),
        ]
    );
    assert_eq!(state.active_header, Some((0, 1)));
    let buf = render_list(&sel, &mut state, 40, 20);
    let rows: Vec<String> = (0..20).map(|y| row_text(&buf, y, 40)).collect();
    let g1: Vec<&str> = rows
        .iter()
        .filter(|r| r.contains("/g1"))
        .map(|r| r.trim_end())
        .collect();
    assert!(
        g1[0].starts_with("  \u{25be} /g1"),
        "first run open: {g1:?}"
    );
    assert!(
        g1.iter().any(|r| r.starts_with("  \u{25b8} /g1")),
        "second run folded: {g1:?}"
    );
    expand_group(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/g1/ab".to_string()),
            VisualTarget::Child("/g2/ab2".to_string()),
            VisualTarget::Child("/g1/azb".to_string()),
        ]
    );
    assert_eq!(sel.matches()[sel.selected_line() - 1].entry, "/g1/azb");
    sel.select_entry("/g1/ab");
    collapse_group(&mut state, "/g1", 0);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Header(0, 0),
            VisualTarget::Child("/g2/ab2".to_string()),
            VisualTarget::Child("/g1/azb".to_string()),
        ]
    );
}

#[test]
fn grouped_query_contx_keeps_personal_remainder_hit() {
    // Screenshot shape for query `contx`: ~/.config/contx, the
    // missing ~/personal/contx remainder hit, and ~/work/prefapp
    // remainder hits with nested linked worktrees under each main.
    // Production groups dir children under the expanded parent and
    // dir/* grandchildren under the intermediate parent.
    let config = "/Users/vieitesprefapp/.config/contx";
    let personal = "/Users/vieitesprefapp/personal/contx";
    let tfm = "/Users/vieitesprefapp/work/prefapp/tfm-specs-to-context";
    let pcm = "/Users/vieitesprefapp/work/prefapp/private-context-modeling";
    let tfm_nested = "/Users/vieitesprefapp/work/prefapp/tfm-specs-to-context/tfm-specs-to-context";
    let pcm_nested = "/Users/vieitesprefapp/work/prefapp/private-context-modeling/chore/context-modeling-migration";
    let config_g = "/Users/vieitesprefapp/.config";
    let personal_g = "/Users/vieitesprefapp/personal";
    let prefapp_g = "/Users/vieitesprefapp/work/prefapp";
    let mut sel = typed_selection(
        &[config, personal, tfm, pcm, tfm_nested, pcm_nested],
        "contx",
    );
    let mut state = grouped_state(&[
        (config, config_g),
        (personal, personal_g),
        (tfm, prefapp_g),
        (pcm, prefapp_g),
        (tfm_nested, prefapp_g),
        (pcm_nested, prefapp_g),
    ]);
    state.home = Some("/Users/vieitesprefapp".to_string());
    state.group_order = vec![
        config_g.to_string(),
        personal_g.to_string(),
        prefapp_g.to_string(),
    ];
    apply_query_folds(&mut state, &mut sel);
    let personal_child = VisualTarget::Child(personal.to_string());
    let before_git = visual_entries(&sel, &state);
    assert!(
        before_git.contains(&personal_child),
        "personal/contx missing from {before_git:?}",
    );
    state.git.apply(vec![
        grouped_git(tfm, false, None),
        grouped_git(pcm, false, None),
        grouped_git(tfm_nested, true, Some(tfm)),
        grouped_git(pcm_nested, true, Some(pcm)),
    ]);
    let after_git = visual_entries(&sel, &state);
    assert!(
        after_git.contains(&personal_child),
        "personal/contx dropped after nested git: {after_git:?}",
    );
}

#[test]
fn grouped_query_tucks_nested_worktree_off_rank_slot() {
    // Nested ranks after /g2 but still tucks under its primary
    // in /g1, skipping its own rank slot.
    let mut sel = typed_selection(&["/g1/ab", "/g2/ab2", "/g1/ab-wt"], "ab");
    assert_eq!(sel.matches()[0].entry, "/g1/ab");
    assert_eq!(sel.matches()[1].entry, "/g2/ab2");
    assert_eq!(sel.matches()[2].entry, "/g1/ab-wt");
    let mut state = grouped_state(&[
        ("/g1/ab", "/g1"),
        ("/g2/ab2", "/g2"),
        ("/g1/ab-wt", "/g1"),
    ]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    state.git.apply(vec![
        grouped_git("/g1/ab", false, None),
        grouped_git("/g2/ab2", false, None),
        grouped_git("/g1/ab-wt", true, Some("/g1/ab")),
    ]);
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/g1/ab".to_string()),
            VisualTarget::Child("/g1/ab-wt".to_string()),
            VisualTarget::Child("/g2/ab2".to_string()),
        ]
    );
}

#[test]
fn visual_entries_fall_back_to_match_order_without_group_order() {
    // Hand-built states without the resolved order keep the
    // old first-seen behavior instead of scrambling.
    let sel = typed_selection(&["/g1/xab", "/g2/ab"], "ab");
    let state = grouped_state(&[("/g1/xab", "/g1"), ("/g2/ab", "/g2")]);
    assert!(state.group_order.is_empty());
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/g2/ab".to_string()),
            VisualTarget::Child("/g1/xab".to_string()),
        ]
    );
}

#[test]
fn visual_entries_include_folded_headers_as_stops() {
    let sel = selection(&["/g1/a1", "/g1/a2", "/g2/b1"]);
    let mut state = grouped_state(&[
        ("/g1/a1", "/g1"),
        ("/g1/a2", "/g1"),
        ("/g2/b1", "/g2"),
    ]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    // Folded g1 is a header stop; unfolded g2 still walks children.
    state.folded.insert(("/g1".to_string(), 0));
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Header(0, 0),
            VisualTarget::Child("/g2/b1".to_string()),
        ]
    );
    state.folded.insert(("/g2".to_string(), 0));
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0), VisualTarget::Header(1, 0)]
    );
}

#[test]
fn visual_motion_lands_on_folded_headers() {
    let mut sel = selection(&["/g1/a1", "/g2/b1"]);
    let mut state = grouped_state(&[("/g1/a1", "/g1"), ("/g2/b1", "/g2")]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    state.folded.insert(("/g1".to_string(), 0));
    state.folded.insert(("/g2".to_string(), 0));
    // Home lands on the first folded header; selected_line stays.
    assert_eq!(
        apply_visual_motion(&mut sel, &mut state, VisualMotion::First),
        Some(VisualTarget::Header(0, 0)),
    );
    assert_eq!(state.active_header, Some((0, 0)));
    assert_eq!(sel.selected_line(), 1);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(state.active_header, Some((1, 0)));
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(state.active_header, Some((1, 0)));
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Prev);
    assert_eq!(state.active_header, Some((0, 0)));
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Last);
    assert_eq!(state.active_header, Some((1, 0)));
}

#[test]
fn visual_motion_from_child_onto_folded_header() {
    let mut sel = selection(&["/g1/a1", "/g1/a2", "/g2/b1"]);
    let mut state = grouped_state(&[
        ("/g1/a1", "/g1"),
        ("/g1/a2", "/g1"),
        ("/g2/b1", "/g2"),
    ]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    // g1 open, g2 folded: Child a1, Child a2, Header(1).
    state.folded.insert(("/g2".to_string(), 0));
    assert_eq!(sel.selected_line(), 1);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(sel.selected_line(), 2);
    assert_eq!(state.active_header, None);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(state.active_header, Some((1, 0)));
    assert_eq!(sel.selected_line(), 2);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Prev);
    assert_eq!(state.active_header, None);
    assert_eq!(sel.selected_line(), 2);
}

#[test]
fn visual_next_prev_walk_children_not_fuzzy_neighbors() {
    // "ab" ranks [/g1/ab, /g2/ab2, /g1/azb]. Remainder-hit
    // visual order follows that rank globally, repeating the
    // /g1 header around the interleaved /g2 run. Up/Down walk
    // that visual order, not clustered group membership.
    let mut sel = typed_selection(&["/g1/azb", "/g1/ab", "/g2/ab2"], "ab");
    assert_eq!(sel.matches()[0].entry, "/g1/ab");
    assert_eq!(sel.matches()[1].entry, "/g2/ab2");
    assert_eq!(sel.matches()[2].entry, "/g1/azb");
    let mut state = grouped_state(&[
        ("/g1/azb", "/g1"),
        ("/g1/ab", "/g1"),
        ("/g2/ab2", "/g2"),
    ]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/g1/ab".to_string()),
            VisualTarget::Child("/g2/ab2".to_string()),
            VisualTarget::Child("/g1/azb".to_string()),
        ]
    );
    // Next from the first visual child crosses into /g2.
    assert_eq!(sel.selected_line(), 1);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(sel.selected_line(), 2);
    let buf = render_list(&sel, &mut state, 40, 20);
    // Layout: header, child, git, blank, header, child, git.
    assert_eq!(buf[(0, 5)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(0, 6)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(0, 1)].bg, Theme::LIGHT.bg);
    // Next lands on the weaker /g1 remainder; the last child holds.
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(sel.selected_line(), 3);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(sel.selected_line(), 3);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Prev);
    assert_eq!(sel.selected_line(), 2);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::First);
    assert_eq!(sel.selected_line(), 1);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Last);
    assert_eq!(sel.selected_line(), 3);
    // Last tints the trailing /g1 run: blank + header + child + git.
    let buf = render_list(&sel, &mut state, 40, 20);
    assert_eq!(buf[(0, 9)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(0, 10)].bg, Theme::LIGHT.bg_alt);
}

#[test]
fn visual_motion_matches_rank_order_without_groups() {
    // Legacy flat list: visual order is match order, and an
    // empty list never moves.
    let mut sel = typed_selection(&["/x/azb", "/y/ab"], "ab");
    assert_eq!(sel.matches()[0].entry, "/y/ab");
    let mut state = SessionsListState::new(ThemeMode::Light);
    state.home = Some(FAR_HOME.to_string());
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(sel.selected_line(), 2);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(sel.selected_line(), 2);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Prev);
    assert_eq!(sel.selected_line(), 1);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Last);
    assert_eq!(sel.selected_line(), 2);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::First);
    assert_eq!(sel.selected_line(), 1);

    let mut sel = typed_selection(&["/a"], "zzz");
    assert!(sel.matches().is_empty());
    for motion in [
        VisualMotion::Prev,
        VisualMotion::Next,
        VisualMotion::First,
        VisualMotion::Last,
    ] {
        apply_visual_motion(&mut sel, &mut state, motion);
        assert_eq!(sel.selected_line(), 1);
    }
}

#[test]
fn grouped_tiny_areas_do_not_panic() {
    let sel = typed_selection(&["/parent/alpha", "/parent/beta"], "a");
    let mut state = grouped_state(&[
        ("/parent/alpha", "/parent"),
        ("/parent/beta", "/parent"),
    ]);
    state
        .git
        .apply(vec![grouped_git("/parent/alpha", false, None)]);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 10));
    for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (10, 2), (2, 10), (5, 5)] {
        sel.render(Rect::new(0, 0, w, h), &mut buf, &mut state);
    }
}

fn two_groups_state() -> (Selection, SessionsListState) {
    let sel = selection(&["/g1/a1", "/g1/a2", "/g2/b1"]);
    let mut state = grouped_state(&[
        ("/g1/a1", "/g1"),
        ("/g1/a2", "/g1"),
        ("/g2/b1", "/g2"),
    ]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    (sel, state)
}

#[test]
fn apply_query_folds_empty_query_folds_all_and_clears_header() {
    let (mut sel, mut state) = two_groups_state();
    state.active_header = Some((1, 0));
    apply_query_folds(&mut state, &mut sel);
    assert!(state.folded.contains(&("/g1".to_string(), 0)));
    assert!(state.folded.contains(&("/g2".to_string(), 0)));
    assert_eq!(state.active_header, None);
}

#[test]
fn apply_query_folds_nonempty_unfolds_matching_hides_empty() {
    let mut sel = selection(&["/g1/apple", "/g2/cherry", "/g3/apricot"]);
    let mut state = grouped_state(&[
        ("/g1/apple", "/g1"),
        ("/g2/cherry", "/g2"),
        ("/g3/apricot", "/g3"),
    ]);
    state.group_order =
        vec!["/g1".to_string(), "/g2".to_string(), "/g3".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert!(
        state.folded.contains(&("/g1".to_string(), 0))
            && state.folded.contains(&("/g2".to_string(), 0))
    );
    for c in "ap".chars() {
        sel.handle_key(key(KeyCode::Char(c)));
    }
    apply_query_folds(&mut state, &mut sel);
    assert!(
        !state.folded.contains(&("/g1".to_string(), 0)),
        "apple matches"
    );
    assert!(
        state.folded.contains(&("/g2".to_string(), 0)),
        "cherry misses"
    );
    assert!(
        !state.folded.contains(&("/g3".to_string(), 0)),
        "apricot matches"
    );
    assert_eq!(state.active_header, None);
}

#[test]
fn apply_query_folds_overrides_manual_expand_on_clear() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::First);
    expand_group(&mut state, &mut sel);
    assert!(!state.folded.contains(&("/g1".to_string(), 0)));
    sel.handle_key(key(KeyCode::Char('a')));
    apply_query_folds(&mut state, &mut sel);
    sel.handle_key(key(KeyCode::Backspace));
    apply_query_folds(&mut state, &mut sel);
    assert!(state.folded.contains(&("/g1".to_string(), 0)));
    assert!(state.folded.contains(&("/g2".to_string(), 0)));
}

#[test]
fn expand_group_unfolds_and_selects_first_child() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::First);
    assert_eq!(state.active_header, Some((0, 0)));
    expand_group(&mut state, &mut sel);
    assert!(!state.folded.contains(&("/g1".to_string(), 0)));
    assert_eq!(state.active_header, None);
    assert_eq!(sel.matches()[sel.selected_line() - 1].entry, "/g1/a1");
}

#[test]
fn expand_group_is_noop_without_active_header() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(state.active_header, None);
    expand_group(&mut state, &mut sel);
    assert!(state.folded.contains(&("/g1".to_string(), 0)));
}

#[test]
fn collapse_group_folds_and_lands_on_header() {
    let (sel, mut state) = two_groups_state();
    collapse_group(&mut state, "/g1", 0);
    assert!(state.folded.contains(&("/g1".to_string(), 0)));
    assert_eq!(state.active_header, Some((0, 0)));
    collapse_group(&mut state, "/g2", 0);
    assert_eq!(state.active_header, Some((1, 0)));
    assert_eq!(sel.selected_line(), 1);
}

#[test]
fn empty_query_renders_only_folded_headers() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    let rows: Vec<String> = (0..5).map(|y| row_text(&buf, y, 40)).collect();
    let all = rows.join("\n");
    assert!(rows[0].starts_with("  \u{25b8} /g1 (2)"), "{all}");
    assert!(rows[1].trim().is_empty(), "blank: {all}");
    assert!(rows[2].starts_with("  \u{25b8} /g2 (1)"), "{all}");
    assert!(!all.contains("a1") && !all.contains("a2") && !all.contains("b1"));
    assert_eq!(hit_count(&buf, 0, 40), 0, "empty header: {}", rows[0]);
    assert_eq!(hit_count(&buf, 2, 40), 0, "empty header: {}", rows[2]);
}

#[test]
fn typing_auto_unfolds_matching_and_hides_empty_groups() {
    let mut sel = selection(&["/g1/apple", "/g2/cherry", "/g3/apricot"]);
    let mut state = grouped_state(&[
        ("/g1/apple", "/g1"),
        ("/g2/cherry", "/g2"),
        ("/g3/apricot", "/g3"),
    ]);
    state.group_order =
        vec!["/g1".to_string(), "/g2".to_string(), "/g3".to_string()];
    apply_query_folds(&mut state, &mut sel);
    for c in "ap".chars() {
        sel.handle_key(key(KeyCode::Char(c)));
    }
    apply_query_folds(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    let all: String = (0..12)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all.contains("\u{25be} /g1"), "unfolded match: {all}");
    assert!(all.contains("apple"), "{all}");
    assert!(all.contains("\u{25be} /g3"), "{all}");
    assert!(all.contains("apricot"), "{all}");
    assert!(!all.contains("cherry") && !all.contains("/g2"), "{all}");
}

#[test]
fn clearing_query_refolds_all_groups() {
    let mut sel = selection(&["/g1/apple", "/g2/beta"]);
    let mut state = grouped_state(&[("/g1/apple", "/g1"), ("/g2/beta", "/g2")]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    for c in "ap".chars() {
        sel.handle_key(key(KeyCode::Char(c)));
    }
    apply_query_folds(&mut state, &mut sel);
    assert!(!state.folded.contains(&("/g1".to_string(), 0)));
    sel.handle_key(key(KeyCode::Backspace));
    sel.handle_key(key(KeyCode::Backspace));
    apply_query_folds(&mut state, &mut sel);
    assert!(
        state.folded.contains(&("/g1".to_string(), 0))
            && state.folded.contains(&("/g2".to_string(), 0))
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    let all: String = (0..5)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all.contains("\u{25b8} /g1"), "{all}");
    assert!(!all.contains("apple"), "{all}");
}

#[test]
fn enter_expands_folded_header_left_collapses_child() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::First);
    expand_group(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    let path = row_text(&buf, 1, 40);
    assert!(path.contains("a1"), "first child: {path}");
    assert_eq!(buf[(0, 1)].bg, Theme::LIGHT.bg_alt);
    collapse_group(&mut state, "/g1", 0);
    assert_eq!(state.active_header, Some((0, 0)));
    let buf = render_list(&sel, &mut state, 40, 20);
    let header = row_text(&buf, 0, 40);
    assert!(header.starts_with("  \u{25b8} /g1 (2)"), "{header}");
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg_alt);
    assert!(!row_text(&buf, 1, 40).contains("a1"));
}

#[test]
fn home_end_on_folded_list_tint_first_and_last_headers() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::First);
    let buf = render_list(&sel, &mut state, 40, 20);
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg_alt);
    assert_eq!(buf[(0, 2)].bg, Theme::LIGHT.bg);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Last);
    let buf = render_list(&sel, &mut state, 40, 20);
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg);
    assert_eq!(buf[(0, 2)].bg, Theme::LIGHT.bg_alt);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Prev);
    assert_eq!(state.active_header, Some((0, 0)));
}

#[test]
fn folded_groups_tiny_areas_do_not_panic() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 10));
    for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (10, 2), (2, 10), (5, 5)] {
        sel.render(Rect::new(0, 0, w, h), &mut buf, &mut state);
    }
}

#[test]
fn folded_list_blank_between_headers_no_frames() {
    let (mut sel, mut state) = two_groups_state();
    apply_query_folds(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    assert!(row_text(&buf, 1, 40).trim().is_empty());
    for y in 0..4 {
        let row = row_text(&buf, y, 40);
        assert!(
            !row.chars().any(|c| matches!(
                c,
                '\u{250c}'
                    | '\u{2510}'
                    | '\u{2518}'
                    | '\u{2500}'
                    | '\u{250f}'
                    | '\u{2513}'
                    | '\u{2517}'
                    | '\u{251b}'
                    | '\u{2501}'
                    | '\u{2503}'
            )),
            "no frames: {row}"
        );
        assert!(
            !row.contains('\u{251c}')
                && !row.contains('\u{2514}')
                && !row.contains('\u{2502}'),
            "no spine on folded: {row}"
        );
    }
}

/// Production leak: query "g" matches every ~/.config child
/// on the collapsed group prefix, so unhighlighted remainders
/// (uv, druk, fish, …) used to list under the unfolded group.
fn config_group_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("/config/uv", "/config"),
        ("/config/druk", "/config"),
        ("/config/github", "/config"),
        ("/config/fish", "/config"),
        ("/config/htop", "/config"),
        ("/config/hunk", "/config"),
    ]
}

#[test]
fn grouped_query_lists_only_remainder_matching_children() {
    let paths: Vec<&str> =
        config_group_entries().iter().map(|(p, _)| *p).collect();
    let mut sel = typed_selection(&paths, "g");
    // Full-path fuzzy still matches every child via "config".
    assert_eq!(sel.matches().len(), 6);
    let mut state = grouped_state(&config_group_entries());
    state.group_order = vec!["/config".to_string()];
    apply_query_folds(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    let all: String = (0..20)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all.contains("github"), "remainder match: {all}");
    for leak in ["uv", "druk", "fish", "htop", "hunk"] {
        assert!(!all.contains(leak), "non-match leaked {leak}: {all}");
    }
    let children: Vec<String> = (0..20)
        .map(|y| row_text(&buf, y, 40))
        .filter(|r| r.contains('\u{251c}') || r.contains('\u{2514}'))
        .collect();
    assert_eq!(children.len(), 1, "only matching children: {all}");
    assert!(children[0].starts_with("  \u{2514} github"), "{all}");
    assert_eq!(hit_spelling(&buf, 1, 40), "g");
    // `g` sits in the remainder, not `/config`; the unfolded
    // header stays unhighlighted and off Selection.
    assert_eq!(hit_spelling(&buf, 0, 40), "");
    assert!(row_text(&buf, 0, 40).starts_with("  \u{25be} /config"));
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg);
    assert_eq!(buf[(39, 0)].bg, Theme::LIGHT.bg);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Child("/config/github".to_string())]
    );
}

#[test]
fn grouped_empty_query_manual_expand_lists_all_children() {
    let paths: Vec<&str> =
        config_group_entries().iter().map(|(p, _)| *p).collect();
    let mut sel = selection(&paths);
    let mut state = grouped_state(&config_group_entries());
    state.group_order = vec!["/config".to_string()];
    apply_query_folds(&mut state, &mut sel);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::First);
    expand_group(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 20);
    let all: String = (0..20)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    for child in ["uv", "druk", "github", "fish", "htop", "hunk"] {
        assert!(all.contains(child), "missing {child}: {all}");
    }
    let children: Vec<String> = (0..20)
        .map(|y| row_text(&buf, y, 40))
        .filter(|r| r.contains('\u{251c}') || r.contains('\u{2514}'))
        .collect();
    assert_eq!(children.len(), 6, "all children: {all}");
    assert!(children[0].starts_with("  \u{251c} uv"), "{all}");
    assert!(children[5].starts_with("  \u{2514} hunk"), "{all}");
}

#[test]
fn grouped_empty_query_preserves_resolved_group_order() {
    // Candidate order saw g2 first; resolved group_order still
    // wins under an empty query.
    let mut sel = selection(&["/g2/ab", "/g1/xab"]);
    let mut state = grouped_state(&[("/g1/xab", "/g1"), ("/g2/ab", "/g2")]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0), VisualTarget::Header(1, 0)]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    assert!(row_text(&buf, 0, 40).starts_with("  \u{25b8} /g1"));
    assert!(row_text(&buf, 2, 40).starts_with("  \u{25b8} /g2"));
    assert_eq!(buf[(0, 0)].bg, Theme::LIGHT.bg);
    assert_eq!(buf[(0, 2)].bg, Theme::LIGHT.bg);
}

#[test]
fn grouped_query_filter_and_rank_tiny_areas_do_not_panic() {
    let paths: Vec<&str> =
        config_group_entries().iter().map(|(p, _)| *p).collect();
    let mut sel = typed_selection(&paths, "g");
    let mut state = grouped_state(&config_group_entries());
    state.group_order = vec!["/config".to_string()];
    apply_query_folds(&mut state, &mut sel);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 10));
    for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (10, 2), (2, 10), (5, 5)] {
        sel.render(Rect::new(0, 0, w, h), &mut buf, &mut state);
    }
    let mut sel = typed_selection(&["/g1/xab", "/g2/ab"], "ab");
    let mut state = grouped_state(&[("/g1/xab", "/g1"), ("/g2/ab", "/g2")]);
    state.group_order = vec!["/g1".to_string(), "/g2".to_string()];
    apply_query_folds(&mut state, &mut sel);
    for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (10, 2), (2, 10), (5, 5)] {
        sel.render(Rect::new(0, 0, w, h), &mut buf, &mut state);
    }
}

#[test]
fn grouped_prefix_only_query_shows_folded_header_with_count() {
    // `parent` hits only the collapsed group prefix, so both
    // children are prefix-only: folded header with (N), no
    // unhighlighted remainder dump.
    let mut sel = typed_selection(&["/parent/uv", "/parent/fish"], "parent");
    assert_eq!(sel.matches().len(), 2);
    let mut state = grouped_state(&[
        ("/parent/uv", "/parent"),
        ("/parent/fish", "/parent"),
    ]);
    state.group_order = vec!["/parent".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert!(state.folded.contains(&("/parent".to_string(), 0)));
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0)]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    let header = row_text(&buf, 0, 40);
    assert!(
        header.starts_with("  \u{25b8} /parent (2)"),
        "folded count: {header}"
    );
    assert!(!header.contains("(0)"), "{header}");
    assert_eq!(hit_spelling(&buf, 0, 40), "parent");
    let all: String = (0..8)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!all.contains("uv") && !all.contains("fish"), "{all}");
}

#[test]
fn grouped_query_remainder_groups_sit_above_prefix_only() {
    // `/ab/zzzz` ranks first (tighter span) but is prefix-only;
    // remainder-hit `/z/axb` still occupies tier 1 above it.
    let mut sel = typed_selection(&["/ab/zzzz", "/z/axb"], "ab");
    assert_eq!(sel.matches()[0].entry, "/ab/zzzz");
    assert_eq!(sel.matches()[1].entry, "/z/axb");
    let mut state = grouped_state(&[("/ab/zzzz", "/ab"), ("/z/axb", "/z")]);
    state.group_order = vec!["/ab".to_string(), "/z".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert!(
        !state.folded.contains(&("/z".to_string(), 0)),
        "remainder unfolds"
    );
    assert!(
        state.folded.contains(&("/ab".to_string(), 0)),
        "prefix-only stays folded"
    );
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/z/axb".to_string()),
            VisualTarget::Header(0, 0),
        ]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    let all: String = (0..12)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(row_text(&buf, 0, 40).starts_with("  \u{25be} /z"), "{all}");
    assert!(all.contains("axb"), "{all}");
    let prefix = (0..12)
        .map(|y| row_text(&buf, y, 40))
        .find(|r| r.contains("/ab"))
        .expect("prefix-only header");
    assert!(prefix.starts_with("  \u{25b8} /ab (1)"), "{prefix}");
    assert!(!all.contains("zzzz"), "prefix child hidden: {all}");
}

#[test]
fn grouped_empty_query_does_not_split_tiers() {
    // Same groups as the two-tier query fixture: empty query
    // still folds in resolved group_order, no remainder-first split.
    let mut sel = selection(&["/ab/zzzz", "/z/axb"]);
    let mut state = grouped_state(&[("/ab/zzzz", "/ab"), ("/z/axb", "/z")]);
    state.group_order = vec!["/ab".to_string(), "/z".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0), VisualTarget::Header(1, 0)]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    assert!(row_text(&buf, 0, 40).starts_with("  \u{25b8} /ab"));
    assert!(row_text(&buf, 2, 40).starts_with("  \u{25b8} /z"));
}

#[test]
fn grouped_pre_vieitesssclaims_stays_remainder_hit() {
    let entry = "/home/me/work/pre-vieitesss/claims";
    let mut sel = typed_selection(&[entry], "pre-vieitesssclaims");
    let mut state = grouped_state(&[(entry, "/home/me/work/pre-vieitesss")]);
    state.home = Some("/home/me".to_string());
    state.group_order = vec!["/home/me/work/pre-vieitesss".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert!(
        !state
            .folded
            .contains(&("/home/me/work/pre-vieitesss".to_string(), 0))
    );
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Child(entry.to_string())]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    let path = row_text(&buf, 1, 40);
    assert!(path.contains("claims"), "remainder: {path}");
    assert_eq!(hit_spelling(&buf, 1, 40), "claims");
}

#[test]
fn apply_query_folds_nonempty_focuses_first_remainder_child() {
    let mut sel = typed_selection(&["/ab/zzzz", "/z/axb"], "ab");
    let mut state = grouped_state(&[("/ab/zzzz", "/ab"), ("/z/axb", "/z")]);
    state.group_order = vec!["/ab".to_string(), "/z".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(state.active_header, None);
    assert_eq!(sel.matches()[sel.selected_line() - 1].entry, "/z/axb");
}

#[test]
fn apply_query_folds_nonempty_focuses_first_prefix_only_header() {
    let mut sel = typed_selection(&["/parent/uv", "/parent/fish"], "parent");
    let mut state = grouped_state(&[
        ("/parent/uv", "/parent"),
        ("/parent/fish", "/parent"),
    ]);
    state.group_order = vec!["/parent".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(state.active_header, Some((0, 0)));
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0)]
    );
}

#[test]
fn apply_query_folds_nomatch_leaves_empty_stops_and_empty_state() {
    let mut sel = typed_selection(&["/g1/a1"], "zzz");
    assert!(sel.matches().is_empty());
    let mut state = grouped_state(&[("/g1/a1", "/g1")]);
    state.group_order = vec!["/g1".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(state.active_header, None);
    assert!(visual_entries(&sel, &state).is_empty());
    let buf = render_list(&sel, &mut state, 40, 4);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("no matches for \"zzz\""), "empty: {row}");
}

#[test]
fn expand_prefix_only_lists_children_in_catalog_order() {
    // Fuzzy ranks apple first (lowercase); catalog saw zebra first.
    let mut sel =
        typed_selection(&["/parent/zebra", "/parent/apple"], "parent");
    assert_eq!(sel.matches()[0].entry, "/parent/apple");
    assert_eq!(sel.matches()[1].entry, "/parent/zebra");
    let mut state = grouped_state(&[
        ("/parent/zebra", "/parent"),
        ("/parent/apple", "/parent"),
    ]);
    state.group_order = vec!["/parent".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(state.active_header, Some((0, 0)));
    expand_group(&mut state, &mut sel);
    assert!(!state.folded.contains(&("/parent".to_string(), 0)));
    assert_eq!(state.active_header, None);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/parent/zebra".to_string()),
            VisualTarget::Child("/parent/apple".to_string()),
        ]
    );
    assert_eq!(
        sel.matches()[sel.selected_line() - 1].entry,
        "/parent/zebra"
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    let children: Vec<String> = (0..20)
        .map(|y| row_text(&buf, y, 40))
        .filter(|r| r.contains('\u{251c}') || r.contains('\u{2514}'))
        .collect();
    assert_eq!(children.len(), 2, "{children:?}");
    assert!(children[0].contains("zebra"), "{children:?}");
    assert!(children[1].contains("apple"), "{children:?}");
    assert_eq!(hit_count(&buf, 1, 40), 0, "no remainder paint");
    assert_eq!(hit_count(&buf, 3, 40), 0, "no remainder paint");
    collapse_group(&mut state, "/parent", 0);
    assert!(state.folded.contains(&("/parent".to_string(), 0)));
    assert_eq!(state.active_header, Some((0, 0)));
}

#[test]
fn expand_prefix_only_stays_below_remainder_hit_groups() {
    let mut sel = typed_selection(&["/ab/zzzz", "/z/axb"], "ab");
    let mut state = grouped_state(&[("/ab/zzzz", "/ab"), ("/z/axb", "/z")]);
    state.group_order = vec!["/ab".to_string(), "/z".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(sel.matches()[sel.selected_line() - 1].entry, "/z/axb");
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(state.active_header, Some((0, 0)));
    expand_group(&mut state, &mut sel);
    assert!(!state.folded.contains(&("/ab".to_string(), 0)));
    assert_eq!(
        visual_entries(&sel, &state),
        vec![
            VisualTarget::Child("/z/axb".to_string()),
            VisualTarget::Child("/ab/zzzz".to_string()),
        ]
    );
    let buf = render_list(&sel, &mut state, 40, 20);
    assert!(row_text(&buf, 0, 40).starts_with("  \u{25be} /z"));
    let ab = (0..20)
        .map(|y| row_text(&buf, y, 40))
        .find(|r| r.contains("/ab"))
        .expect("unfolded prefix-only still below");
    assert!(ab.starts_with("  \u{25be} /ab"), "{ab}");
    collapse_group(&mut state, "/ab", 0);
    assert_eq!(state.active_header, Some((0, 0)));
    assert!(state.folded.contains(&("/ab".to_string(), 0)));
}

#[test]
fn discovery_home_has_no_prefix_only_header() {
    // Query `users` matches `/Users/me/proj` only on the group
    // prefix. Discovery's `$HOME` bucket must not grow a
    // prefix-only header; matches stay nonempty so this is not
    // the empty-state copy. Skip is the discovery group key,
    // not `state.home`.
    let mut sel = typed_selection(&["/Users/me/proj"], "users");
    assert_eq!(sel.matches().len(), 1);
    let mut state = grouped_state(&[("/Users/me/proj", "/Users/me")]);
    state.home = Some("/Users/me".to_string());
    state.home_discovery_group = Some("/Users/me".to_string());
    state.group_order = vec!["/Users/me".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert!(visual_entries(&sel, &state).is_empty());
    let buf = render_list(&sel, &mut state, 40, 8);
    let all: String = (0..8)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!all.contains("no matches"), "{all}");
    assert!(
        !all.contains('\u{25b8}') && !all.contains('\u{25be}'),
        "{all}"
    );

    // Remainder-hit discovery children still unfold as tier 1.
    let mut sel = typed_selection(&["/Users/me/proj"], "proj");
    apply_query_folds(&mut state, &mut sel);
    assert!(!state.folded.contains(&("/Users/me".to_string(), 0)));
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Child("/Users/me/proj".to_string())]
    );

    // Empty query still shows the discovery `$HOME` header.
    let mut sel = selection(&["/Users/me/proj"]);
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0)]
    );
}

#[test]
fn configured_home_may_still_be_prefix_only() {
    // Same paths as discovery, but no discovery flag: a configured
    // `$HOME` group may appear as a prefix-only header.
    let mut sel = typed_selection(&["/Users/me/proj"], "users");
    let mut state = grouped_state(&[("/Users/me/proj", "/Users/me")]);
    state.home = Some("/Users/me".to_string());
    assert_eq!(state.home_discovery_group, None);
    state.group_order = vec!["/Users/me".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0)]
    );
    let buf = render_list(&sel, &mut state, 40, 8);
    let header = row_text(&buf, 0, 40);
    assert!(header.starts_with("  \u{25b8} ~ (1)"), "{header}");
}

#[test]
fn grouped_users_query_drops_collapsed_users_hits_on_header() {
    // Configured group under `/Users/...`: prefix-only folded
    // header, children hidden, collapsed `/Users` does not paint.
    let mut sel = typed_selection(&["/Users/me/work/proj"], "users");
    let mut state = grouped_state(&[("/Users/me/work/proj", "/Users/me/work")]);
    state.home = Some("/Users/me".to_string());
    state.group_order = vec!["/Users/me/work".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert_eq!(
        visual_entries(&sel, &state),
        vec![VisualTarget::Header(0, 0)]
    );
    let buf = render_list(&sel, &mut state, 40, 8);
    let header = row_text(&buf, 0, 40);
    assert!(header.starts_with("  \u{25b8} ~/work (1)"), "{header}");
    assert_eq!(hit_spelling(&buf, 0, 40), "", "users collapsed: {header}");
    let tilde = (0..40).find(|&x| buf[(x, 0)].symbol() == "~").unwrap();
    assert_ne!(buf[(tilde, 0)].fg, Theme::LIGHT.accent);
    let all: String = (0..8)
        .map(|y| row_text(&buf, y, 40))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!all.contains("proj"), "hidden until expand: {all}");
    expand_group(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 8);
    let path = row_text(&buf, 1, 40);
    assert!(path.contains("proj"), "remainder: {path}");
    assert_eq!(hit_count(&buf, 1, 40), 0, "no remainder paint: {path}");

    // Discovery `$HOME` still has no prefix-only header.
    let mut sel = typed_selection(&["/Users/me/proj"], "users");
    let mut state = grouped_state(&[("/Users/me/proj", "/Users/me")]);
    state.home = Some("/Users/me".to_string());
    state.home_discovery_group = Some("/Users/me".to_string());
    state.group_order = vec!["/Users/me".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert!(visual_entries(&sel, &state).is_empty());
}

#[test]
fn grouped_remainder_hit_paints_original_case() {
    let mut sel = typed_selection(&["/Users/me/work/GitHub"], "github");
    let mut state =
        grouped_state(&[("/Users/me/work/GitHub", "/Users/me/work")]);
    state.home = Some("/Users/me".to_string());
    state.group_order = vec!["/Users/me/work".to_string()];
    apply_query_folds(&mut state, &mut sel);
    let buf = render_list(&sel, &mut state, 40, 8);
    let path = row_text(&buf, 1, 40);
    assert!(path.contains("GitHub"), "original: {path}");
    assert!(!path.contains("github"), "{path}");
    assert_eq!(hit_spelling(&buf, 1, 40), "GitHub");
}

#[test]
fn tilde_query_does_not_match_home_abbreviation() {
    // `~` is display-only; the match surface is the real path.
    let mut sel = typed_selection(&["/Users/me/work/proj"], "~");
    assert!(sel.matches().is_empty());
    let mut state = grouped_state(&[("/Users/me/work/proj", "/Users/me/work")]);
    state.home = Some("/Users/me".to_string());
    state.group_order = vec!["/Users/me/work".to_string()];
    apply_query_folds(&mut state, &mut sel);
    assert!(visual_entries(&sel, &state).is_empty());
    let buf = render_list(&sel, &mut state, 40, 4);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("no matches for \"~\""), "empty: {row}");
}

fn tinted_folded_header_rows(buf: &Buffer, h: u16, w: u16) -> Vec<u16> {
    let bg_alt = Theme::LIGHT.bg_alt;
    (0..h)
        .filter(|&y| {
            row_text(buf, y, w).contains('\u{25b8}') && buf[(0, y)].bg == bg_alt
        })
        .collect()
}

#[test]
fn ctrl_j_walks_distinct_prefix_only_headers() {
    // Report: query `f`, first stop is remainder-hit `diffs.nvim`
    // under ~/opt; Ctrl-J (Next) tints several folded prefix-only
    // `~/.config`-style headers at once, and the next Next returns
    // to the starting child instead of walking later stops.
    let home = "/Users/vieitesprefapp";
    let diffs = format!("{home}/opt/diffs.nvim");
    let fuzzy = format!("{home}/opt/fuzzy.nvim");
    let config_nvim = format!("{home}/.config/nvim");
    let config_git = format!("{home}/.config/git");
    let g_opt = format!("{home}/opt");
    let g_config = format!("{home}/.config");
    let work_groups: Vec<(String, String)> = (0..8)
        .map(|i| {
            let group = format!("{home}/work/g{i}");
            let path = format!("{group}/notes");
            (path, group)
        })
        .collect();
    let mut paths: Vec<&str> = vec![&diffs, &fuzzy, &config_nvim, &config_git];
    paths.extend(work_groups.iter().map(|(p, _)| p.as_str()));
    let mut sel = typed_selection(&paths, "f");
    let mut pairs: Vec<(&str, &str)> = vec![
        (&diffs, &g_opt),
        (&fuzzy, &g_opt),
        (&config_nvim, &g_config),
        (&config_git, &g_config),
    ];
    pairs.extend(work_groups.iter().map(|(p, g)| (p.as_str(), g.as_str())));
    let mut state = grouped_state(&pairs);
    state.home = Some(home.to_string());
    let mut order = vec![g_config.clone(), g_opt.clone()];
    order.extend(work_groups.iter().map(|(_, g)| g.clone()));
    state.group_order = order;
    apply_query_folds(&mut state, &mut sel);

    let stops = visual_entries(&sel, &state);
    let start = match state.active_header {
        Some((idx, run)) => VisualTarget::Header(idx, run),
        None => VisualTarget::Child(
            sel.matches()[sel.selected_line() - 1].entry.clone(),
        ),
    };
    assert_eq!(stops.first(), Some(&start), "first stop: {stops:?}");
    assert!(
        matches!(start, VisualTarget::Child(ref p) if p.ends_with("diffs.nvim") || p.ends_with("fuzzy.nvim")),
        "first stop should be an opt remainder child: {start:?}"
    );
    let header_stops: Vec<_> = stops
        .iter()
        .filter(|s| matches!(s, VisualTarget::Header(_, _)))
        .collect();
    assert!(
        header_stops.len() >= 2,
        "need multiple prefix-only headers: {stops:?}"
    );
    assert!(
        stops
            .iter()
            .any(|s| matches!(s, VisualTarget::Child(p) if p == &diffs))
    );
    assert!(
        stops
            .iter()
            .any(|s| matches!(s, VisualTarget::Child(p) if p == &fuzzy))
    );

    let mut landed = Vec::new();
    let view_h = 12u16;
    for _ in 0..stops.len() + 2 {
        let buf = render_list(&sel, &mut state, 48, view_h);
        let tinted = tinted_folded_header_rows(&buf, view_h, 48);
        assert!(
            tinted.len() <= 1,
            "folded headers must not share tint: {tinted:?} scroll={} active={:?} line={}\n{}",
            state.scroll,
            state.active_header,
            sel.selected_line(),
            (0..view_h)
                .map(|y| row_text(&buf, y, 48))
                .collect::<Vec<_>>()
                .join("\n")
        );
        apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
        let now = match state.active_header {
            Some((idx, run)) => VisualTarget::Header(idx, run),
            None => VisualTarget::Child(
                sel.matches()[sel.selected_line() - 1].entry.clone(),
            ),
        };
        landed.push(now);
    }
    assert_eq!(
        landed[0], stops[1],
        "first Next must advance to the next stop, not stay or skip: {landed:?} stops={stops:?}"
    );
    assert_ne!(
        landed[1], start,
        "second Next must not return to the starting child: {landed:?}"
    );
    let unique_prefix: Vec<_> =
        landed.iter().take(stops.len() - 1).cloned().collect();
    assert_eq!(
        unique_prefix,
        stops[1..].to_vec(),
        "Next must walk each later stop once, no wrap: {landed:?} stops={stops:?}"
    );
    assert_eq!(
        landed[stops.len() - 1],
        *stops.last().unwrap(),
        "Next past the last stop must hold, not wrap: {landed:?}"
    );
}

fn focus_stop(sel: &Selection, state: &SessionsListState) -> VisualTarget {
    match state.active_header {
        Some((idx, run)) => VisualTarget::Header(idx, run),
        None => VisualTarget::Child(
            sel.matches()[sel.selected_line() - 1].entry.clone(),
        ),
    }
}

#[test]
fn ctrl_j_from_remainder_child_does_not_loop_across_prefix_only_headers() {
    // Frozen regression: query `f` ranks `/cfg/fa`, `/opt/diffs.nvim`,
    // `/cfg/fuzzy-long-name` so folded `/cfg` appears as two runs
    // around the remainder child. Each visible folded header must be
    // its own stop; Next walks that order with no wrap; at most one
    // folded header is tinted; diffs is not retinted until Prev.
    let diffs = "/opt/diffs.nvim";
    let cfg_a = "/cfg/fa";
    let cfg_b = "/cfg/fuzzy-long-name";
    let g_opt = "/opt";
    let g_config = "/cfg";
    let mut sel = typed_selection(&[cfg_a, diffs, cfg_b], "f");
    assert_eq!(
        sel.matches()
            .iter()
            .map(|m| m.entry.as_str())
            .collect::<Vec<_>>(),
        vec![cfg_a, diffs, cfg_b],
        "rank must interleave config runs around diffs"
    );
    let mut state =
        grouped_state(&[(cfg_a, g_config), (diffs, g_opt), (cfg_b, g_config)]);
    state.group_order = vec![g_config.to_string(), g_opt.to_string()];
    apply_query_folds(&mut state, &mut sel);
    state.folded.insert((g_config.to_string(), 0));
    state.folded.insert((g_config.to_string(), 1));
    state.active_header = None;
    sel.select_entry(diffs);
    assert_eq!(
        focus_stop(&sel, &state),
        VisualTarget::Child(diffs.to_string())
    );

    let stops = visual_entries(&sel, &state);
    let header_stops: Vec<_> = stops
        .iter()
        .filter(|s| matches!(s, VisualTarget::Header(_, _)))
        .cloned()
        .collect();
    assert!(
        header_stops.len() >= 2,
        "need repeated folded header runs: {stops:?}"
    );
    for (i, h) in header_stops.iter().enumerate() {
        assert!(
            !header_stops[..i].contains(h),
            "each folded header run must be its own stop: {stops:?}"
        );
    }
    let start = VisualTarget::Child(diffs.to_string());
    assert_eq!(stops.iter().filter(|s| *s == &start).count(), 1);
    let start_pos = stops.iter().position(|s| s == &start).unwrap();
    assert_eq!(focus_stop(&sel, &state), start);

    let (w, h) = (40u16, 8u16);
    let mut landed = Vec::new();
    for step in 0..stops.len() - start_pos + 1 {
        let buf = render_list(&sel, &mut state, w, h);
        let tinted = tinted_folded_header_rows(&buf, h, w);
        assert!(
            tinted.len() <= 1,
            "at most one folded header tint at step {step}: {tinted:?} focus={:?}\n{}",
            focus_stop(&sel, &state),
            (0..h)
                .map(|y| row_text(&buf, y, w))
                .collect::<Vec<_>>()
                .join("\n")
        );
        if step > 0 {
            assert_ne!(
                focus_stop(&sel, &state),
                start,
                "starting child must not be retinted until Prev: step={step} landed={landed:?}"
            );
        }
        apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
        landed.push(focus_stop(&sel, &state));
    }
    let suffix = &stops[start_pos + 1..];
    assert_eq!(
        &landed[..suffix.len()],
        suffix,
        "Next must walk each later stop: {landed:?} stops={stops:?}"
    );
    assert_eq!(
        landed[suffix.len() - 1],
        *stops.last().unwrap(),
        "Next past the last stop must hold, not wrap: {landed:?}"
    );
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Prev);
    assert_eq!(
        focus_stop(&sel, &state),
        start,
        "Prev from the later header run returns to diffs"
    );
}

/// H1 one-variable probe: distinct group keys → distinct Header idx.
/// Shared tint and 2-cycle must disappear.
#[test]
fn ctrl_j_distinct_header_indices_do_not_share_tint_or_loop() {
    let diffs = "/opt/diffs.nvim";
    let cfg_a = "/cfg-a/fa";
    let cfg_b = "/cfg-b/fuzzy-long-name";
    let mut sel = typed_selection(&[cfg_a, diffs, cfg_b], "f");
    assert_eq!(
        sel.matches()
            .iter()
            .map(|m| m.entry.as_str())
            .collect::<Vec<_>>(),
        vec![cfg_a, diffs, cfg_b]
    );
    let mut state =
        grouped_state(&[(cfg_a, "/cfg-a"), (diffs, "/opt"), (cfg_b, "/cfg-b")]);
    state.group_order = vec!["/cfg-a".to_string(), "/cfg-b".to_string()];
    state.folded.insert(("/cfg-a".to_string(), 0));
    state.folded.insert(("/cfg-b".to_string(), 0));
    sel.select_entry(diffs);

    let stops = visual_entries(&sel, &state);
    assert_eq!(
        stops,
        vec![
            VisualTarget::Header(0, 0),
            VisualTarget::Child(diffs.to_string()),
            VisualTarget::Header(1, 0),
        ]
    );
    let (w, h) = (40u16, 8u16);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(focus_stop(&sel, &state), VisualTarget::Header(1, 0));
    assert_eq!(state.active_header, Some((1, 0)));
    let buf = render_list(&sel, &mut state, w, h);
    assert_eq!(tinted_folded_header_rows(&buf, h, w).len(), 1);
    apply_visual_motion(&mut sel, &mut state, VisualMotion::Next);
    assert_eq!(focus_stop(&sel, &state), VisualTarget::Header(1, 0));
    assert_ne!(
        focus_stop(&sel, &state),
        VisualTarget::Child(diffs.to_string())
    );
}

#[test]
fn git_spans_failed_scan_keeps_branch_pr_and_failure_indicator() {
    let mut state = named_state(WorkState::Failed, "topic", Upstream::Absent);
    state.pull_request = Some(PullRequest {
        number: 17,
        state: PullRequestState::Open,
    });
    let text = spans_text(&state, 40);
    assert!(text.starts_with("\u{f467} topic"), "{text}");
    assert!(text.contains("#17 OPEN"), "{text}");
}

#[test]
fn git_spans_pending_shows_head_name_and_badge() {
    // Identity-only partial: the name stands alone, never a
    // fabricated change summary.
    let mut state = named_state(WorkState::Pending, "topic", Upstream::Absent);
    assert_eq!(spans_text(&state, 20), "\u{ec6f} topic");
    // A PR that lands before the scan still badges the row.
    state.pull_request = Some(PullRequest {
        number: 5,
        state: PullRequestState::Open,
    });
    assert!(spans_text(&state, 40).contains("#5 OPEN"));
}
