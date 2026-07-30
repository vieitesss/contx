pub mod errors;
pub mod sessions;

use std::{
    env,
    io::{self, ErrorKind::InvalidData},
    process::Command,
};

use errors::TmuxError;

pub type Result<T> = std::result::Result<T, TmuxError>;

fn tmux_command(args: &[&str]) -> Result<String> {
    let output = Command::new("tmux").args(args).output()?;

    if !output.status.success() {
        return Err(TmuxError::CommandFailed(io::Error::other(
            String::from_utf8_lossy(&output.stderr).to_owned(),
        )));
    }

    String::from_utf8(output.stdout)
        .map_err(|err| TmuxError::IoError(io::Error::new(InvalidData, err)))
}

fn is_tmux_process() -> bool {
    if let Ok(value) = env::var("TMUX") {
        return value != "";
    }

    false
}

pub fn normalize_session_name(name: &str) -> String {
    name.replace(".", "_")
}

fn switch_client(session: &str) -> Result<()> {
    tmux_command(&["switch-client", "-t", session])?;
    Ok(())
}

fn new_session(session: &str, path: &str) -> Result<()> {
    tmux_command(&["new-session", "-ds", session, "-c", path])?;
    Ok(())
}

fn has_session(session: &str) -> Result<()> {
    tmux_command(&["has-session", "-t", session])?;
    Ok(())
}

pub fn open(session_name: &str, path: &str) -> Result<()> {
    if !is_tmux_process() {
        return Err(TmuxError::NotInTmux);
    }

    if let Err(_) = has_session(session_name) {
        new_session(session_name, path)?;
    } else {
        switch_client(session_name)?;
    }

    Ok(())
}

// pub fn sessions() -> Result<Vec<TmuxSession>> {
//     if !is_tmux_process() {
//         return Err(TmuxError::NotInTmux);
//     }
//
//     let output: String = tmux_command(&["list-sessions"])?;
//
//     Ok(sessions::parse(&output))
// }
