use super::{Picker, visual_motion};
use crate::{
    config::{SessionCandidate, Startup, resolve_with},
    theme::Theme,
    tui::{
        git::{CandidateState, Head, Upstream, WorkState},
        selection::Intent,
        sessions_list::{VisualMotion, VisualTarget},
    },
    utils::test_utils::TempDir,
};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::Rect,
    widgets::StatefulWidget,
};
use std::ffi::OsString;
use std::fs;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn cand(path: &str, group: &str) -> SessionCandidate {
    SessionCandidate::new(path.to_string(), group.to_string())
}

fn two_groups() -> Picker {
    Picker::new(
        &[
            cand("/g1/a1", "/g1"),
            cand("/g1/a2", "/g1"),
            cand("/g2/b1", "/g2"),
        ],
        Theme::ANSI,
    )
}

fn render(picker: &mut Picker, w: u16, h: u16) -> Buffer {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    (&picker.selection).render(area, &mut buf, &mut picker.list);
    buf
}

fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
    (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

#[test]
fn arrows_and_home_end_are_visual_motions() {
    assert_eq!(visual_motion(&key(KeyCode::Up)), Some(VisualMotion::Prev));
    assert_eq!(visual_motion(&key(KeyCode::Down)), Some(VisualMotion::Next));
    assert_eq!(
        visual_motion(&key(KeyCode::Home)),
        Some(VisualMotion::First)
    );
    assert_eq!(visual_motion(&key(KeyCode::End)), Some(VisualMotion::Last));
}

#[test]
fn vim_aliases_are_visual_motions() {
    assert_eq!(
        visual_motion(&ctrl(KeyCode::Char('j'))),
        Some(VisualMotion::Next)
    );
    assert_eq!(
        visual_motion(&ctrl(KeyCode::Char('k'))),
        Some(VisualMotion::Prev)
    );
    assert_eq!(
        visual_motion(&ctrl(KeyCode::Char('t'))),
        Some(VisualMotion::First)
    );
    assert_eq!(
        visual_motion(&ctrl(KeyCode::Char('g'))),
        Some(VisualMotion::Last)
    );
    assert_eq!(
        visual_motion(&ctrl(KeyCode::Char('b'))),
        Some(VisualMotion::Last)
    );
}

#[test]
fn editing_exit_and_alt_chords_are_not_motions() {
    assert_eq!(visual_motion(&key(KeyCode::Char('a'))), None);
    assert_eq!(visual_motion(&key(KeyCode::Enter)), None);
    assert_eq!(visual_motion(&ctrl(KeyCode::Char('w'))), None);
    assert_eq!(visual_motion(&ctrl(KeyCode::Char('c'))), None);
    assert_eq!(
        visual_motion(&KeyEvent::new(KeyCode::Up, KeyModifiers::ALT)),
        None
    );
    assert_eq!(
        visual_motion(&KeyEvent::new_with_kind(
            KeyCode::Down,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )),
        None
    );
}

#[test]
fn empty_query_starts_folded_in_first_seen_group_order() {
    let picker = two_groups();
    assert_eq!(picker.query(), "");
    assert_eq!(
        picker.stops(),
        vec![VisualTarget::Header(0, 0), VisualTarget::Header(1, 0)]
    );
    assert_eq!(picker.focus(), Some(VisualTarget::Header(0, 0)));
    assert!(picker.list.folded.contains(&("/g1".to_string(), 0)));
    assert!(picker.list.folded.contains(&("/g2".to_string(), 0)));
}

#[test]
fn enter_right_space_expand_header_and_never_activate() {
    for code in [KeyCode::Enter, KeyCode::Right, KeyCode::Char(' ')] {
        let mut picker = two_groups();
        assert_eq!(picker.handle_key(key(code)), Intent::None);
        assert_eq!(picker.focus(), Some(VisualTarget::Child("/g1/a1".into())));
        assert!(!picker.list.folded.contains(&("/g1".to_string(), 0)));
        assert_eq!(
            picker.stops(),
            vec![
                VisualTarget::Child("/g1/a1".into()),
                VisualTarget::Child("/g1/a2".into()),
                VisualTarget::Header(1, 0),
            ]
        );
    }
}

#[test]
fn left_on_child_collapses_parent_run() {
    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Enter));
    assert_eq!(picker.handle_key(key(KeyCode::Left)), Intent::None);
    assert_eq!(picker.focus(), Some(VisualTarget::Header(0, 0)));
    assert!(picker.list.folded.contains(&("/g1".to_string(), 0)));
}

