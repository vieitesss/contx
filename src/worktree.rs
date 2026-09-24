use std::{
    env, io,
    path::Path,
    process::{Command, Stdio},
};

use serde::Serialize;

use crate::{cli, clone, config::ResolvedConfig};

#[derive(Debug, Serialize)]
pub(crate) struct WorktreeOutcome {
    pub repo: String,
    pub branch: String,
    pub destination: String,
    pub new_branch: bool,
    pub config_updated: bool,
    pub discoverable: bool,
}

/// Create a linked Git worktree from a current repository candidate.
/// Neither a failed Git operation nor a failed config write deletes the path.
pub(crate) fn create(
    config: &ResolvedConfig,
    repo: &str,
    branch: &str,
    destination: &str,
    new_branch: bool,
    add_parent: bool,
) -> Result<WorktreeOutcome, String> {
    let repo = cli::candidate_path(config, repo)?;
    let root = git_output(repo, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim_end_matches(['\r', '\n']);
    let canonical_root =
        Path::new(root).canonicalize().map_err(|e| e.to_string())?;
    let canonical_repo =
        Path::new(repo).canonicalize().map_err(|e| e.to_string())?;
    if root.is_empty() || canonical_root != canonical_repo {
        return Err(format!("not a Git repository root: `{repo}`"));
    }
    let valid = Command::new("git")
        .args(["check-ref-format", "--branch", branch])
        .output()
        .map_err(|e| e.to_string())?;
    if !valid.status.success() {
        return Err(format!("invalid branch name: `{branch}`"));
    }
    let refname = format!("refs/heads/{branch}");
    let exists = Command::new("git")
        .args(["-C", repo, "show-ref", "--verify", "--quiet", &refname])
        .status()
        .map_err(|e| e.to_string())?;
    match (new_branch, exists.code()) {
        (true, Some(0)) => {
            return Err(format!("branch already exists: `{branch}`"));
        }
        (false, Some(1)) => {
            return Err(format!(
                "local branch does not exist: `{branch}`; pass --new-branch to create it"
            ));
        }
        (_, Some(0 | 1)) => {}
        _ => {
            return Err(format!(
                "could not inspect branch `{branch}` in `{repo}`"
            ));
        }
    }

    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let vars = |name: &str| env::var_os(name);
    let dest = clone::resolve_destination(destination, Some(&cwd), &vars)
        .map_err(|e| e.to_string())?;
    if clone::dest_exists(&dest) {
        return Err(format!("destination already exists: `{dest}`"));
    }
    let covered = config
        .destination_covered(&dest, &vars)
        .map_err(|e| e.to_string())?;
    eprintln!("worktree destination: {dest}");

    let mut command = Command::new("git");
    command.args(["-C", repo, "worktree", "add"]);
    if new_branch {
        command.args(["-b", branch]);
    }
    command.arg("--").arg(&dest);
    if !new_branch {
        command.arg(branch);
    }
    if config.json {
        command.stdout(Stdio::from(io::stderr()));
        command.env("GIT_TERMINAL_PROMPT", "0");
    }
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        let detail = status.code().map_or_else(
            || "interrupted".to_string(),
            |code| format!("exit {code}"),
        );
        return Err(format!(
            "git worktree add failed ({detail}); surviving destination: {dest}"
        ));
    }
    if add_parent && !covered {
        config.append_parent_to_paths(&dest, &vars).map_err(|e| {
            format!(
                "created worktree at `{dest}`, but failed to update config: {e}"
            )
        })?;
    }
    Ok(WorktreeOutcome {
        repo: repo.to_string(),
        branch: branch.to_string(),
        destination: dest,
        new_branch,
        config_updated: add_parent && !covered,
        discoverable: covered || add_parent,
    })
}

fn git_output(root: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("not a Git repository: `{root}`"));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
