use std::error;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum TmuxError {
    NotInTmux,
    IoError(std::io::Error),
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
            TmuxError::NotInTmux => write!(f, "not running inside tmux"),
            TmuxError::IoError(e) => write!(f, "{e:?}"),
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
            TmuxError::NotInTmux => None,
            TmuxError::IoError(e) => Some(e),
        }
    }
}