#[test]
fn enter_on_child_activates_and_header_or_empty_refuses() {
    let mut picker = two_groups();
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::None);
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter)),
        Intent::Activate("/g1/a1".into())
    );

    let mut empty = Picker::new(&[], Theme::ANSI);
    assert_eq!(empty.handle_key(key(KeyCode::Enter)), Intent::None);

    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Char('z')));
    picker.handle_key(key(KeyCode::Char('z')));
    assert!(picker.stops().is_empty());
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::None);
}

#[test]
fn query_unfolds_remainder_keeps_prefix_only_hides_empty_refolds_on_clear() {
    let mut picker = Picker::new(
        &[
            cand("/z/axb", "/z"),
            cand("/ab/zzzz", "/ab"),
            cand("/empty/nope", "/empty"),
        ],
        Theme::ANSI,
    );
    for c in "ab".chars() {
        picker.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(
        picker.stops(),
        vec![
            VisualTarget::Child("/z/axb".into()),
            VisualTarget::Header(1, 0),
        ]
    );
    assert!(!picker.list.folded.contains(&("/z".to_string(), 0)));
    assert!(picker.list.folded.contains(&("/ab".to_string(), 0)));
    assert!(!picker.stops().iter().any(|s| matches!(
        s,
        VisualTarget::Header(idx, _) if picker.list.group_order.get(*idx).map(String::as_str) == Some("/empty")
    )));
    picker.handle_key(key(KeyCode::Backspace));
    picker.handle_key(key(KeyCode::Backspace));
    assert_eq!(
        picker.stops(),
        vec![
            VisualTarget::Header(0, 0),
            VisualTarget::Header(1, 0),
            VisualTarget::Header(2, 0),
        ]
    );
    assert!(picker.list.folded.contains(&("/z".to_string(), 0)));
}

#[test]
fn motion_walks_visible_stops_without_wrapping() {
    let mut picker = two_groups();
    assert_eq!(picker.focus(), Some(VisualTarget::Header(0, 0)));
    picker.handle_key(key(KeyCode::Down));
    assert_eq!(picker.focus(), Some(VisualTarget::Header(1, 0)));
    picker.handle_key(ctrl(KeyCode::Char('j')));
    assert_eq!(picker.focus(), Some(VisualTarget::Header(1, 0)));
    picker.handle_key(key(KeyCode::Home));
    assert_eq!(picker.focus(), Some(VisualTarget::Header(0, 0)));
    picker.handle_key(key(KeyCode::Up));
    assert_eq!(picker.focus(), Some(VisualTarget::Header(0, 0)));
}

#[test]
fn home_discovery_groups_under_home_and_skips_prefix_only_header() {
    let mut home = cand("/Users/me/proj", "/Users/me");
    home.from_home_discovery = true;
    let mut picker = Picker::new(&[home], Theme::ANSI);
    picker.list.home = Some("/Users/me".to_string());
    assert_eq!(picker.stops(), vec![VisualTarget::Header(0, 0)]);
    picker.handle_key(key(KeyCode::Char('p')));
    picker.handle_key(key(KeyCode::Char('r')));
    picker.handle_key(key(KeyCode::Char('o')));
    picker.handle_key(key(KeyCode::Char('j')));
    assert_eq!(
        picker.stops(),
        vec![VisualTarget::Child("/Users/me/proj".into())]
    );
}

#[test]
fn ctrl_j_walks_repeated_run_headers_without_wrap() {
    let mut picker = Picker::new(
        &[
            cand("/g1/azb", "/g1"),
            cand("/g1/ab", "/g1"),
            cand("/g2/ab2", "/g2"),
        ],
        Theme::ANSI,
    );
    for c in "ab".chars() {
        picker.handle_key(key(KeyCode::Char(c)));
    }
    let start = picker.focus();
    let stops = picker.stops();
    assert!(stops.len() >= 2, "{stops:?}");
    picker.handle_key(ctrl(KeyCode::Char('j')));
    assert_eq!(picker.focus(), stops.get(1).cloned());
    for _ in 0..stops.len() + 2 {
        picker.handle_key(ctrl(KeyCode::Char('j')));
    }
    assert_eq!(picker.focus(), stops.last().cloned());
    assert_ne!(picker.focus(), start);
}

#[test]
fn empty_catalog_renders_no_candidates_copy() {
    let mut picker = Picker::new(&[], Theme::ANSI);
    let buf = render(&mut picker, 40, 4);
    let row = row_text(&buf, 0, 40);
    assert!(row.contains("no session candidates"), "{row}");
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::None);
}

