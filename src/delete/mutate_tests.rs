use super::{
    Confirm, DeleteOutcome, TrashOps, WorktreeOps, run_with,
    trash_confirm_prompt, worktree_confirm_prompt,
};
use crate::{
    config::{Command, Multiplexer, ResolvedConfig, SessionCandidate},
    delete::{DeleteError, DeleteRequest, DeleteStrategy},
    mux::PaneCwdOutcome,
    utils::test_utils::TempDir,
};
use std::{
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command as GitCmd,
};

fn cand(path: &Path) -> SessionCandidate {
    SessionCandidate::new(
        path.display().to_string(),
        path.parent().unwrap_or(path).display().to_string(),
    )
}

fn cfg(candidates: Vec<SessionCandidate>) -> ResolvedConfig {
    ResolvedConfig {
        candidates,
        multiplexer: Multiplexer::Auto,
        command: Command::Picker,
        permanent_delete: false,
        clone: crate::config::CloneSettings::default(),
        config_path: "/tmp/contx-test.toml".into(),
        paths: vec![],
        git_from_home: false,
        config_existed: true,
    }
}

fn req(
    path: &str,
    dry_run: bool,
    permanent: bool,
    force: bool,
) -> DeleteRequest {
    DeleteRequest {
        path: path.to_string(),
        dry_run,
        permanent,
        force,
    }
}

struct SkipFetch;

impl crate::delete::Fetcher for SkipFetch {
    fn fetch_all_prune(
        &mut self,
        _root: &str,
    ) -> crate::delete::preflight::FetchResult {
        crate::delete::preflight::FetchResult::Failed
    }
}

struct FakeTrash {
    fail: bool,
    calls: Vec<String>,
}

impl FakeTrash {
    fn ok() -> Self {
        Self {
            fail: false,
            calls: vec![],
        }
    }
    fn fail() -> Self {
        Self {
            fail: true,
            calls: vec![],
        }
    }
}

fn actually_delete(path: &str) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(path).map_err(|e| e.to_string())
    } else {
        fs::remove_dir_all(path).map_err(|e| e.to_string())
    }
}

impl TrashOps for FakeTrash {
    fn available(&self) -> bool {
        true
    }
    fn trash(&mut self, path: &str) -> Result<(), String> {
        self.calls.push(path.to_string());
        if self.fail {
            return Err("unavailable".to_string());
        }
        actually_delete(path)
    }
}

struct FakeWorktree {
    calls: Vec<String>,
    fail: bool,
}

impl WorktreeOps for FakeWorktree {
    fn remove(&mut self, path: &str) -> Result<(), String> {
        self.calls.push(path.to_string());
        if self.fail {
            return Err("dirty".to_string());
        }
        actually_delete(path)
    }
}

struct ScriptConfirm {
    interactive: bool,
    trash_yes: bool,
    permanent_yes: bool,
    trash_calls: usize,
    worktree_calls: usize,
    permanent_calls: usize,
}

impl ScriptConfirm {
    fn yes() -> Self {
        Self {
            interactive: true,
            trash_yes: true,
            permanent_yes: true,
            trash_calls: 0,
            worktree_calls: 0,
            permanent_calls: 0,
        }
    }
    fn no() -> Self {
        Self {
            interactive: true,
            trash_yes: false,
            permanent_yes: false,
            trash_calls: 0,
            worktree_calls: 0,
            permanent_calls: 0,
        }
    }
    fn noninteractive() -> Self {
        Self {
            interactive: false,
            trash_yes: true,
            permanent_yes: true,
            trash_calls: 0,
            worktree_calls: 0,
            permanent_calls: 0,
        }
    }
}

impl Confirm for ScriptConfirm {
    fn is_interactive(&self) -> bool {
        self.interactive
    }
    fn confirm_trash(&mut self, _path: &str) -> io::Result<bool> {
        self.trash_calls += 1;
        Ok(self.trash_yes)
    }
    fn confirm_worktree(&mut self, _path: &str) -> io::Result<bool> {
        self.worktree_calls += 1;
        Ok(self.trash_yes)
    }
    fn confirm_permanent(&mut self, _path: &str) -> io::Result<bool> {
        self.permanent_calls += 1;
        Ok(self.permanent_yes)
    }
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = GitCmd::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git spawn");
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).unwrap()
}

fn init_repo(dir: &Path) {
    git_in(dir, &["init", "-b", "topic"]);
    fs::write(dir.join("file.txt"), "one\n").unwrap();
    git_in(dir, &["add", "file.txt"]);
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
            "first",
        ],
    );
}

