use super::Tui;
use crate::config::{Command, Multiplexer, ResolvedConfig, SessionCandidate};
use crate::tui::action::FakePty;
use crate::utils::test_utils::TempDir;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::Rect,
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
        permanent_delete: false,
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
