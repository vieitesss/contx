use super::{
    Blocker, FetchResult, Fetcher, RemoteVerification, Warning, preflight,
};
use crate::{
    config::SessionCandidate,
    delete::{DeleteClass, DeleteRequest, DeleteStrategy},
    mux::PaneCwdOutcome,
    utils::test_utils::TempDir,
};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
};

fn cand(path: &Path) -> SessionCandidate {
    SessionCandidate::new(
        path.display().to_string(),
        path.parent().unwrap_or(path).display().to_string(),
    )
}

fn request(path: &str, dry_run: bool, permanent: bool) -> DeleteRequest {
    DeleteRequest {
        path: path.to_string(),
        dry_run,
        permanent,
        force: false,
    }
}

struct RecordFetch {
    called: Rc<Cell<bool>>,
    result: FetchResult,
}

impl Fetcher for RecordFetch {
    fn fetch_all_prune(&mut self, _root: &str) -> FetchResult {
        self.called.set(true);
        self.result
    }
}

fn no_fetch() -> RecordFetch {
    RecordFetch {
        called: Rc::new(Cell::new(false)),
        result: FetchResult::Failed,
    }
}

fn git_cmd(dir: Option<&Path>, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd
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
    String::from_utf8(out.stdout).expect("git output is not UTF-8")
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    git_cmd(Some(dir), args)
}

fn commit(dir: &Path, name: &str, content: &str, msg: &str) {
    fs::write(dir.join(name), content).unwrap();
    git_in(dir, &["add", name]);
    git_in(
        dir,
        &[
            "-c",
            "user.email=contx-test@example.com",
            "-c",
            "user.name=contx-test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            msg,
        ],
    );
}

fn init_repo(dir: &Path) {
    git_in(dir, &["init", "-b", "topic"]);
    commit(dir, "file.txt", "one\n", "first");
}

fn add_linked_worktree(main: &Path, linked: &Path) -> PathBuf {
    git_in(main, &["branch", "side"]);
    git_in(
        main,
        &[
            "worktree",
            "add",
            linked.to_str().expect("temp path is UTF-8"),
            "side",
        ],
    );
    linked.to_path_buf()
}

#[allow(clippy::too_many_arguments)]
fn run(
    path: &Path,
    candidates: &[SessionCandidate],
    process_cwd: &Path,
    panes: PaneCwdOutcome,
    dry_run: bool,
    permanent: bool,
    trash_available: bool,
    fetcher: &mut dyn Fetcher,
) -> super::Preflight {
    preflight(
        path.to_str().unwrap(),
        candidates,
        path.parent().unwrap_or(path),
        process_cwd,
        &|_| None,
        &request(path.to_str().unwrap(), dry_run, permanent),
        false,
        panes,
        trash_available,
        fetcher,
    )
    .unwrap()
}

