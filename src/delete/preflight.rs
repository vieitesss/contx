use serde::Serialize;
use std::{
    ffi::OsString,
    fmt, fs, io,
    path::Path,
    process::{Command, Stdio},
};

use crate::config::{SessionCandidate, identity};
use crate::mux::PaneCwdOutcome;

use super::{
    DeleteClass, DeleteError, DeleteRequest, DeleteStrategy, DeleteTarget,
    classify, plan,
};

/// Overridable preflight findings. Acceptance never bypasses a blocker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Warning {
    NonemptyDirectory,
    Staged,
    Unstaged,
    Untracked,
    Ignored,
    Stashes,
    Ahead,
    NoUpstream,
    LocalOnlyBranches,
    LocalOnlyTags,
    LocalOnlyCommits,
    UnreachableDetachedHead,
    TrackedByEnclosingRepo { root: String },
    NestedRepositories { paths: Vec<String> },
    NoRemotes,
}

/// Hard blockers. `--force` cannot bypass these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    NotCandidate,
    Disappeared,
    IdentityChanged,
    ActiveProcessCwd,
    ActivePaneCwd,
    PaneListFailed,
    PrimaryHasLinkedWorktrees,
    FetchFailed,
    FetchCancelled,
    TrashUnavailable,
    GitWorktreeRefused { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteVerification {
    NotPerformed,
    Performed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preflight {
    pub path: String,
    pub target: Option<DeleteTarget>,
    pub warnings: Vec<Warning>,
    pub blockers: Vec<Blocker>,
    pub remote_verification: RemoteVerification,
}

impl Preflight {
    pub fn is_blocked(&self) -> bool {
        !self.blockers.is_empty()
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonemptyDirectory => write!(f, "nonempty directory"),
            Self::Staged => write!(f, "staged changes"),
            Self::Unstaged => write!(f, "unstaged changes"),
            Self::Untracked => write!(f, "untracked files"),
            Self::Ignored => write!(f, "ignored files"),
            Self::Stashes => write!(f, "stashes"),
            Self::Ahead => write!(f, "ahead of upstream"),
            Self::NoUpstream => write!(f, "no upstream"),
            Self::LocalOnlyBranches => write!(f, "local-only branches"),
            Self::LocalOnlyTags => write!(f, "local-only tags"),
            Self::LocalOnlyCommits => write!(f, "local-only commits"),
            Self::UnreachableDetachedHead => {
                write!(f, "unreachable detached HEAD")
            }
            Self::TrackedByEnclosingRepo { root } => {
                write!(f, "tracked by enclosing repository `{root}`")
            }
            Self::NestedRepositories { paths } => {
                write!(f, "nested repositories: {}", paths.join(", "))
            }
            Self::NoRemotes => write!(f, "no remotes"),
        }
    }
}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCandidate => write!(f, "not a session candidate"),
            Self::Disappeared => write!(f, "path disappeared"),
            Self::IdentityChanged => write!(f, "path identity or type changed"),
            Self::ActiveProcessCwd => {
                write!(f, "contx is running inside the target")
            }
            Self::ActivePaneCwd => {
                write!(f, "a visible pane is inside the target")
            }
            Self::PaneListFailed => {
                write!(f, "could not list multiplexer pane working directories")
            }
            Self::PrimaryHasLinkedWorktrees => {
                write!(f, "primary checkout has linked worktrees")
            }
            Self::FetchFailed => write!(f, "remote verification fetch failed"),
            Self::FetchCancelled => {
                write!(f, "remote verification fetch cancelled")
            }
            Self::TrashUnavailable => write!(f, "trash is unavailable"),
            Self::GitWorktreeRefused { reason } => {
                write!(f, "git worktree deletion refused ({reason})")
            }
        }
    }
}

impl fmt::Display for RemoteVerification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPerformed => write!(f, "not performed"),
            Self::Performed => write!(f, "performed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl fmt::Display for Preflight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "path: {}", self.path)?;
        match &self.target {
            Some(t) => {
                writeln!(f, "class: {}", class_name(t.class))?;
                writeln!(f, "strategy: {}", t.strategy)?;
            }
            None => writeln!(f, "class: unknown")?,
        }
        writeln!(f, "remote verification: {}", self.remote_verification)?;
        if !self.warnings.is_empty() {
            writeln!(f, "warnings:")?;
            for w in &self.warnings {
                writeln!(f, "  - {w}")?;
            }
        }
        if !self.blockers.is_empty() {
            writeln!(f, "blockers:")?;
            for b in &self.blockers {
                writeln!(f, "  - {b}")?;
            }
        }
        Ok(())
    }
}

