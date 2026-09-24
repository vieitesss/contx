use serde::Serialize;
use std::{ffi::OsString, fmt, fs, io, path::Path};

mod mutate;
mod preflight;

use crate::config::{ConfigError, SessionCandidate, identity};

pub(crate) use mutate::{DeleteOutcome, permanent_delete, revalidate, run};
pub(crate) use preflight::{
    FetchResult, Fetcher, Preflight, has_configured_remotes, preflight,
};

/// Classification of the exact candidate path. Never follows a symlink
/// and never widens a nested candidate to an enclosing Git root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteClass {
    Symlink,
    LinkedWorktree,
    StandaloneRepo,
    OrdinaryDirectory,
}

/// How the exact path will be deleted. Linked worktrees always use Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteStrategy {
    Trash,
    Permanent,
    GitWorktree,
}

/// Planned deletion of one session candidate. No filesystem mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteTarget {
    /// Catalog spelling of the matched candidate, not a widened Git root.
    pub path: String,
    pub class: DeleteClass,
    pub strategy: DeleteStrategy,
}

/// CLI delete flags. `--force` never selects a strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRequest {
    pub path: String,
    pub dry_run: bool,
    pub permanent: bool,
    pub force: bool,
}

#[derive(Debug)]
pub enum DeleteError {
    Config(ConfigError),
    NotAbsolute(String),
    NotCandidate(String),
    NotFound(String),
    NotDirectory(String),
    IdentityChanged(String),
    ConfirmationRequired,
    Blocked(Preflight),
    TrashFailed { path: String, cause: String },
    GitWorktreeFailed { path: String, detail: String },
    PermanentFailed { path: String, cause: io::Error },
    Io(io::Error),
}

impl fmt::Display for DeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "{e}"),
            Self::NotAbsolute(path) => {
                write!(f, "not an absolute path: `{path}`")
            }
            Self::NotCandidate(path) => {
                write!(f, "not a session candidate: `{path}`")
            }
            Self::NotFound(path) => {
                write!(f, "path no longer exists: `{path}`")
            }
            Self::NotDirectory(path) => {
                write!(f, "not a directory: `{path}`")
            }
            Self::IdentityChanged(path) => {
                write!(f, "path identity or type changed: `{path}`")
            }
            Self::ConfirmationRequired => write!(
                f,
                "deletion requires confirmation; pass --force to skip"
            ),
            Self::Blocked(preflight) => write!(f, "{preflight}"),
            Self::TrashFailed { path, cause } => {
                write!(f, "trash failed for `{path}`: {cause}")
            }
            Self::GitWorktreeFailed { path, detail } => {
                write!(f, "git worktree deletion failed for `{path}`: {detail}")
            }
            Self::PermanentFailed { path, cause } => {
                write!(f, "permanent deletion failed for `{path}`: {cause}")
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DeleteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::PermanentFailed { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

impl fmt::Display for DeleteStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trash => write!(f, "trash"),
            Self::Permanent => write!(f, "permanent deletion"),
            Self::GitWorktree => write!(f, "git worktree deletion"),
        }
    }
}

/// Expand `~` and env vars, then resolve CLI relatives against `cwd`.
pub(crate) fn resolve_path(
    path: &str,
    cwd: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<String, DeleteError> {
    if path.is_empty() {
        return Err(DeleteError::NotAbsolute(path.to_string()));
    }
    let expanded =
        crate::config::expand(path, env).map_err(DeleteError::Config)?;
    let path = Path::new(&expanded);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let abs = trim_trailing_slashes(&joined.display().to_string()).to_string();
    if !abs.starts_with('/') {
        return Err(DeleteError::NotAbsolute(abs));
    }
    Ok(abs)
}

fn trim_trailing_slashes(path: &str) -> &str {
    if path == "/" {
        path
    } else {
        path.trim_end_matches('/')
    }
}

/// Match `path` to a current session candidate by canonical identity.
/// The catalog spelling is returned; the Git root is never substituted.
pub(crate) fn match_candidate<'a>(
    path: &str,
    candidates: &'a [SessionCandidate],
) -> Option<&'a SessionCandidate> {
    let want = identity(path);
    candidates.iter().find(|c| identity(&c.path) == want)
}

/// Classify the exact path with `lstat`. A symlink is never followed.
pub(crate) fn classify(path: &str) -> Result<DeleteClass, DeleteError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(DeleteError::NotFound(path.to_string()));
        }
        Err(e) => return Err(DeleteError::Io(e)),
    };
    if meta.file_type().is_symlink() {
        return Ok(DeleteClass::Symlink);
    }
    if !meta.is_dir() {
        return Err(DeleteError::NotDirectory(path.to_string()));
    }
    let git = Path::new(path).join(".git");
    match fs::symlink_metadata(&git) {
        Ok(git_meta) if git_meta.file_type().is_dir() => {
            Ok(DeleteClass::StandaloneRepo)
        }
        Ok(_) => Ok(DeleteClass::LinkedWorktree),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Ok(DeleteClass::OrdinaryDirectory)
        }
        Err(e) => Err(DeleteError::Io(e)),
    }
}

/// Strategy for a class. `--force` is ignored. Linked worktrees always
/// use non-force Git worktree deletion.
pub(crate) fn strategy(
    class: DeleteClass,
    permanent: bool,
    config_permanent_delete: bool,
) -> DeleteStrategy {
    match class {
        DeleteClass::LinkedWorktree => DeleteStrategy::GitWorktree,
        DeleteClass::Symlink
        | DeleteClass::StandaloneRepo
        | DeleteClass::OrdinaryDirectory => {
            if permanent || config_permanent_delete {
                DeleteStrategy::Permanent
            } else {
                DeleteStrategy::Trash
            }
        }
    }
}

/// Resolve, match, and classify without mutating anything.
pub(crate) fn plan(
    path: &str,
    candidates: &[SessionCandidate],
    cwd: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
    request: &DeleteRequest,
    config_permanent_delete: bool,
) -> Result<DeleteTarget, DeleteError> {
    let abs = resolve_path(path, cwd, env)?;
    let Some(candidate) = match_candidate(&abs, candidates) else {
        return Err(DeleteError::NotCandidate(abs));
    };
    let class = classify(&candidate.path)?;
    let strategy = strategy(class, request.permanent, config_permanent_delete);
    Ok(DeleteTarget {
        path: candidate.path.clone(),
        class,
        strategy,
    })
}

#[cfg(test)]
#[path = "delete_tests.rs"]
mod tests;
