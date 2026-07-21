pub mod errors;
pub mod sessions;

use errors::TmuxError;
use log::debug;
use sessions::TmuxSession;
use std::{
    env,
    io::{self, ErrorKind::InvalidData},
    process::Command,
};

fn tmux_command(args: &[&str]) -> Result<String, TmuxError> {
    let output = Command::new("tmux").args(args).output()?;

    if !output.status.success() {
        return Err(TmuxError::IoError(io::Error::other(
            String::from_utf8_lossy(&output.stderr).to_owned(),
        )));
    }

    String::from_utf8(output.stdout)
        .map_err(|err| TmuxError::IoError(io::Error::new(InvalidData, err)))
}

fn is_tmux_process() -> bool {
    if let Ok(value) = env::var("TMUX") {
        debug!("TMUX={value}");
        return value != "";
    }

    false
}

pub fn sessions() -> Result<Vec<TmuxSession>, TmuxError> {
    if !is_tmux_process() {
        return Err(TmuxError::NotInTmux);
    }

    let output: String = tmux_command(&["list-sessions"])?;

    Ok(sessions::parse(&output))
}