fn exec(
    config: &ResolvedConfig,
    request: &DeleteRequest,
    cwd: &Path,
    trash: &mut FakeTrash,
    worktree: &mut FakeWorktree,
    confirm: &mut ScriptConfirm,
) -> Result<DeleteOutcome, DeleteError> {
    let mut out = Vec::new();
    let mut err = Vec::new();
    run_with(
        config,
        request,
        cwd,
        cwd,
        &|_| None,
        PaneCwdOutcome::Skipped,
        &mut SkipFetch,
        trash,
        worktree,
        confirm,
        &mut out,
        &mut err,
    )
}

#[test]
fn dry_run_prints_report_and_does_not_mutate() {
    let d = TempDir::new();
    let dir = d.child("plain");
    fs::write(dir.join("f"), "x").unwrap();
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::yes();
    let outcome = exec(
        &config,
        &req(dir.to_str().unwrap(), true, false, false),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap();
    assert!(matches!(outcome, DeleteOutcome::DryRun(_)));
    assert!(dir.exists());
    assert!(trash.calls.is_empty());
    assert_eq!(confirm.trash_calls, 0);
}

#[test]
fn dry_run_writes_the_report_once() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::yes();
    let mut out = Vec::new();
    let mut err = Vec::new();
    run_with(
        &config,
        &req(dir.to_str().unwrap(), true, false, false),
        d.path(),
        d.path(),
        &|_| None,
        PaneCwdOutcome::Skipped,
        &mut SkipFetch,
        &mut trash,
        &mut wt,
        &mut confirm,
        &mut out,
        &mut err,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches("path:").count(), 1);
    assert_eq!(text.matches("strategy:").count(), 1);
    assert!(err.is_empty());
}

