use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    ffi::OsString,
    fmt,
    io::{self, BufRead, ErrorKind, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command,
};

/// Classified failure at the Herdr CLI JSON boundary. Never falls back
/// to tmux, and never retries `herdr` on PATH after a set `HERDR_BIN_PATH`.
#[derive(Debug)]
pub enum HerdrError {
    NotFound { program: String },
    UnknownCommand { detail: String },
    GarbageJson,
    ServerNotRunning { message: String },
    ProtocolMismatch { message: String },
    Api { code: String, message: String },
    Io(io::Error),
}

impl fmt::Display for HerdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { program } => {
                write!(f, "herdr binary `{program}` not found")
            }
            Self::UnknownCommand { detail } => {
                write!(f, "herdr command not available: {detail}")
            }
            Self::GarbageJson => {
                write!(f, "herdr printed unreadable JSON")
            }
            Self::ServerNotRunning { message } => {
                write!(f, "{message}")
            }
            Self::ProtocolMismatch { message } => {
                write!(f, "{message}")
            }
            Self::Api { code, message } => {
                write!(f, "herdr error `{code}`: {message}")
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Narrow seam at the Herdr command boundary: execute one argv against a
/// resolved program. Production runs the external binary; tests script it.
trait CommandRunner {
    fn run(&mut self, program: &str, args: &[&str]) -> io::Result<RawOutput>;
}

/// One Herdr process invocation, before classification.
#[derive(Debug, Clone, PartialEq)]
struct RawOutput {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct ProductionRunner;

impl CommandRunner for ProductionRunner {
    fn run(&mut self, program: &str, args: &[&str]) -> io::Result<RawOutput> {
        let output = Command::new(program).args(args).output()?;
        Ok(RawOutput {
            code: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[derive(Deserialize)]
struct Envelope {
    result: Option<Value>,
    error: Option<ErrorBody>,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    #[serde(default)]
    message: String,
}

fn program(env: &dyn Fn(&str) -> Option<OsString>) -> String {
    match env("HERDR_BIN_PATH") {
        Some(path) if !path.is_empty() => path.to_string_lossy().into_owned(),
        _ => "herdr".to_string(),
    }
}

fn invoke(
    runner: &mut dyn CommandRunner,
    env: &dyn Fn(&str) -> Option<OsString>,
    args: &[&str],
) -> Result<Value, HerdrError> {
    let program = program(env);
    let result = runner.run(&program, args);
    classify(program, result)
}

fn classify(
    program: String,
    result: io::Result<RawOutput>,
) -> Result<Value, HerdrError> {
    let output = match result {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(HerdrError::NotFound { program });
        }
        Err(e) => return Err(HerdrError::Io(e)),
        Ok(output) => output,
    };
    match output.code {
        0 => parse_success(&output.stdout),
        2 => Err(HerdrError::UnknownCommand {
            detail: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        _ => parse_failure(&output.stderr, &output.stdout),
    }
}

fn parse_success(stdout: &[u8]) -> Result<Value, HerdrError> {
    let envelope: Envelope =
        serde_json::from_slice(stdout).map_err(|_| HerdrError::GarbageJson)?;
    if let Some(error) = envelope.error {
        return Err(api_error(error));
    }
    envelope.result.ok_or(HerdrError::GarbageJson)
}

fn parse_failure(stderr: &[u8], stdout: &[u8]) -> Result<Value, HerdrError> {
    let raw = if stderr.is_empty() { stdout } else { stderr };
    let envelope: Envelope =
        serde_json::from_slice(raw).map_err(|_| HerdrError::GarbageJson)?;
    match envelope.error {
        Some(error) => Err(api_error(error)),
        None => Err(HerdrError::GarbageJson),
    }
}

fn api_error(error: ErrorBody) -> HerdrError {
    match error.code.as_str() {
        "server_not_running" => HerdrError::ServerNotRunning {
            message: error.message,
        },
        "protocol_mismatch" => HerdrError::ProtocolMismatch {
            message: error.message,
        },
        code => HerdrError::Api {
            code: code.to_string(),
            message: error.message,
        },
    }
}

fn pane_list(
    runner: &mut dyn CommandRunner,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Value, HerdrError> {
    invoke(runner, env, &["pane", "list"])
}

fn workspace_focus(
    runner: &mut dyn CommandRunner,
    env: &dyn Fn(&str) -> Option<OsString>,
    workspace_id: &str,
) -> Result<Value, HerdrError> {
    invoke(runner, env, &["workspace", "focus", workspace_id])
}

fn workspace_create(
    runner: &mut dyn CommandRunner,
    env: &dyn Fn(&str) -> Option<OsString>,
    cwd: &str,
    label: &str,
) -> Result<Value, HerdrError> {
    invoke(
        runner,
        env,
        &[
            "workspace",
            "create",
            "--cwd",
            cwd,
            "--label",
            label,
            "--focus",
        ],
    )
}

/// Deliberate success of activating a Herdr workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub workspace_id: String,
}

/// Outcome of asking which matching workspace to activate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    Workspace(String),
    Cancel,
}

/// Policy-level failure after the CLI JSON boundary.
#[derive(Debug)]
pub enum OpenError {
    CanonicalizeFailed { candidate: String },
    InvalidCandidate(String),
    Cli(HerdrError),
    Disappeared { workspace_id: String },
    NotATty { ids: Vec<String> },
    InvalidChoice,
    MalformedResult,
    Io(io::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalizeFailed { candidate } => write!(
                f,
                "cannot canonicalize `{candidate}`; not creating a workspace"
            ),
            Self::InvalidCandidate(candidate) => {
                write!(f, "cannot derive a workspace label from `{candidate}`")
            }
            Self::Cli(e) => write!(f, "{e}"),
            Self::Disappeared { workspace_id } => write!(
                f,
                "Herdr workspace `{workspace_id}` disappeared; not recreating it"
            ),
            Self::NotATty { ids } => write!(
                f,
                "multiple Herdr workspaces match ({}); not a TTY so cannot ask",
                ids.join(", ")
            ),
            Self::InvalidChoice => {
                write!(f, "invalid workspace choice; not creating")
            }
            Self::MalformedResult => {
                write!(f, "unexpected Herdr response; not creating")
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

fn matches_canonical(reported: Option<&str>, canonical: &Path) -> bool {
    let Some(raw) = reported.filter(|s| !s.is_empty()) else {
        return false;
    };
    match Path::new(raw).canonicalize() {
        Ok(path) => path == canonical,
        Err(_) => false,
    }
}

fn expect_type(result: &Value, expected: &str) -> Result<(), OpenError> {
    if result.get("type").and_then(Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(OpenError::MalformedResult)
    }
}

fn matching_ids(
    result: &Value,
    canonical: &Path,
) -> Result<Vec<String>, OpenError> {
    expect_type(result, "pane_list")?;
    let Some(panes) = result.get("panes").and_then(Value::as_array) else {
        return Err(OpenError::MalformedResult);
    };
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for pane in panes {
        let Some(id) = pane.get("workspace_id").and_then(Value::as_str) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let cwd = pane.get("cwd").and_then(Value::as_str);
        let foreground = pane.get("foreground_cwd").and_then(Value::as_str);
        if !(matches_canonical(cwd, canonical)
            || matches_canonical(foreground, canonical))
        {
            continue;
        }
        if seen.insert(id.to_string()) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn workspace_id_from(result: &Value) -> Result<String, OpenError> {
    result
        .get("workspace")
        .and_then(|w| w.get("workspace_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or(OpenError::MalformedResult)
}

fn focus_observed(
    runner: &mut dyn CommandRunner,
    env: &dyn Fn(&str) -> Option<OsString>,
    workspace_id: &str,
) -> Result<Option<Activation>, OpenError> {
    match workspace_focus(runner, env, workspace_id) {
        Ok(result) => {
            expect_type(&result, "workspace_info")?;
            Ok(Some(Activation {
                workspace_id: workspace_id_from(&result)?,
            }))
        }
        Err(HerdrError::Api { code, .. }) if code == "workspace_not_found" => {
            Err(OpenError::Disappeared {
                workspace_id: workspace_id.to_string(),
            })
        }
        Err(e) => Err(OpenError::Cli(e)),
    }
}

fn label_for(
    candidate: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<String, OpenError> {
    crate::tmux::session_name(candidate, env).map_err(|e| match e {
        crate::tmux::errors::ActivationError::InvalidCandidate(c) => {
            OpenError::InvalidCandidate(c)
        }
        _ => OpenError::InvalidCandidate(candidate.to_string()),
    })
}

fn choose_from(
    candidate: &str,
    ids: &[String],
    stdin_is_terminal: bool,
    stdin: &mut dyn BufRead,
    stderr: &mut dyn Write,
) -> Result<Choice, OpenError> {
    if !stdin_is_terminal {
        return Err(OpenError::NotATty { ids: ids.to_vec() });
    }
    writeln!(stderr, "Multiple Herdr workspaces match `{candidate}`:")
        .map_err(OpenError::Io)?;
    for (i, id) in ids.iter().enumerate() {
        writeln!(stderr, "  {}) {id}", i + 1).map_err(OpenError::Io)?;
    }
    writeln!(stderr, "Enter number to activate, or empty to cancel:")
        .map_err(OpenError::Io)?;
    let mut line = String::new();
    let n = stdin.read_line(&mut line).map_err(OpenError::Io)?;
    if n == 0 {
        return Ok(Choice::Cancel);
    }
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed == "q" {
        return Ok(Choice::Cancel);
    }
    let Ok(index) = trimmed.parse::<usize>() else {
        return Err(OpenError::InvalidChoice);
    };
    if index >= 1 && index <= ids.len() {
        Ok(Choice::Workspace(ids[index - 1].clone()))
    } else {
        Err(OpenError::InvalidChoice)
    }
}

fn production_choose(
    candidate: &str,
    ids: &[String],
) -> Result<Choice, OpenError> {
    let stdin = io::stdin();
    let is_tty = stdin.is_terminal();
    choose_from(candidate, ids, is_tty, &mut stdin.lock(), &mut io::stderr())
}

fn open_with(
    candidate: &str,
    runner: &mut dyn CommandRunner,
    env: &dyn Fn(&str) -> Option<OsString>,
    chooser: &mut dyn FnMut(&str, &[String]) -> Result<Choice, OpenError>,
) -> Result<Option<Activation>, OpenError> {
    let canonical: PathBuf =
        Path::new(candidate).canonicalize().map_err(|_| {
            OpenError::CanonicalizeFailed {
                candidate: candidate.to_string(),
            }
        })?;
    let listed = pane_list(runner, env).map_err(OpenError::Cli)?;
    let ids = matching_ids(&listed, &canonical)?;
    match ids.as_slice() {
        [] => {
            let label = label_for(candidate, env)?;
            let cwd = canonical.display().to_string();
            let result = workspace_create(runner, env, &cwd, &label)
                .map_err(OpenError::Cli)?;
            expect_type(&result, "workspace_created")?;
            Ok(Some(Activation {
                workspace_id: workspace_id_from(&result)?,
            }))
        }
        [id] => focus_observed(runner, env, id),
        _ => match chooser(candidate, &ids)? {
            Choice::Cancel => Ok(None),
            Choice::Workspace(id) => focus_observed(runner, env, &id),
        },
    }
}

/// Activate the Herdr workspace for a session candidate.
pub fn open(candidate: &str) -> Result<Option<Activation>, OpenError> {
    let mut runner = ProductionRunner;
    open_with(
        candidate,
        &mut runner,
        &|name| std::env::var_os(name),
        &mut production_choose,
    )
}

#[cfg(test)]
#[path = "herdr_tests.rs"]
mod tests;
