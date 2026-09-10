use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
    process::Command,
};

use crate::config::ResolvedConfig;
use crate::mux::PaneCwdOutcome;

use super::{
    DeleteClass, DeleteError, DeleteRequest, DeleteStrategy, DeleteTarget,
    Fetcher, Preflight, classify, match_candidate, preflight,
};

/// Result of a delete invocation that did not fail.
#[derive(Debug)]
pub enum DeleteOutcome {
    DryRun(#[allow(dead_code)] Preflight),
    Deleted {
        #[allow(dead_code)]
        path: String,
        #[allow(dead_code)]
        strategy: DeleteStrategy,
    },
    Cancelled,
}

pub(crate) trait TrashOps {
    fn available(&self) -> bool;
    fn trash(&mut self, path: &str) -> Result<(), String>;
}

pub(crate) trait WorktreeOps {
    fn remove(&mut self, path: &str) -> Result<(), String>;
}

pub(crate) fn trash_confirm_prompt(path: &str) -> String {
    format!("Move `{path}` to trash? [y/N]")
}

pub(crate) fn worktree_confirm_prompt(path: &str) -> String {
    format!("Delete git worktree `{path}`? [y/N]")
}

pub(crate) fn permanent_confirm_prompt(path: &str) -> String {
    format!("Permanently delete `{path}`. Type the exact path to confirm:")
}

pub(crate) trait Confirm {
    fn is_interactive(&self) -> bool;
    fn confirm_trash(&mut self, path: &str) -> io::Result<bool>;
    fn confirm_worktree(&mut self, path: &str) -> io::Result<bool>;
    fn confirm_permanent(&mut self, path: &str) -> io::Result<bool>;
}

struct ProductionTrash;

impl TrashOps for ProductionTrash {
    fn available(&self) -> bool {
        true
    }

    fn trash(&mut self, path: &str) -> Result<(), String> {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

struct ProductionWorktree;

impl WorktreeOps for ProductionWorktree {
    fn remove(&mut self, path: &str) -> Result<(), String> {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["worktree", "remove", path])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else if status.code().is_none() {
            Err("interrupted".to_string())
        } else {
            Err(format!("exit {}", status.code().unwrap_or(-1)))
        }
    }
}

struct ProductionConfirm;

impl Confirm for ProductionConfirm {
    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn confirm_trash(&mut self, path: &str) -> io::Result<bool> {
        let mut stderr = io::stderr();
        writeln!(stderr, "{}", trash_confirm_prompt(path))?;
        stderr.flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(matches!(line.trim(), "y" | "Y"))
    }

    fn confirm_worktree(&mut self, path: &str) -> io::Result<bool> {
        let mut stderr = io::stderr();
        writeln!(stderr, "{}", worktree_confirm_prompt(path))?;
        stderr.flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(matches!(line.trim(), "y" | "Y"))
    }