fn git_state(
    path: &str,
    linked: bool,
    primary: Option<&str>,
    state: WorkState,
    head: Head,
) -> (String, CandidateState) {
    (
        path.to_string(),
        CandidateState {
            root: Some(path.to_string()),
            linked,
            primary: primary.map(str::to_string),
            state,
            head,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )
}

fn interleaved_g1() -> Picker {
    Picker::new(
        &[
            cand("/g1/azb", "/g1"),
            cand("/g1/ab", "/g1"),
            cand("/g2/ab2", "/g2"),
        ],
        Theme::ANSI,
    )
}

#[test]
fn identity_only_snapshot_does_not_move_focus_or_folds() {
    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Enter));
    let focus = picker.focus();
    let folded = picker.list.folded.clone();
    picker.apply_git(vec![git_state(
        "/g1/a1",
        false,
        None,
        WorkState::Measurable {
            added: 3,
            deleted: 1,
        },
        Head::Named("topic".into()),
    )]);
    assert_eq!(picker.focus(), focus);
    assert_eq!(picker.list.folded, folded);
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter)),
        Intent::Activate("/g1/a1".into())
    );
}

#[test]
fn omitted_snapshot_keeps_last_known_and_focus() {
    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Enter));
    picker.apply_git(vec![git_state(
        "/g1/a1",
        false,
        None,
        WorkState::Clean,
        Head::Named("topic".into()),
    )]);
    let focus = picker.focus();
    picker.apply_git(vec![git_state(
        "/g2/b1",
        false,
        None,
        WorkState::Marker,
        Head::Absent,
    )]);
    assert_eq!(picker.focus(), focus);
    assert_eq!(
        picker.list.git.get("/g1/a1").map(|s| s.head.clone()),
        Some(Head::Named("topic".into()))
    );
}

#[test]
fn late_linked_snapshot_keeps_child_that_is_still_a_stop() {
    let mut picker = interleaved_g1();
    for c in "ab".chars() {
        picker.handle_key(key(KeyCode::Char(c)));
    }
    picker.handle_key(key(KeyCode::End));
    assert_eq!(picker.focus(), Some(VisualTarget::Child("/g1/azb".into())));
    picker.apply_git(vec![
        git_state(
            "/g1/ab",
            false,
            None,
            WorkState::Clean,
            Head::Named("main".into()),
        ),
        git_state(
            "/g1/azb",
            true,
            Some("/g1/ab"),
            WorkState::Clean,
            Head::Named("side".into()),
        ),
    ]);
    assert_eq!(picker.focus(), Some(VisualTarget::Child("/g1/azb".into())));
    assert!(
        picker
            .stops()
            .contains(&VisualTarget::Child("/g1/azb".into()))
    );
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter)),
        Intent::Activate("/g1/azb".into())
    );
}