fn class_name(class: DeleteClass) -> &'static str {
    match class {
        DeleteClass::Symlink => "symlink",
        DeleteClass::LinkedWorktree => "linked worktree",
        DeleteClass::StandaloneRepo => "standalone repository",
        DeleteClass::OrdinaryDirectory => "ordinary directory",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchResult {
    Success,
    Failed,
    Cancelled,
}

pub(crate) trait Fetcher {
    fn fetch_all_prune(&mut self, root: &str) -> FetchResult;
}

pub(crate) struct ProductionFetcher(pub bool);

impl Fetcher for ProductionFetcher {
    fn fetch_all_prune(&mut self, root: &str) -> FetchResult {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(root)
            .args(["fetch", "--all", "--prune"]);
        if self.0 {
            command.stdout(Stdio::from(io::stderr()));
            command.env("GIT_TERMINAL_PROMPT", "0");
        }
        match command.status() {
            Ok(s) if s.success() => FetchResult::Success,
            Ok(s) if s.code().is_none() => FetchResult::Cancelled,
            _ => FetchResult::Failed,
        }
    }
}

/// Whether `cwd` equals `target` or is a descendant, after canonicalize.
pub(crate) fn path_is_at_or_under(cwd: &Path, target: &str) -> bool {
    let Ok(cwd) = cwd.canonicalize() else {
        return false;
    };
    let Ok(target) = Path::new(target).canonicalize() else {
        return false;
    };
    cwd == target || cwd.starts_with(&target)
}

fn git(root: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn git_ok(root: &str, args: &[&str]) -> bool {
    git(root, args).is_some()
}

/// Inspect a proposed deletion. Read-only except standalone fetch when
/// `dry_run` is false and remotes exist. Never prompts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preflight(
    path: &str,
    candidates: &[SessionCandidate],
    cwd: &Path,
    process_cwd: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
    request: &DeleteRequest,
    config_permanent_delete: bool,
    panes: PaneCwdOutcome,
    trash_available: bool,
    fetcher: &mut dyn Fetcher,
) -> Result<Preflight, DeleteError> {
    let target = match plan(
        path,
        candidates,
        cwd,
        env,
        request,
        config_permanent_delete,
    ) {
        Ok(t) => t,
        Err(DeleteError::NotCandidate(p)) => {
            return Ok(Preflight {
                path: p,
                target: None,
                warnings: vec![],
                blockers: vec![Blocker::NotCandidate],
                remote_verification: RemoteVerification::NotPerformed,
            });
        }
        Err(DeleteError::NotFound(p)) => {
            return Ok(Preflight {
                path: p,
                target: None,
                warnings: vec![],
                blockers: vec![Blocker::Disappeared],
                remote_verification: RemoteVerification::NotPerformed,
            });
        }
        Err(e) => return Err(e),
    };

    let mut warnings = Vec::new();
    let mut blockers = Vec::new();
    let mut remote_verification = RemoteVerification::NotPerformed;

    if path_is_at_or_under(process_cwd, &target.path) {
        blockers.push(Blocker::ActiveProcessCwd);
    }
    match &panes {
        PaneCwdOutcome::Skipped => {}
        PaneCwdOutcome::Failed => blockers.push(Blocker::PaneListFailed),
        PaneCwdOutcome::Listed(cwds) => {
            if cwds
                .iter()
                .any(|c| path_is_at_or_under(Path::new(c), &target.path))
            {
                blockers.push(Blocker::ActivePaneCwd);
            }
        }
    }

    if target.strategy == DeleteStrategy::Trash && !trash_available {
        blockers.push(Blocker::TrashUnavailable);
    }

    match target.class {
        DeleteClass::Symlink => {}
        DeleteClass::OrdinaryDirectory => {
            inspect_ordinary(&target.path, &mut warnings);
        }
        DeleteClass::LinkedWorktree => {
            inspect_git(&target.path, &mut warnings);
            inspect_linked(&target.path, &mut blockers);
            inspect_nested(&target.path, &mut warnings);
        }
        DeleteClass::StandaloneRepo => {
            inspect_git(&target.path, &mut warnings);
            inspect_primary(&target.path, &mut blockers);
            inspect_nested(&target.path, &mut warnings);
            remote_verification = verify_remote(
                &target.path,
                request.dry_run,
                fetcher,
                &mut warnings,
                &mut blockers,
            );
        }
    }

    match classify(&target.path) {
        Ok(class) if class != target.class => {
            blockers.push(Blocker::IdentityChanged);
        }
        Err(DeleteError::NotFound(_)) => {
            blockers.push(Blocker::Disappeared);
        }
        _ => {}
    }

    Ok(Preflight {
        path: target.path.clone(),
        target: Some(target),
        warnings,
        blockers,
        remote_verification,
    })
}

fn inspect_ordinary(path: &str, warnings: &mut Vec<Warning>) {
    if dir_nonempty(path) {
        warnings.push(Warning::NonemptyDirectory);
    }
    if let Some(root) = enclosing_root(Path::new(path))
        && tracked_by(&root, path)
    {
        warnings.push(Warning::TrackedByEnclosingRepo {
            root: root.display().to_string(),
        });
    }
    inspect_nested(path, warnings);
}

fn dir_nonempty(path: &str) -> bool {
    fs::read_dir(path)
        .ok()
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

fn enclosing_root(path: &Path) -> Option<std::path::PathBuf> {
    let mut dir = path.parent()?;
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

fn tracked_by(root: &Path, path: &str) -> bool {
    let Ok(root_c) = root.canonicalize() else {
        return false;
    };
    let Ok(path_c) = Path::new(path).canonicalize() else {
        return false;
    };
    let Ok(rel) = path_c.strip_prefix(&root_c) else {
        return false;
    };
    let rel = rel.display().to_string();
    if rel.is_empty() {
        return false;
    }
    git(&root_c.display().to_string(), &["ls-files", "--", &rel])
        .is_some_and(|s| !s.trim().is_empty())
}

fn inspect_nested(path: &str, warnings: &mut Vec<Warning>) {
    let nested = nested_repos(Path::new(path));
    if !nested.is_empty() {
        warnings.push(Warning::NestedRepositories { paths: nested });
    }
}

fn nested_repos(base: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk_nested(base, base, &mut out);
    out
}

fn walk_nested(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        if entry.file_name() == ".git" {
            continue;
        }
        if path.join(".git").exists() && path != base {
            out.push(path.display().to_string());
            continue;
        }
        walk_nested(base, &path, out);
    }
}

fn inspect_git(root: &str, warnings: &mut Vec<Warning>) {
    let (staged, unstaged, untracked, ignored) = porcelain(root);
    if staged {
        warnings.push(Warning::Staged);
    }
    if unstaged {
        warnings.push(Warning::Unstaged);
    }
    if untracked {
        warnings.push(Warning::Untracked);
    }
    if ignored {
        warnings.push(Warning::Ignored);
    }
    if git(root, &["stash", "list"]).is_some_and(|s| !s.trim().is_empty()) {
        warnings.push(Warning::Stashes);
    }
    let abbrev = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string());
    if abbrev.as_deref() == Some("HEAD") {
        let branches =
            git(root, &["branch", "--contains", "HEAD"]).unwrap_or_default();
        let tags =
            git(root, &["tag", "--contains", "HEAD"]).unwrap_or_default();
        if branches.trim().is_empty() && tags.trim().is_empty() {
            warnings.push(Warning::UnreachableDetachedHead);
        }
    } else if !git_ok(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]) {
        warnings.push(Warning::NoUpstream);
    } else if let Some(counts) = git(
        root,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    ) {
        let ahead = counts
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        if ahead > 0 {
            warnings.push(Warning::Ahead);
        }
    }
    if local_only_branches(root) {
        warnings.push(Warning::LocalOnlyBranches);
    }
    if local_only_tags(root) {
        warnings.push(Warning::LocalOnlyTags);
    }
    if git(
        root,
        &["rev-list", "--branches", "--tags", "--not", "--remotes"],
    )
    .is_some_and(|s| !s.trim().is_empty())
    {
        warnings.push(Warning::LocalOnlyCommits);
    }
}

