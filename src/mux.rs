use std::{
    env,
    ffi::OsString,
    fmt,
    io::{self, ErrorKind},
    process::Command,
};

use crate::config::Multiplexer;
use crate::herdr;
use crate::tmux;

/// Inner multiplexer chosen by `auto` detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Tmux,
    Herdr,
}

/// Why `auto` refused to choose a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectError {
    /// Neither `TMUX` nor `HERDR_ENV=1`.
    Outside,
    /// Both markers, but tmux TTY ownership could not be completed.
    Ambiguous,
    /// Only `TMUX`, and TTY was unknown or the tmux probe failed.
    TmuxOwnershipUnknown,
    /// Only `TMUX`, TTY known, not a live pane or client (stale `TMUX`).
    StaleTmux,
    /// `HERDR_ENV=1` without a nonempty `HERDR_SOCKET_PATH`.
    MissingSocket,
}

/// Live tmux pane and client TTY names from a successful probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxTtys {
    pub panes: Vec<String>,
    pub clients: Vec<String>,
}

/// Injected tmux TTY-list failure. Production mapping comes later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxProbeError {
    Io,
    Command,
    InvalidUtf8,
}

fn env_nonempty(env: &dyn Fn(&str) -> Option<OsString>, name: &str) -> bool {
    matches!(env(name), Some(value) if !value.is_empty())
}

fn herdr_env(env: &dyn Fn(&str) -> Option<OsString>) -> bool {
    matches!(env("HERDR_ENV"), Some(value) if value == "1")
}

fn usable_tty(tty: Option<&str>) -> Option<&str> {
    tty.map(str::trim).filter(|t| !t.is_empty())
}

fn owns_tty(ttys: &TmuxTtys, tty: &str) -> bool {
    ttys.panes
        .iter()
        .chain(&ttys.clients)
        .any(|listed| listed.trim() == tty)
}

/// Select the inner multiplexer for `auto`. Explicit preferences are not
/// handled here. `probe` runs only when `TMUX` is set.
pub fn detect_auto(
    env: &dyn Fn(&str) -> Option<OsString>,
    tty: Option<&str>,
    probe: &mut dyn FnMut() -> Result<TmuxTtys, TmuxProbeError>,
) -> Result<Backend, DetectError> {
    let tmux = env_nonempty(env, "TMUX");
    let herdr = herdr_env(env);
    let socket = env_nonempty(env, "HERDR_SOCKET_PATH");

    match (tmux, herdr) {
        (false, false) => Err(DetectError::Outside),
        (true, false) => {
            let Some(tty) = usable_tty(tty) else {
                return Err(DetectError::TmuxOwnershipUnknown);
            };
            match probe() {
                Ok(ttys) if owns_tty(&ttys, tty) => Ok(Backend::Tmux),
                Ok(_) => Err(DetectError::StaleTmux),
                Err(_) => Err(DetectError::TmuxOwnershipUnknown),
            }
        }
        (false, true) => {
            if socket {
                Ok(Backend::Herdr)
            } else {
                Err(DetectError::MissingSocket)
            }
        }
        (true, true) => {
            let Some(tty) = usable_tty(tty) else {
                return Err(DetectError::Ambiguous);
            };
            match probe() {
                Ok(ttys) if owns_tty(&ttys, tty) => Ok(Backend::Tmux),
                Ok(_) if socket => Ok(Backend::Herdr),
                Ok(_) => Err(DetectError::MissingSocket),
                Err(_) => Err(DetectError::Ambiguous),
            }
        }
    }
}

/// Outcome of routing activation after the TUI is restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateResult {
    Completed,
    Cancelled,
}

/// Neutral activation failure. Chosen-backend errors never fall back.
#[derive(Debug)]
pub enum ActivateError {
    Detect(DetectError),
    NotInHerdr,
    Tmux(tmux::errors::ActivationError),
    Herdr(herdr::OpenError),
}

impl fmt::Display for ActivateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Detect(DetectError::Outside) => write!(
                f,
                "not running inside a multiplexer; pass --multiplexer tmux or --multiplexer herdr"
            ),
            Self::Detect(DetectError::Ambiguous) => write!(
                f,
                "ambiguous multiplexer; pass --multiplexer tmux or --multiplexer herdr"
            ),
            Self::Detect(DetectError::TmuxOwnershipUnknown) => write!(
                f,
                "could not confirm tmux owns this terminal; pass --multiplexer tmux or --multiplexer herdr"
            ),
            Self::Detect(DetectError::StaleTmux) => write!(
                f,
                "TMUX is set but this terminal is not a live tmux pane or client; pass --multiplexer tmux or --multiplexer herdr"
            ),
            Self::Detect(DetectError::MissingSocket) => write!(
                f,
                "HERDR_ENV is set but HERDR_SOCKET_PATH is missing; pass --multiplexer tmux or --multiplexer herdr"
            ),
            Self::NotInHerdr => write!(f, "not running inside herdr"),
            Self::Tmux(e) => write!(f, "{e}"),
            Self::Herdr(e) => write!(f, "{e}"),
        }
    }
}

