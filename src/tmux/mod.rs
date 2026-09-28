pub mod errors;

use std::{
    env,
    ffi::OsString,
    io::{self, ErrorKind::InvalidData},
    process::Command,
};

use errors::{ActivationError, TmuxError};

/// Deliberate success outcome of activating a session candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Activation {
    /// The tmux session name derived from the candidate and attached to.
    pub session: String,
}

/// Narrow internal seam at the tmux command boundary: execute one tmux
/// argv. Production runs the external program; tests script results.
trait CommandRunner {
    fn run(&mut self, args: &[&str]) -> io::Result<RawOutput>;
}

/// One tmux process invocation, before classification.
#[derive(Debug, Clone, PartialEq)]
struct RawOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct ProductionRunner;

impl CommandRunner for ProductionRunner {
    fn run(&mut self, args: &[&str]) -> io::Result<RawOutput> {
        let output = Command::new("tmux").args(args).output()?;
        Ok(RawOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Classified result of one tmux invocation.
#[derive(Debug)]
enum CommandResult {
    Success,
    SessionAbsent { detail: String },
    Failed(TmuxError),
}

/// Interpret one invocation. Launch failures, unexpected exits, and invalid
/// output are failures; an exit that authoritatively reports no such
/// session is absence.
fn classify(result: io::Result<RawOutput>) -> CommandResult {
    let output = match result {
        Ok(output) => output,
        Err(e) => return CommandResult::Failed(TmuxError::IoError(e)),
    };
    if output.success {
        if String::from_utf8(output.stdout).is_err() {
            return CommandResult::Failed(TmuxError::IoError(io::Error::new(
                InvalidData,
                "tmux printed invalid UTF-8",
            )));
        }
        return CommandResult::Success;
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if is_session_absent(&stderr) {
        return CommandResult::SessionAbsent { detail: stderr };
    }
    CommandResult::Failed(TmuxError::CommandFailed(io::Error::other(stderr)))
}

/// Whether stderr authoritatively reports no such session, covering tmux's
/// historical wordings.
fn is_session_absent(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("can't find session")
        || lower.contains("couldn't find session")
        || lower.contains("session not found")
}

fn is_tmux_process(env: &dyn Fn(&str) -> Option<OsString>) -> bool {
    matches!(env("TMUX"), Some(value) if !value.is_empty())
}

fn has_session(runner: &mut dyn CommandRunner, session: &str) -> CommandResult {
    let target = format!("={session}");
    classify(runner.run(&["has-session", "-t", &target]))
}

fn new_session(
    runner: &mut dyn CommandRunner,
    session: &str,
    path: &str,
) -> CommandResult {
    classify(runner.run(&["new-session", "-ds", session, "-c", path]))
}

fn switch_client(
    runner: &mut dyn CommandRunner,
    session: &str,
) -> CommandResult {
    let target = format!("={session}");
    classify(runner.run(&["switch-client", "-t", &target]))
}

/// Activate the tmux session for the selected session candidate, deriving
/// the session name from the candidate path. An existing session is checked
/// then switched; an initially absent session is checked, created at the
/// candidate path, then switched. Any failure stops the sequence with its
/// meaning preserved for the caller.
pub fn open(candidate: &str) -> Result<Activation, ActivationError> {
    open_with(candidate, &mut ProductionRunner, &|name| env::var_os(name))
}

/// Activation policy over an injected command runner and environment, so
/// tests can script command sequences deterministically.
fn open_with(
    candidate: &str,
    runner: &mut dyn CommandRunner,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Activation, ActivationError> {
    if !is_tmux_process(env) {
        return Err(ActivationError::NotInTmux);
    }

    let session = crate::label::project_target_label(candidate, env)
        .map_err(|e| ActivationError::InvalidCandidate(e.0))?;

    match has_session(runner, &session) {
        CommandResult::Failed(cause) => {
            Err(ActivationError::CheckFailed { session, cause })
        }
        CommandResult::SessionAbsent { .. } => {
            create_then_switch(runner, candidate, session)
        }
        CommandResult::Success => switch(runner, session),
    }
}

fn create_then_switch(
    runner: &mut dyn CommandRunner,
    candidate: &str,
    session: String,
) -> Result<Activation, ActivationError> {
    match new_session(runner, &session, candidate) {
        CommandResult::Success => switch(runner, session),
        CommandResult::SessionAbsent { detail } => {
            Err(ActivationError::CreateFailed {
                session,
                cause: TmuxError::CommandFailed(io::Error::other(detail)),
            })
        }
        CommandResult::Failed(cause) => {
            Err(ActivationError::CreateFailed { session, cause })
        }
    }
}

/// List every pane's current path (`list-panes -a -F #{pane_current_path}`).
pub(crate) fn pane_cwds() -> Result<Vec<String>, TmuxError> {
    pane_cwds_with(&mut ProductionRunner)
}

fn pane_cwds_with(
    runner: &mut dyn CommandRunner,
) -> Result<Vec<String>, TmuxError> {
    let output = runner
        .run(&["list-panes", "-a", "-F", "#{pane_current_path}"])
        .map_err(TmuxError::IoError)?;
    if !output.success {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(TmuxError::CommandFailed(io::Error::other(stderr)));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| {
        TmuxError::IoError(io::Error::new(
            InvalidData,
            "tmux printed invalid UTF-8",
        ))
    })?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Switch to an observed session. Absence at switch time means the observed
/// resource disappeared; it is reported, never recreated.
fn switch(
    runner: &mut dyn CommandRunner,
    session: String,
) -> Result<Activation, ActivationError> {
    match switch_client(runner, &session) {
        CommandResult::Success => Ok(Activation { session }),
        CommandResult::SessionAbsent { .. } => {
            Err(ActivationError::SessionDisappeared { session })
        }
        CommandResult::Failed(cause) => {
            Err(ActivationError::SwitchFailed { session, cause })
        }
    }
}

#[cfg(test)]
#[path = "tmux_tests.rs"]
mod tests;
