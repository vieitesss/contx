use std::{env, fmt, path::Path};

use serde::Serialize;

use crate::{
    clone,
    config::{Multiplexer, ResolvedConfig},
    delete, herdr,
    mux::{self, Backend},
    tmux,
};

/// Resolve a CLI path to a live candidate. All workspace and worktree
/// operations use catalog identity, not a guessed project name.
pub(crate) fn candidate_path<'a>(
    config: &'a ResolvedConfig,
    path: &str,
) -> Result<&'a str, String> {
    let cwd = env::current_dir().map_err(|e| e.to_string())?;
    let abs =
        clone::resolve_destination(path, Some(&cwd), &|name| env::var_os(name))
            .map_err(|e| e.to_string())?;
    let candidate = delete::match_candidate(&abs, &config.candidates)
        .ok_or_else(|| format!("not a session candidate: `{abs}`"))?;
    if !Path::new(&candidate.path).is_dir() {
        return Err(format!(
            "candidate is no longer a directory: `{}`",
            candidate.path
        ));
    }
    Ok(&candidate.path)
}

#[derive(Debug, Serialize)]
#[serde(tag = "multiplexer", rename_all = "lowercase")]
pub(crate) enum Opened {
    Tmux { path: String, session: String },
    Herdr { path: String, workspace_id: String },
}

#[derive(Debug)]
pub(crate) struct OpenError {
    pub message: String,
    pub workspace_ids: Option<Vec<String>>,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<String> for OpenError {
    fn from(message: String) -> Self {
        Self {
            message,
            workspace_ids: None,
        }
    }
}

impl From<herdr::OpenError> for OpenError {
    fn from(error: herdr::OpenError) -> Self {
        let workspace_ids = match &error {
            herdr::OpenError::NotATty { ids } => Some(ids.clone()),
            herdr::OpenError::WorkspaceNotMatching { ids, .. } => {
                Some(ids.clone())
            }
            _ => None,
        };
        Self {
            message: error.to_string(),
            workspace_ids,
        }
    }
}

/// Open/focus a current candidate, never asking stdin to disambiguate.
pub(crate) fn open(
    config: &ResolvedConfig,
    path: &str,
    workspace_id: Option<&str>,
) -> Result<Opened, OpenError> {
    let path = candidate_path(config, path)?;
    let backend = match config.multiplexer {
        Multiplexer::Auto => {
            mux::detect_auto_backend().map_err(|e| e.to_string())?
        }
        Multiplexer::Tmux => Backend::Tmux,
        Multiplexer::Herdr => Backend::Herdr,
    };
    match backend {
        Backend::Tmux => {
            if workspace_id.is_some() {
                return Err("--workspace-id requires the Herdr multiplexer"
                    .to_string()
                    .into());
            }
            let activation = tmux::open(path).map_err(|e| e.to_string())?;
            Ok(Opened::Tmux {
                path: path.to_string(),
                session: activation.session,
            })
        }
        Backend::Herdr => {
            mux::require_herdr_context().map_err(|e| e.to_string())?;
            let activation = herdr::open_noninteractive(path, workspace_id)
                .map_err(OpenError::from)?;
            Ok(Opened::Herdr {
                path: path.to_string(),
                workspace_id: activation.workspace_id,
            })
        }
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
