use std::{
    env,
    ffi::OsString,
    fmt,
    io::{self, IsTerminal, Write},
    path::Path,
    process::Command,
};

use crate::config::{ConfigError, ResolvedConfig};

/// Result of a completed clone. Config may or may not have been updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneOutcome {
    pub dest: String,
    pub config_updated: bool,
}

/// Clone failure. A surviving destination is never deleted.
#[derive(Debug)]
pub enum CloneError {
    Config(ConfigError),
    NotAbsolute(String),
    DestExists(String),
    Io(io::Error),
    GitFailed {
        dest: String,
        code: Option<i32>,
    },
    GitInterrupted {
        dest: String,
    },
    /// Clone succeeded; writing the config path entry failed.
    Partial {
        dest: String,
        cause: ConfigError,
    },
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "{e}"),
            Self::NotAbsolute(path) => {
                write!(f, "not an absolute path: `{path}`")
            }
            Self::DestExists(path) => {
                write!(f, "destination already exists: `{path}`")
            }
            Self::Io(e) => write!(f, "{e}"),
            Self::GitFailed { dest, code } => match code {
                Some(code) => write!(
                    f,
                    "git clone failed (exit {code}); surviving destination: {dest}"
                ),
                None => {
                    write!(f, "git clone failed; surviving destination: {dest}")
                }
            },
            Self::GitInterrupted { dest } => write!(
                f,
                "git clone interrupted; surviving destination: {dest}"
            ),
            Self::Partial { dest, cause } => write!(
                f,
                "cloned to `{dest}`, but failed to update config: {cause}"
            ),
        }
    }
}

impl std::error::Error for CloneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Partial { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

/// Outcome of one `git clone` invocation, before mapping to `CloneError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitCloneStatus {
    Success,
    Failed { code: Option<i32> },
    Interrupted,
}

/// Injectable `git clone <source> <dest>` with inherited stdio in production.
pub(crate) trait GitClone {
    fn clone_repo(
        &mut self,
        source: &str,
        dest: &str,
    ) -> io::Result<GitCloneStatus>;
}

/// Interactive config-offer seam. Noninteractive clones never mutate config.
pub(crate) trait Interact {
    fn is_interactive(&self) -> bool;
    fn confirm_add_path(&mut self, parent: &str) -> io::Result<bool>;
}

struct ProductionGit;

impl GitClone for ProductionGit {
    fn clone_repo(
        &mut self,
        source: &str,
        dest: &str,
    ) -> io::Result<GitCloneStatus> {
        let status = Command::new("git")
            .arg("clone")
            .arg(source)
            .arg(dest)
            .status()?;
        if status.success() {
            Ok(GitCloneStatus::Success)
        } else if status.code().is_none() {
            Ok(GitCloneStatus::Interrupted)
        } else {
            Ok(GitCloneStatus::Failed {
                code: status.code(),
            })
        }
    }
}

struct ProductionInteract;

impl Interact for ProductionInteract {
    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn confirm_add_path(&mut self, parent: &str) -> io::Result<bool> {
        let mut stderr = io::stderr();
        writeln!(stderr, "Add `{parent}` to paths? [y/N]")?;
        stderr.flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(matches!(line.trim(), "y" | "Y"))
    }
}

/// Expand `~` and env vars. Relatives join `relative_base` when given
/// (CLI: caller CWD; picker: preselected group). Without a base,
/// a relative dest is rejected — never silently HOME/CWD.
pub(crate) fn resolve_destination(
    dest: &str,
    relative_base: Option<&Path>,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<String, CloneError> {
    if dest.is_empty() {
        return Err(CloneError::NotAbsolute(dest.to_string()));
    }
    let expanded =
        crate::config::expand(dest, env).map_err(CloneError::Config)?;
    let path = Path::new(&expanded);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(base) = relative_base {
        base.join(path)
    } else {
        return Err(CloneError::NotAbsolute(expanded));
    };
    let abs = trim_trailing_slashes(&joined.display().to_string()).to_string();
    if !abs.starts_with('/') {
        return Err(CloneError::NotAbsolute(abs));
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

pub(crate) fn dest_exists(path: &str) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Production clone: caller CWD, process env, inherited git stdio.
pub fn run(
    config: &ResolvedConfig,
    source: &str,
    destination: &str,
) -> Result<CloneOutcome, CloneError> {
    let cwd = env::current_dir().map_err(CloneError::Io)?;
    run_with(
        config,
        source,
        destination,
        &cwd,
        &|name| env::var_os(name),
        &mut ProductionGit,
        &mut ProductionInteract,
        &mut io::stderr(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_with(
    config: &ResolvedConfig,
    source: &str,
    destination: &str,
    cwd: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
    git: &mut dyn GitClone,
    interact: &mut dyn Interact,
    stderr: &mut dyn Write,
) -> Result<CloneOutcome, CloneError> {
    let abs = resolve_destination(destination, Some(cwd), env)?;
    if dest_exists(&abs) {
        return Err(CloneError::DestExists(abs));
    }
    writeln!(stderr, "clone destination: {abs}").map_err(CloneError::Io)?;

    let covered = config
        .destination_covered(&abs, env)
        .map_err(CloneError::Config)?;
    let mut want_write = false;
    if interact.is_interactive()
        && !covered
        && let Some(parent) = Path::new(&abs).parent()
        && !parent.as_os_str().is_empty()
    {
        want_write = interact
            .confirm_add_path(&parent.display().to_string())
            .map_err(CloneError::Io)?;
    }

    match git.clone_repo(source, &abs).map_err(CloneError::Io)? {
        GitCloneStatus::Success => {}
        GitCloneStatus::Failed { code } => {
            return Err(CloneError::GitFailed { dest: abs, code });
        }
        GitCloneStatus::Interrupted => {
            return Err(CloneError::GitInterrupted { dest: abs });
        }
    }

    if !want_write {
        return Ok(CloneOutcome {
            dest: abs,
            config_updated: false,
        });
    }
    match config.append_parent_to_paths(&abs, env) {
        Ok(()) => Ok(CloneOutcome {
            dest: abs,
            config_updated: true,
        }),
        Err(cause) => Err(CloneError::Partial { dest: abs, cause }),
    }
}

#[cfg(test)]
#[path = "clone_tests.rs"]
mod tests;