    fn confirm_permanent(&mut self, path: &str) -> io::Result<bool> {
        let mut stderr = io::stderr();
        writeln!(stderr, "{}", permanent_confirm_prompt(path))?;
        stderr.flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(line.trim() == path)
    }
}

pub fn run(
    config: &ResolvedConfig,
    request: &DeleteRequest,
) -> Result<DeleteOutcome, DeleteError> {
    let cwd = env::current_dir().map_err(DeleteError::Io)?;
    run_with(
        config,
        request,
        &cwd,
        &cwd,
        &|name| env::var_os(name),
        crate::mux::pane_cwds(config.multiplexer),
        &mut crate::delete::preflight::ProductionFetcher,
        &mut ProductionTrash,
        &mut ProductionWorktree,
        &mut ProductionConfirm,
        &mut io::stdout(),
        &mut io::stderr(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_with(
    config: &ResolvedConfig,
    request: &DeleteRequest,
    cwd: &Path,
    process_cwd: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
    panes: PaneCwdOutcome,
    fetcher: &mut dyn Fetcher,
    trash: &mut dyn TrashOps,
    worktree: &mut dyn WorktreeOps,
    confirm: &mut dyn Confirm,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<DeleteOutcome, DeleteError> {
    let report = preflight(
        &request.path,
        &config.candidates,
        cwd,
        process_cwd,
        env,
        request,
        config.permanent_delete,
        panes,
        trash.available(),
        fetcher,
    )?;

    if request.dry_run {
        write!(stdout, "{report}").map_err(DeleteError::Io)?;
        return Ok(DeleteOutcome::DryRun(report));
    }

    if report.is_blocked() {
        return Err(DeleteError::Blocked(report));
    }

    let target = report
        .target
        .clone()
        .ok_or_else(|| DeleteError::NotCandidate(report.path.clone()))?;

    write!(stderr, "{report}").map_err(DeleteError::Io)?;

    if !request.force {
        match confirm_strategy(confirm, &target)? {
            None => return Err(DeleteError::ConfirmationRequired),
            Some(false) => return Ok(DeleteOutcome::Cancelled),
            Some(true) => {}
        }
    }

    revalidate(&target, &config.candidates)?;
    let interactive = confirm.is_interactive();
    apply(
        &target,
        trash,
        worktree,
        confirm,
        request.force,
        interactive,
    )
}

fn confirm_strategy(
    confirm: &mut dyn Confirm,
    target: &DeleteTarget,
) -> Result<Option<bool>, DeleteError> {
    if !confirm.is_interactive() {
        return Ok(None);
    }
    let ok = match target.strategy {
        DeleteStrategy::Trash => confirm
            .confirm_trash(&target.path)
            .map_err(DeleteError::Io)?,
        DeleteStrategy::GitWorktree => confirm
            .confirm_worktree(&target.path)
            .map_err(DeleteError::Io)?,
        DeleteStrategy::Permanent => confirm
            .confirm_permanent(&target.path)
            .map_err(DeleteError::Io)?,
    };
    Ok(Some(ok))
}

pub(crate) fn revalidate(
    target: &DeleteTarget,
    candidates: &[crate::config::SessionCandidate],
) -> Result<(), DeleteError> {
    if match_candidate(&target.path, candidates).is_none() {
        return Err(DeleteError::NotCandidate(target.path.clone()));
    }
    match classify(&target.path) {
        Ok(class) if class == target.class => Ok(()),
        Ok(_) | Err(DeleteError::NotDirectory(_)) => {
            Err(DeleteError::IdentityChanged(target.path.clone()))
        }
        Err(e) => Err(e),
    }
}

fn apply(
    target: &DeleteTarget,
    trash: &mut dyn TrashOps,
    worktree: &mut dyn WorktreeOps,
    confirm: &mut dyn Confirm,
    force: bool,
    interactive: bool,
) -> Result<DeleteOutcome, DeleteError> {
    match target.strategy {
        DeleteStrategy::GitWorktree => {
            worktree.remove(&target.path).map_err(|detail| {
                DeleteError::GitWorktreeFailed {
                    path: target.path.clone(),
                    detail,
                }
            })?;
            Ok(DeleteOutcome::Deleted {
                path: target.path.clone(),
                strategy: DeleteStrategy::GitWorktree,
            })
        }
        DeleteStrategy::Permanent => {
            permanent_delete(&target.path, target.class)?;
            Ok(DeleteOutcome::Deleted {
                path: target.path.clone(),
                strategy: DeleteStrategy::Permanent,
            })
        }
        DeleteStrategy::Trash => match trash.trash(&target.path) {
            Ok(()) => Ok(DeleteOutcome::Deleted {
                path: target.path.clone(),
                strategy: DeleteStrategy::Trash,
            }),
            Err(cause) => {
                trash_failed(target, &cause, confirm, force, interactive)
            }
        },
    }
}

fn trash_failed(
    target: &DeleteTarget,
    cause: &str,
    confirm: &mut dyn Confirm,
    force: bool,
    interactive: bool,
) -> Result<DeleteOutcome, DeleteError> {
    let _ = force;
    if !interactive {
        return Err(DeleteError::TrashFailed {
            path: target.path.clone(),
            cause: cause.to_string(),
        });
    }
    let ok = confirm
        .confirm_permanent(&target.path)
        .map_err(DeleteError::Io)?;
    if !ok {
        return Err(DeleteError::TrashFailed {
            path: target.path.clone(),
            cause: cause.to_string(),
        });
    }
    revalidate_class(target)?;
    permanent_delete(&target.path, target.class)?;
    Ok(DeleteOutcome::Deleted {
        path: target.path.clone(),
        strategy: DeleteStrategy::Permanent,
    })
}

fn revalidate_class(target: &DeleteTarget) -> Result<(), DeleteError> {
    match classify(&target.path) {
        Ok(class) if class == target.class => Ok(()),
        Ok(_) | Err(DeleteError::NotDirectory(_)) => {
            Err(DeleteError::IdentityChanged(target.path.clone()))
        }
        Err(e) => Err(e),
    }
}

pub(crate) fn permanent_delete(
    path: &str,
    class: DeleteClass,
) -> Result<(), DeleteError> {
    let result = match class {
        DeleteClass::Symlink => fs::remove_file(path),
        DeleteClass::OrdinaryDirectory
        | DeleteClass::StandaloneRepo
        | DeleteClass::LinkedWorktree => fs::remove_dir_all(path),
    };
    result.map_err(|cause| DeleteError::PermanentFailed {
        path: path.to_string(),
        cause,
    })
}

#[cfg(test)]
#[path = "mutate_tests.rs"]
mod tests;