#[test]
fn nonempty_ordinary_directory_warns() {
    let d = TempDir::new();
    let dir = d.child("plain");
    fs::write(dir.join("file"), "x").unwrap();
    let empty = d.child("empty");
    let mut fetch = no_fetch();
    let report = run(
        &dir,
        &[cand(&dir)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(report.warnings.contains(&Warning::NonemptyDirectory));
    let report = run(
        &empty,
        &[cand(&empty)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(!report.warnings.contains(&Warning::NonemptyDirectory));
}

#[test]
fn process_cwd_under_target_blocks_sibling_does_not() {
    let d = TempDir::new();
    let target = d.child("proj");
    let inside = d.child("proj/src");
    let mut fetch = no_fetch();
    let blocked = run(
        &target,
        &[cand(&target)],
        &inside,
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(blocked.blockers.contains(&Blocker::ActiveProcessCwd));

    let other = d.child("projx");
    let ok = run(
        &target,
        &[cand(&target)],
        &other,
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(!ok.blockers.contains(&Blocker::ActiveProcessCwd));
}

#[test]
fn pane_cwd_under_target_warns_and_list_failure_blocks() {
    let d = TempDir::new();
    let target = d.child("proj");
    let inside = d.child("proj/src");
    let mut fetch = no_fetch();
    let report = run(
        &target,
        &[cand(&target)],
        d.path(),
        PaneCwdOutcome::Listed(vec![inside.display().to_string()]),
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(!report.is_blocked(), "{report}");
    assert!(report.warnings.contains(&Warning::ActivePaneCwd));
    assert!(
        format!("{report}")
            .contains("pane working directory is inside the target")
    );

    let failed = run(
        &target,
        &[cand(&target)],
        d.path(),
        PaneCwdOutcome::Failed,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(failed.blockers.contains(&Blocker::PaneListFailed));
}

#[test]
fn not_a_candidate_is_a_blocker_not_an_error() {
    let d = TempDir::new();
    let path = d.child("nope");
    let mut fetch = no_fetch();
    let report = preflight(
        path.to_str().unwrap(),
        &[],
        d.path(),
        d.path(),
        &|_| None,
        &request(path.to_str().unwrap(), true, false),
        false,
        PaneCwdOutcome::Skipped,
        true,
        &mut fetch,
    )
    .unwrap();
    assert!(report.blockers.contains(&Blocker::NotCandidate));
    assert!(report.is_blocked());
    assert!(report.target.is_none());
    assert!(!fetch.called.get());
}

#[test]
fn dry_run_skips_fetch_and_reports_class_strategy() {
    let d = TempDir::new();
    let repo = d.child("repo");
    init_repo(&repo);
    let mut fetch = no_fetch();
    let report = run(
        &repo,
        &[cand(&repo)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(!fetch.called.get());
    assert_eq!(report.remote_verification, RemoteVerification::NotPerformed);
    let target = report.target.as_ref().unwrap();
    assert_eq!(target.class, DeleteClass::StandaloneRepo);
    assert_eq!(target.strategy, DeleteStrategy::Trash);
    let text = format!("{report}");
    assert!(text.contains("standalone repository"));
    assert!(text.contains("trash"));
    assert!(text.contains("not performed"));
    assert!(repo.exists());
}

#[test]
fn no_remotes_warns_and_skips_fetch() {
    let d = TempDir::new();
    let repo = d.child("repo");
    init_repo(&repo);
    let mut fetch = no_fetch();
    let report = run(
        &repo,
        &[cand(&repo)],
        d.path(),
        PaneCwdOutcome::Skipped,
        false,
        false,
        true,
        &mut fetch,
    );
    assert!(!fetch.called.get());
    assert!(report.warnings.contains(&Warning::NoRemotes));
    assert_eq!(report.remote_verification, RemoteVerification::NotPerformed);
}

#[test]
fn has_configured_remotes_follows_git_remote() {
    let d = TempDir::new();
    let repo = d.child("repo");
    init_repo(&repo);
    let path = repo.to_str().unwrap();
    assert!(!super::has_configured_remotes(path));
    git_in(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            d.child("origin").to_str().unwrap(),
        ],
    );
    assert!(super::has_configured_remotes(path));
}

#[test]
fn fetch_failure_is_a_hard_blocker() {
    let d = TempDir::new();
    let origin = d.child("origin");
    init_repo(&origin);
    let work = d.path().join("work");
    git_cmd(
        None,
        &["clone", origin.to_str().unwrap(), work.to_str().unwrap()],
    );
    let called = Rc::new(Cell::new(false));
    let mut fetch = RecordFetch {
        called: Rc::clone(&called),
        result: FetchResult::Failed,
    };
    let report = run(
        &work,
        &[cand(&work)],
        d.path(),
        PaneCwdOutcome::Skipped,
        false,
        false,
        true,
        &mut fetch,
    );
    assert!(called.get());
    assert!(report.blockers.contains(&Blocker::FetchFailed));
    assert_eq!(report.remote_verification, RemoteVerification::Failed);

    let mut cancelled = RecordFetch {
        called: Rc::new(Cell::new(false)),
        result: FetchResult::Cancelled,
    };
    let report = run(
        &work,
        &[cand(&work)],
        d.path(),
        PaneCwdOutcome::Skipped,
        false,
        false,
        true,
        &mut cancelled,
    );
    assert!(report.blockers.contains(&Blocker::FetchCancelled));

    let mut ok = RecordFetch {
        called: Rc::new(Cell::new(false)),
        result: FetchResult::Success,
    };
    let report = run(
        &work,
        &[cand(&work)],
        d.path(),
        PaneCwdOutcome::Skipped,
        false,
        false,
        true,
        &mut ok,
    );
    assert_eq!(report.remote_verification, RemoteVerification::Performed);
    assert!(!report.blockers.contains(&Blocker::FetchFailed));
}

#[test]
fn primary_with_linked_worktrees_is_blocked() {
    let d = TempDir::new();
    let main = d.child("main");
    init_repo(&main);
    add_linked_worktree(&main, &d.path().join("linked"));
    let mut fetch = no_fetch();
    let report = run(
        &main,
        &[cand(&main)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(
        report
            .blockers
            .contains(&Blocker::PrimaryHasLinkedWorktrees)
    );
}

#[test]
fn dirty_linked_worktree_is_refused() {
    let d = TempDir::new();
    let main = d.child("main");
    init_repo(&main);
    let linked = add_linked_worktree(&main, &d.path().join("linked"));
    fs::write(linked.join("dirt.txt"), "x").unwrap();
    let mut fetch = no_fetch();
    let report = run(
        &linked,
        &[cand(&linked)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| matches!(b, Blocker::GitWorktreeRefused { .. }))
    );
}

#[test]
fn nested_repository_is_identified_not_fetched() {
    let d = TempDir::new();
    let outer = d.child("outer");
    init_repo(&outer);
    let inner = d.child("outer/vendor/lib");
    init_repo(&inner);
    let mut fetch = no_fetch();
    let report = run(
        &outer,
        &[cand(&outer)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(!fetch.called.get());
    assert!(
        report.warnings.iter().any(|w| matches!(
            w,
            Warning::NestedRepositories { paths } if paths.iter().any(|p| p.ends_with("vendor/lib"))
        ))
    );
}

#[test]
fn nested_candidate_tracked_by_enclosing_repo_warns() {
    let d = TempDir::new();
    let repo = d.child("repo");
    init_repo(&repo);
    let nested = d.child("repo/src");
    fs::write(nested.join("tracked.txt"), "x").unwrap();
    git_in(&repo, &["add", "src/tracked.txt"]);
    git_in(
        &repo,
        &[
            "-c",
            "user.email=contx-test@example.com",
            "-c",
            "user.name=contx-test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "nested",
        ],
    );
    let mut fetch = no_fetch();
    let report = run(
        &nested,
        &[cand(&nested)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::TrackedByEnclosingRepo { .. }))
    );
    assert_eq!(
        report.target.as_ref().unwrap().class,
        DeleteClass::OrdinaryDirectory
    );
}

#[test]
fn trash_unavailable_blocks_trash_not_permanent() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let mut fetch = no_fetch();
    let trash = run(
        &dir,
        &[cand(&dir)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        false,
        &mut fetch,
    );
    assert!(trash.blockers.contains(&Blocker::TrashUnavailable));
    let permanent = run(
        &dir,
        &[cand(&dir)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        true,
        false,
        &mut fetch,
    );
    assert!(!permanent.blockers.contains(&Blocker::TrashUnavailable));
}

#[test]
fn git_status_and_stash_warnings() {
    let d = TempDir::new();
    let repo = d.child("repo");
    init_repo(&repo);
    fs::write(repo.join("file.txt"), "dirty\n").unwrap();
    fs::write(repo.join("extra.txt"), "u\n").unwrap();
    git_in(&repo, &["add", "file.txt"]);
    git_in(
        &repo,
        &[
            "-c",
            "user.email=contx-test@example.com",
            "-c",
            "user.name=contx-test",
            "-c",
            "commit.gpgsign=false",
            "stash",
            "push",
            "-m",
            "wip",
        ],
    );
    fs::write(repo.join("file.txt"), "unstaged\n").unwrap();
    fs::write(repo.join("ignored.bin"), "x").unwrap();
    fs::write(repo.join(".gitignore"), "ignored.bin\n").unwrap();
    let mut fetch = no_fetch();
    let report = run(
        &repo,
        &[cand(&repo)],
        d.path(),
        PaneCwdOutcome::Skipped,
        true,
        false,
        true,
        &mut fetch,
    );
    assert!(report.warnings.contains(&Warning::Unstaged));
    assert!(report.warnings.contains(&Warning::Untracked));
    assert!(report.warnings.contains(&Warning::Ignored));
    assert!(report.warnings.contains(&Warning::Stashes));
}

#[test]
fn path_at_or_under_does_not_prefix_match_siblings() {
    let d = TempDir::new();
    let foo = d.child("foo");
    let foobar = d.child("foobar");
    assert!(super::path_is_at_or_under(&foo, foo.to_str().unwrap()));
    assert!(!super::path_is_at_or_under(&foobar, foo.to_str().unwrap()));
}