#[test]
fn late_linked_snapshot_reconciles_focused_run_header() {
    let mut picker = interleaved_g1();
    for c in "ab".chars() {
        picker.handle_key(key(KeyCode::Char(c)));
    }
    picker.handle_key(key(KeyCode::End));
    picker.handle_key(key(KeyCode::Left));
    assert_eq!(picker.focus(), Some(VisualTarget::Header(0, 1)));
    assert!(picker.list.folded.contains(&("/g1".to_string(), 1)));
    picker.apply_git(vec![
        git_state(
            "/g1/ab",
            false,
            None,
            WorkState::Clean,
            Head::Named("main".into()),
        ),
        git_state(
            "/g1/azb",
            true,
            Some("/g1/ab"),
            WorkState::Clean,
            Head::Named("side".into()),
        ),
    ]);
    assert!(!picker.list.folded.contains(&("/g1".to_string(), 1)));
    let focus = picker.focus();
    assert_ne!(focus, Some(VisualTarget::Header(0, 1)));
    assert_eq!(focus, picker.stops().first().cloned());
}

#[test]
fn git_snapshot_on_filter_miss_clears_header_keeps_selected_line() {
    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Char('z')));
    picker.handle_key(key(KeyCode::Char('z')));
    assert!(picker.stops().is_empty());
    let line = picker.selection.selected_line();
    picker.apply_git(vec![git_state(
        "/g1/a1",
        false,
        None,
        WorkState::Clean,
        Head::Named("topic".into()),
    )]);
    assert!(picker.stops().is_empty());
    assert_eq!(picker.list.active_header, None);
    assert_eq!(picker.selection.selected_line(), line);
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::None);
}

fn resolve_catalog(cfg: &str, home: &str) -> Vec<SessionCandidate> {
    let home = home.to_string();
    let args = vec!["-c".to_string(), cfg.to_string()];
    match resolve_with(&args, &|name| {
        (name == "HOME").then(|| OsString::from(home.as_str()))
    }) {
        Ok(Startup::Ready(resolved)) => resolved.candidates,
        other => panic!("expected ready catalog, got {other:?}"),
    }
}

#[test]
fn production_empty_catalog_from_missing_config() {
    let d = TempDir::new();
    let home = d.path().display().to_string();
    let args: Vec<String> = vec![];
    let startup = resolve_with(&args, &|name| {
        (name == "HOME").then(|| OsString::from(home.as_str()))
    })
    .unwrap();
    let Startup::Ready(resolved) = startup else {
        panic!("expected ready");
    };
    assert!(resolved.candidates.is_empty());
    let mut picker = Picker::new(&resolved.candidates, Theme::ANSI);
    assert!(picker.stops().is_empty());
    picker.handle_key(ctrl(KeyCode::Char('j')));
    assert!(picker.stops().is_empty());
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::None);
}