trait CommandRunner {
    fn run(&mut self, program: &str, args: &[&str]) -> io::Result<RawOutput>;
}

struct RawOutput {
    success: bool,
    stdout: Vec<u8>,
}

struct ProductionRunner;

impl CommandRunner for ProductionRunner {
    fn run(&mut self, program: &str, args: &[&str]) -> io::Result<RawOutput> {
        let output = Command::new(program).args(args).output()?;
        Ok(RawOutput {
            success: output.status.success(),
            stdout: output.stdout,
        })
    }
}

fn lines_from(stdout: Vec<u8>) -> Result<Vec<String>, TmuxProbeError> {
    let text =
        String::from_utf8(stdout).map_err(|_| TmuxProbeError::InvalidUtf8)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn tmux_tty_list(
    runner: &mut dyn CommandRunner,
    args: &[&str],
) -> Result<Vec<String>, TmuxProbeError> {
    let output = match runner.run("tmux", args) {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(TmuxProbeError::Io);
        }
        Err(_) => return Err(TmuxProbeError::Io),
        Ok(output) => output,
    };
    if !output.success {
        return Err(TmuxProbeError::Command);
    }
    lines_from(output.stdout)
}

fn probe_tmux_ttys(
    runner: &mut dyn CommandRunner,
) -> Result<TmuxTtys, TmuxProbeError> {
    Ok(TmuxTtys {
        panes: tmux_tty_list(
            runner,
            &["list-panes", "-a", "-F", "#{pane_tty}"],
        )?,
        clients: tmux_tty_list(
            runner,
            &["list-clients", "-F", "#{client_tty}"],
        )?,
    })
}

fn herdr_context(env: &dyn Fn(&str) -> Option<OsString>) -> bool {
    herdr_env(env) && env_nonempty(env, "HERDR_SOCKET_PATH")
}

fn map_herdr(
    result: Result<Option<herdr::Activation>, herdr::OpenError>,
) -> Result<ActivateResult, ActivateError> {
    match result {
        Ok(Some(_)) => Ok(ActivateResult::Completed),
        Ok(None) => Ok(ActivateResult::Cancelled),
        Err(e) => Err(ActivateError::Herdr(e)),
    }
}

fn map_tmux(
    result: Result<tmux::Activation, tmux::errors::ActivationError>,
) -> Result<ActivateResult, ActivateError> {
    result
        .map(|_| ActivateResult::Completed)
        .map_err(ActivateError::Tmux)
}

fn activate_auto_with(
    candidate: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
    tty: Option<&str>,
    probe: &mut dyn FnMut() -> Result<TmuxTtys, TmuxProbeError>,
    tmux_open: &mut dyn FnMut(
        &str,
    ) -> Result<
        tmux::Activation,
        tmux::errors::ActivationError,
    >,
    herdr_open: &mut dyn FnMut(
        &str,
    ) -> Result<
        Option<herdr::Activation>,
        herdr::OpenError,
    >,
) -> Result<ActivateResult, ActivateError> {
    match detect_auto(env, tty, probe) {
        Ok(Backend::Tmux) => map_tmux(tmux_open(candidate)),
        Ok(Backend::Herdr) => map_herdr(herdr_open(candidate)),
        Err(e) => Err(ActivateError::Detect(e)),
    }
}

fn activate_explicit_herdr_with(
    candidate: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
    herdr_open: &mut dyn FnMut(
        &str,
    ) -> Result<
        Option<herdr::Activation>,
        herdr::OpenError,
    >,
) -> Result<ActivateResult, ActivateError> {
    if !herdr_context(env) {
        return Err(ActivateError::NotInHerdr);
    }
    map_herdr(herdr_open(candidate))
}

