use std::error;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum TmuxError {
    IoError(std::io::Error),
    CommandFailed(std::io::Error),
}

impl From<io::Error> for TmuxError {
    fn from(error: io::Error) -> Self {
        TmuxError::IoError(error)
    }
}

// Use Display so we can print the errors.
impl fmt::Display for TmuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TmuxError::IoError(e) => write!(f, "{e:?}"),
            TmuxError::CommandFailed(e) => write!(f, "{e:?}"),
        }
    }
}

// Overwrite source() only when we are wrapping a different error.
// e.g.:
// TmuxError {
//     IoError(std::io::Error)
// }
impl error::Error for TmuxError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            TmuxError::IoError(e) => Some(e),
            TmuxError::CommandFailed(e) => Some(e),
        }
    }
}

/// Deliberate outcome of a failed activation: which step stopped the
/// command sequence and why. The caller reports this instead of panicking.
#[derive(Debug)]
pub enum ActivationError {
    /// Not running inside tmux; no tmux command ran.
    NotInTmux,
    /// The candidate path cannot be mapped to a tmux session name.
    InvalidCandidate(String),
    /// `has-session` itself failed, so absence could not be established.
    CheckFailed { session: String, cause: TmuxError },
    /// `new-session` failed after authoritative absence.
    CreateFailed { session: String, cause: TmuxError },
    /// `switch-client` failed; the session may exist but we did not attach.
    SwitchFailed { session: String, cause: TmuxError },
    /// The session was observed but was gone at switch time. Per
    /// observed-resource policy it is reported, never recreated.
    SessionDisappeared { session: String },
}

impl fmt::Display for ActivationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActivationError::NotInTmux => {
                write!(f, "not running inside tmux")
            }
            ActivationError::InvalidCandidate(candidate) => {
                write!(
                    f,
                    "cannot derive a tmux session name from `{candidate}`"
                )
            }
            ActivationError::CheckFailed { session, cause } => {
                write!(f, "could not check tmux session `{session}`: {cause}")
            }
            ActivationError::CreateFailed { session, cause } => {
                write!(f, "could not create tmux session `{session}`: {cause}")
            }
            ActivationError::SwitchFailed { session, cause } => {
                write!(
                    f,
                    "could not switch to tmux session `{session}`: {cause}"
                )
            }
            ActivationError::SessionDisappeared { session } => {
                write!(
                    f,
                    "tmux session `{session}` disappeared; not recreating it"
                )
            }
        }
    }
}

impl error::Error for ActivationError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            ActivationError::CheckFailed { cause, .. }
            | ActivationError::CreateFailed { cause, .. }
            | ActivationError::SwitchFailed { cause, .. } => Some(cause),
            ActivationError::NotInTmux
            | ActivationError::InvalidCandidate(_)
            | ActivationError::SessionDisappeared { .. } => None,
        }
    }
}