#[test]
fn production_catalog_scripted_query_motion_fold_and_activate() {
    let d = TempDir::new();
    let home = d.child("home");
    let work = d.child("home/work");
    d.child("home/work/alpha");
    d.child("home/work/beta");
    d.child("home/opt/gamma");
    let repo = d.child("home/repo");
    fs::create_dir(repo.join(".git")).unwrap();
    let cfg = d.file(
        "config.toml",
        "paths = [\"$HOME/work\", \"$HOME/opt\"]\ngit-from-home = true\n",
    );
    let candidates =
        resolve_catalog(cfg.to_str().unwrap(), &home.display().to_string());
    assert!(candidates.iter().any(|c| c.from_home_discovery));
    assert!(candidates.iter().any(|c| !c.from_home_discovery));
    let work_group = work.display().to_string();
    let alpha = format!("{}/alpha", work.display());
    assert!(
        candidates
            .iter()
            .any(|c| c.path == alpha && c.group == work_group)
    );

    let mut picker = Picker::new(&candidates, Theme::ANSI);
    let home_s = home.display().to_string();
    picker.list.home = Some(home_s.clone());
    assert_eq!(
        picker.list.home_discovery_group.as_deref(),
        Some(home_s.as_str())
    );
    assert!(
        picker
            .stops()
            .iter()
            .all(|s| matches!(s, VisualTarget::Header(_, 0)))
    );
    assert_eq!(picker.stops().len(), picker.list.group_order.len());
    assert!(picker.stops().len() >= 3);

    let start = picker.focus();
    let n = picker.stops().len();
    for _ in 0..n + 2 {
        picker.handle_key(ctrl(KeyCode::Char('j')));
    }
    assert_eq!(picker.focus(), picker.stops().last().cloned());
    picker.handle_key(key(KeyCode::Home));
    assert_eq!(picker.focus(), start);
    picker.handle_key(key(KeyCode::Up));
    assert_eq!(picker.focus(), start);

    let work_idx = picker
        .list
        .group_order
        .iter()
        .position(|g| g == &work_group)
        .expect("work group");
    picker.handle_key(key(KeyCode::Home));
    while picker.focus() != Some(VisualTarget::Header(work_idx, 0)) {
        let before = picker.focus();
        picker.handle_key(key(KeyCode::Down));
        assert_ne!(picker.focus(), before, "should reach work header");
    }
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::None);
    assert!(matches!(picker.focus(), Some(VisualTarget::Child(_))));
    assert_eq!(picker.handle_key(key(KeyCode::Left)), Intent::None);
    assert_eq!(picker.focus(), Some(VisualTarget::Header(work_idx, 0)));
    assert_eq!(picker.handle_key(key(KeyCode::Right)), Intent::None);
    picker.handle_key(key(KeyCode::Left));
    assert_eq!(picker.handle_key(key(KeyCode::Char(' '))), Intent::None);
    picker.handle_key(key(KeyCode::Left));

    for c in "alpha".chars() {
        picker.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(picker.focus(), Some(VisualTarget::Child(alpha.clone())));
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter)),
        Intent::Activate(alpha)
    );
}

#[test]
fn ctrl_x_opens_action_menu_escape_closes_without_editing_query() {
    let mut picker = two_groups();
    assert!(picker.menu().is_none());
    assert_eq!(picker.handle_key(ctrl(KeyCode::Char('x'))), Intent::None);
    let menu = picker.menu().expect("menu open");
    assert_eq!(menu.selected, None);
    assert!(!menu.delete_enabled);
    assert_eq!(picker.handle_key(key(KeyCode::Char('z'))), Intent::None);
    assert_eq!(picker.query(), "");
    assert_eq!(picker.handle_key(key(KeyCode::Esc)), Intent::None);
    assert!(picker.menu().is_none());
}

#[test]
fn action_menu_clone_is_global_delete_needs_a_child() {
    let mut picker = two_groups();
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.handle_key(key(KeyCode::Char('d'))), Intent::None);
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.handle_key(key(KeyCode::Char('c'))), Intent::Clone);
    assert!(picker.menu().is_none());

    picker.handle_key(key(KeyCode::Enter));
    assert!(matches!(picker.focus(), Some(VisualTarget::Child(_))));
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert!(picker.menu().unwrap().delete_enabled);
    assert_eq!(
        picker.handle_key(key(KeyCode::Char('d'))),
        Intent::Delete("/g1/a1".into())
    );
}

#[test]
fn action_menu_arrows_and_enter_select() {
    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Enter));
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.menu().unwrap().selected, None);
    assert_eq!(picker.handle_key(key(KeyCode::Up)), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(0));
    assert_eq!(picker.handle_key(key(KeyCode::Down)), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(1));
    assert_eq!(picker.handle_key(key(KeyCode::Down)), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(2));
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter)),
        Intent::Delete("/g1/a1".into())
    );
}