#[test]
fn trash_deletes_ordinary_directory() {
    let d = TempDir::new();
    let dir = d.child("plain");
    fs::write(dir.join("f"), "x").unwrap();
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::yes();
    let outcome = exec(
        &config,
        &req(dir.to_str().unwrap(), false, false, false),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap();
    match outcome {
        DeleteOutcome::Deleted {
            path,
            strategy: DeleteStrategy::Trash,
        } => assert!(path.ends_with("plain")),
        other => panic!("{other:?}"),
    }
    assert!(!dir.exists());
    assert_eq!(confirm.trash_calls, 1);
    assert_eq!(confirm.permanent_calls, 0);
}

#[test]
fn permanent_deletes_directory_and_symlink_link_only() {
    let d = TempDir::new();
    let target = d.child("real");
    fs::write(target.join("keep"), "x").unwrap();
    let link = d.path().join("link");
    symlink(&target, &link).unwrap();
    let config = cfg(vec![cand(&link)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::yes();
    exec(
        &config,
        &req(link.to_str().unwrap(), false, true, false),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap();
    assert!(!link.exists());
    assert!(target.join("keep").exists());
    assert_eq!(confirm.permanent_calls, 1);
    assert!(trash.calls.is_empty());
}

#[test]
fn force_skips_confirmation() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::no();
    exec(
        &config,
        &req(dir.to_str().unwrap(), false, false, true),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap();
    assert!(!dir.exists());
    assert_eq!(confirm.trash_calls, 0);
    assert_eq!(confirm.permanent_calls, 0);
}

#[test]
fn force_does_not_bypass_blockers() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::yes();
    let err = exec(
        &config,
        &req(dir.to_str().unwrap(), false, false, true),
        &dir,
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap_err();
    assert!(matches!(err, DeleteError::Blocked(_)));
    assert!(dir.exists());
}

#[test]
fn trash_fail_interactive_permanent_fallback() {
    let d = TempDir::new();
    let dir = d.child("plain");
    fs::write(dir.join("f"), "x").unwrap();
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::fail();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::yes();
    let outcome = exec(
        &config,
        &req(dir.to_str().unwrap(), false, false, false),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        DeleteOutcome::Deleted {
            strategy: DeleteStrategy::Permanent,
            ..
        }
    ));
    assert!(!dir.exists());
    assert_eq!(confirm.permanent_calls, 1);
}

#[test]
fn trash_fail_interactive_decline_leaves_path() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::fail();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm {
        interactive: true,
        trash_yes: true,
        permanent_yes: false,
        trash_calls: 0,
        worktree_calls: 0,
        permanent_calls: 0,
    };
    let err = exec(
        &config,
        &req(dir.to_str().unwrap(), false, false, false),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap_err();
    assert!(matches!(err, DeleteError::TrashFailed { .. }));
    assert!(dir.exists());
}

#[test]
fn trash_fail_noninteractive_force_is_not_fallback() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::fail();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::noninteractive();
    let err = exec(
        &config,
        &req(dir.to_str().unwrap(), false, false, true),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap_err();
    assert!(matches!(err, DeleteError::TrashFailed { .. }));
    assert!(dir.exists());
    assert_eq!(confirm.permanent_calls, 0);
}

#[test]
fn identity_race_blocks_before_mutate() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let path = dir.clone();
    // Replace the directory with a file after preflight, during confirm.
    let err = {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut confirm = RaceConfirm {
            inner: ScriptConfirm::yes(),
            path: path.clone(),
        };
        run_with(
            &config,
            &req(dir.to_str().unwrap(), false, false, false),
            d.path(),
            d.path(),
            &|_| None,
            PaneCwdOutcome::Skipped,
            &mut SkipFetch,
            &mut trash,
            &mut wt,
            &mut confirm,
            &mut out,
            &mut err,
        )
        .unwrap_err()
    };
    assert!(matches!(err, DeleteError::IdentityChanged(_)));
    assert!(path.exists());
    assert!(trash.calls.is_empty());
}

struct RaceConfirm {
    inner: ScriptConfirm,
    path: PathBuf,
}

impl Confirm for RaceConfirm {
    fn is_interactive(&self) -> bool {
        self.inner.is_interactive()
    }
    fn confirm_trash(&mut self, path: &str) -> io::Result<bool> {
        let ok = self.inner.confirm_trash(path)?;
        let _ = fs::remove_dir_all(&self.path);
        fs::write(&self.path, "now a file").unwrap();
        Ok(ok)
    }
    fn confirm_worktree(&mut self, path: &str) -> io::Result<bool> {
        self.inner.confirm_worktree(path)
    }
    fn confirm_permanent(&mut self, path: &str) -> io::Result<bool> {
        self.inner.confirm_permanent(path)
    }
}

struct LocalWorktree;

impl WorktreeOps for LocalWorktree {
    fn remove(&mut self, path: &str) -> Result<(), String> {
        let status = GitCmd::new("git")
            .arg("-C")
            .arg(path)
            .args(["worktree", "remove", path])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("git worktree remove failed".into())
        }
    }
}

#[test]
fn git_worktree_remove_without_force() {
    let d = TempDir::new();
    let main = d.child("main");
    init_repo(&main);
    git_in(&main, &["branch", "side"]);
    let linked = d.path().join("linked");
    git_in(
        &main,
        &["worktree", "add", linked.to_str().unwrap(), "side"],
    );
    let config = cfg(vec![cand(&linked)]);
    let mut trash = FakeTrash::ok();
    let mut confirm = ScriptConfirm::yes();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let outcome = run_with(
        &config,
        &req(linked.to_str().unwrap(), false, true, false),
        d.path(),
        d.path(),
        &|_| None,
        PaneCwdOutcome::Skipped,
        &mut SkipFetch,
        &mut trash,
        &mut LocalWorktree,
        &mut confirm,
        &mut out,
        &mut err,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        DeleteOutcome::Deleted {
            strategy: DeleteStrategy::GitWorktree,
            ..
        }
    ));
    assert!(!linked.exists());
    assert!(main.exists());
    assert!(trash.calls.is_empty());
    assert_eq!(confirm.trash_calls, 0);
    assert_eq!(confirm.worktree_calls, 1);
    let list = git_in(&main, &["worktree", "list"]);
    assert!(!list.contains("linked"));
}

#[test]
fn confirm_prompts_name_the_chosen_strategy() {
    let trash = trash_confirm_prompt("/tmp/proj");
    assert!(trash.contains("trash"));
    let worktree = worktree_confirm_prompt("/tmp/linked");
    assert!(worktree.contains("worktree"));
    assert!(
        !worktree.to_lowercase().contains("trash"),
        "worktree prompt must not mention trash: {worktree}"
    );
}

#[test]
fn noninteractive_without_force_requires_confirmation() {
    let d = TempDir::new();
    let dir = d.child("plain");
    let config = cfg(vec![cand(&dir)]);
    let mut trash = FakeTrash::ok();
    let mut wt = FakeWorktree {
        calls: vec![],
        fail: false,
    };
    let mut confirm = ScriptConfirm::noninteractive();
    let err = exec(
        &config,
        &req(dir.to_str().unwrap(), false, false, false),
        d.path(),
        &mut trash,
        &mut wt,
        &mut confirm,
    )
    .unwrap_err();
    assert!(matches!(err, DeleteError::ConfirmationRequired));
    assert!(dir.exists());
}