/// `auto`: detect the inner multiplexer, then activate only that backend.
pub fn activate_auto(candidate: &str) -> Result<ActivateResult, ActivateError> {
    let tty = controlling_tty();
    let mut runner = ProductionRunner;
    let mut probe = || probe_tmux_ttys(&mut runner);
    activate_auto_with(
        candidate,
        &|name| env::var_os(name),
        tty.as_deref(),
        &mut probe,
        &mut tmux::open,
        &mut herdr::open,
    )
}

/// Resolve `auto` for a noninteractive CLI operation without opening a target.
pub(crate) fn detect_auto_backend() -> Result<Backend, ActivateError> {
    let tty = controlling_tty();
    let mut runner = ProductionRunner;
    detect_auto(&|name| env::var_os(name), tty.as_deref(), &mut || {
        probe_tmux_ttys(&mut runner)
    })
    .map_err(ActivateError::Detect)
}

pub(crate) fn require_herdr_context() -> Result<(), ActivateError> {
    if herdr_context(&|name| env::var_os(name)) {
        Ok(())
    } else {
        Err(ActivateError::NotInHerdr)
    }
}

/// Explicit tmux: skip inner-vs-outer TTY probe; `tmux::open` still
/// requires nonempty `TMUX`.
pub fn activate_explicit_tmux(
    candidate: &str,
) -> Result<ActivateResult, ActivateError> {
    map_tmux(tmux::open(candidate))
}

/// Explicit herdr: skip inner-vs-outer TTY probe, but refuse without
/// current Herdr context so we never target a default server.
pub fn activate_explicit_herdr(
    candidate: &str,
) -> Result<ActivateResult, ActivateError> {
    activate_explicit_herdr_with(
        candidate,
        &|name| env::var_os(name),
        &mut herdr::open,
    )
}

/// Result of listing pane working directories for active-target detection.
/// `Skipped` means the selected multiplexer context is not live; `Failed`
/// means the context is live but listing did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneCwdOutcome {
    Skipped,
    Listed(Vec<String>),
    Failed,
}

fn map_list(result: Result<Vec<String>, ()>) -> PaneCwdOutcome {
    match result {
        Ok(cwds) => PaneCwdOutcome::Listed(cwds),
        Err(()) => PaneCwdOutcome::Failed,
    }
}

/// List pane cwds for the invocation's selected multiplexer only.
pub(crate) fn pane_cwds(multiplexer: Multiplexer) -> PaneCwdOutcome {
    let tty = controlling_tty();
    pane_cwds_with(
        multiplexer,
        &|name| env::var_os(name),
        tty.as_deref(),
        &mut || {
            let mut runner = ProductionRunner;
            probe_tmux_ttys(&mut runner)
        },
        &mut || tmux::pane_cwds().map_err(|_| ()),
        &mut || herdr::pane_cwds().map_err(|_| ()),
    )
}

fn pane_cwds_with(
    multiplexer: Multiplexer,
    env: &dyn Fn(&str) -> Option<OsString>,
    tty: Option<&str>,
    probe: &mut dyn FnMut() -> Result<TmuxTtys, TmuxProbeError>,
    tmux_list: &mut dyn FnMut() -> Result<Vec<String>, ()>,
    herdr_list: &mut dyn FnMut() -> Result<Vec<String>, ()>,
) -> PaneCwdOutcome {
    match multiplexer {
        Multiplexer::Tmux => {
            if !env_nonempty(env, "TMUX") {
                return PaneCwdOutcome::Skipped;
            }
            map_list(tmux_list())
        }
        Multiplexer::Herdr => {
            if !herdr_context(env) {
                return PaneCwdOutcome::Skipped;
            }
            map_list(herdr_list())
        }
        Multiplexer::Auto => match detect_auto(env, tty, probe) {
            Ok(Backend::Tmux) => map_list(tmux_list()),
            Ok(Backend::Herdr) => map_list(herdr_list()),
            Err(DetectError::Outside) => PaneCwdOutcome::Skipped,
            Err(DetectError::MissingSocket) => PaneCwdOutcome::Skipped,
            Err(_) => PaneCwdOutcome::Failed,
        },
    }
}

/// Controlling TTY name, or `None` when it cannot be obtained.
/// Unix: open `/dev/tty` and `ttyname` that fd. Not production-proven
/// against nested Herdr.
pub fn controlling_tty() -> Option<String> {
    #[cfg(unix)]
    {
        controlling_tty_unix()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(unix)]
fn controlling_tty_unix() -> Option<String> {
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn ttyname(fd: i32) -> *const std::os::raw::c_char;
    }

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let ptr = unsafe { ttyname(file.as_raw_fd()) };
    if ptr.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "mux_tests.rs"]
mod tests;