#[test]
fn action_menu_n_key_opens_new_directory() {
    let mut picker = two_groups();
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.handle_key(key(KeyCode::Char('n'))), Intent::NewDir);
    assert!(picker.menu().is_none());
}

#[test]
fn action_menu_enter_on_new_directory_returns_intent() {
    let mut picker = two_groups();
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.handle_key(key(KeyCode::Up)), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(0));
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::Clone);
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.handle_key(key(KeyCode::Down)), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(1));
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::NewDir);
}

#[test]
fn action_menu_k_and_j_navigate_like_up_and_down() {
    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Enter));
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.handle_key(key(KeyCode::Char('k'))), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(0));
    assert_eq!(picker.handle_key(key(KeyCode::Char('j'))), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(1));
    assert_eq!(picker.handle_key(key(KeyCode::Char('j'))), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(2));
    assert_eq!(picker.handle_key(key(KeyCode::Char('k'))), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(1));
    assert_eq!(picker.handle_key(key(KeyCode::Char('k'))), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(0));
}

#[test]
fn action_menu_enter_without_navigation_clones() {
    let mut picker = two_groups();
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert_eq!(picker.menu().unwrap().selected, None);
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::Clone);
    assert!(picker.menu().is_none());
}

#[test]
fn action_menu_down_without_delete_stops_at_new_directory() {
    let mut picker = two_groups();
    picker.handle_key(ctrl(KeyCode::Char('x')));
    assert!(!picker.menu().unwrap().delete_enabled);
    assert_eq!(picker.handle_key(key(KeyCode::Down)), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(1));
    assert_eq!(picker.handle_key(key(KeyCode::Char('j'))), Intent::None);
    assert_eq!(picker.menu().unwrap().selected, Some(1));
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), Intent::NewDir);
}

#[test]
fn clone_dest_parent_uses_group_or_stays_unset() {
    let mut picker = two_groups();
    assert_eq!(picker.clone_dest_parent().as_deref(), Some("/g1"));
    picker.handle_key(key(KeyCode::Enter));
    assert_eq!(picker.clone_dest_parent().as_deref(), Some("/g1"));
    let empty = Picker::new(&[], Theme::ANSI);
    assert_eq!(empty.clone_dest_parent(), None);
}

#[test]
fn refresh_after_delete_preserves_query_and_focuses_nearest() {
    let mut picker = two_groups();
    picker.handle_key(key(KeyCode::Enter));
    picker.handle_key(key(KeyCode::Down));
    assert_eq!(picker.focus(), Some(VisualTarget::Child("/g1/a2".into())));
    picker.refresh_after_delete(
        &[cand("/g1/a1", "/g1"), cand("/g2/b1", "/g2")],
        "/g1/a2",
    );
    assert_eq!(picker.query(), "");
    assert_eq!(picker.focus(), Some(VisualTarget::Child("/g1/a1".into())));
}

#[test]
fn refresh_after_clone_focuses_only_when_discovered_and_matching() {
    let mut picker = two_groups();
    for c in "a1".chars() {
        picker.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(picker.query(), "a1");
    let focused = picker.refresh_after_clone(
        &[
            cand("/g1/a1", "/g1"),
            cand("/g1/a2", "/g1"),
            cand("/g2/b1", "/g2"),
            cand("/g1/new", "/g1"),
        ],
        "/g1/new",
    );
    assert!(!focused);
    assert_eq!(picker.query(), "a1");

    let mut picker = two_groups();
    let focused = picker.refresh_after_clone(
        &[
            cand("/g1/a1", "/g1"),
            cand("/g1/a2", "/g1"),
            cand("/g2/b1", "/g2"),
            cand("/g1/cloned", "/g1"),
        ],
        "/g1/cloned",
    );
    assert!(focused);
    assert_eq!(
        picker.focus(),
        Some(VisualTarget::Child("/g1/cloned".into()))
    );
}