fn porcelain(root: &str) -> (bool, bool, bool, bool) {
    let Some(status) = git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    ) else {
        return (false, false, false, false);
    };
    let mut staged = false;
    let mut unstaged = false;
    let mut untracked = false;
    let mut ignored = false;
    for field in status.split('\0') {
        if field.len() < 3 {
            continue;
        }
        let bytes = field.as_bytes();
        if bytes[0] == b'!' && bytes[1] == b'!' {
            ignored = true;
            continue;
        }
        if bytes[0] == b'?' && bytes[1] == b'?' {
            untracked = true;
            continue;
        }
        if bytes[0] != b' ' {
            staged = true;
        }
        if bytes[1] != b' ' {
            unstaged = true;
        }
    }
    (staged, unstaged, untracked, ignored)
}

fn local_only_branches(root: &str) -> bool {
    let heads = git(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .unwrap_or_default();
    let remotes = git(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/remotes"],
    )
    .unwrap_or_default();
    let remote: Vec<&str> = remotes.split_whitespace().collect();
    heads.split_whitespace().any(|branch| {
        !remote
            .iter()
            .any(|r| r.ends_with(&format!("/{branch}")) || *r == branch)
    })
}

fn local_only_tags(root: &str) -> bool {
    git(root, &["rev-list", "--tags", "--not", "--remotes"])
        .is_some_and(|s| !s.trim().is_empty())
}

fn inspect_linked(root: &str, blockers: &mut Vec<Blocker>) {
    if worktrees(root)
        .into_iter()
        .any(|(path, locked)| locked && identity(&path) == identity(root))
    {
        blockers.push(Blocker::GitWorktreeRefused {
            reason: "locked".to_string(),
        });
        return;
    }
    let (staged, unstaged, untracked, _) = porcelain(root);
    if staged || unstaged || untracked {
        blockers.push(Blocker::GitWorktreeRefused {
            reason: "dirty".to_string(),
        });
    }
}

fn inspect_primary(root: &str, blockers: &mut Vec<Blocker>) {
    if worktrees(root).len() > 1 {
        blockers.push(Blocker::PrimaryHasLinkedWorktrees);
    }
}

fn worktrees(root: &str) -> Vec<(String, bool)> {
    let Some(out) = git(root, &["worktree", "list", "--porcelain"]) else {
        return vec![];
    };
    let mut list = Vec::new();
    let mut current: Option<String> = None;
    let mut locked = false;
    let flush = |list: &mut Vec<(String, bool)>,
                 current: &mut Option<String>,
                 locked: &mut bool| {
        if let Some(path) = current.take() {
            list.push((path, *locked));
            *locked = false;
        }
    };
    for line in out.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            flush(&mut list, &mut current, &mut locked);
            current = Some(path.to_string());
        } else if line.starts_with("locked") {
            locked = true;
        } else if line.is_empty() {
            flush(&mut list, &mut current, &mut locked);
        }
    }
    flush(&mut list, &mut current, &mut locked);
    list
}

pub(crate) fn has_configured_remotes(root: &str) -> bool {
    let remotes = git(root, &["remote"]).unwrap_or_default();
    remotes.split_whitespace().next().is_some()
}

fn verify_remote(
    root: &str,
    dry_run: bool,
    fetcher: &mut dyn Fetcher,
    warnings: &mut Vec<Warning>,
    blockers: &mut Vec<Blocker>,
) -> RemoteVerification {
    if dry_run {
        return RemoteVerification::NotPerformed;
    }
    if !has_configured_remotes(root) {
        warnings.push(Warning::NoRemotes);
        return RemoteVerification::NotPerformed;
    }
    match fetcher.fetch_all_prune(root) {
        FetchResult::Success => RemoteVerification::Performed,
        FetchResult::Failed => {
            blockers.push(Blocker::FetchFailed);
            RemoteVerification::Failed
        }
        FetchResult::Cancelled => {
            blockers.push(Blocker::FetchCancelled);
            RemoteVerification::Failed
        }
    }
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
