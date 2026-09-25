mod cli;
mod clone;
mod config;
mod delete;
mod fuzzy;
mod herdr;
mod label;
mod mux;
mod theme;
mod tmux;
mod tui;
mod utils;
mod worktree;

use env_logger::{Builder, Target};
use serde::Serialize;
use std::{
    env,
    ffi::OsString,
    fs::{self, OpenOptions},
    io,
    path::PathBuf,
    process::exit,
};

use config::Multiplexer;
use mux::{ActivateError, ActivateResult};
use tui::Tui;

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

fn main() -> io::Result<()> {
    init_logging();

    let error_json = std::env::args().any(|arg| arg == "--json");
    match config::resolve() {
        Ok(config::Startup::Help) => {
            print!("{}", config::USAGE);
        }
        Ok(config::Startup::Ready(resolved)) => match &resolved.command {
            config::Command::Picker => {
                let theme = theme::Theme::detect();
                let selected = ratatui::run(|terminal| {
                    Tui::from_config(*resolved.clone(), theme).run(terminal)
                })?;
                if let Some(candidate) = selected {
                    let outcome = match resolved.multiplexer {
                        Multiplexer::Auto => mux::activate_auto(&candidate),
                        Multiplexer::Tmux => {
                            mux::activate_explicit_tmux(&candidate)
                        }
                        Multiplexer::Herdr => {
                            mux::activate_explicit_herdr(&candidate)
                        }
                    };
                    if let Some(diagnostic) = report_activation(outcome) {
                        eprintln!("{diagnostic}");
                        exit(1);
                    }
                }
            }
            config::Command::List => {
                if resolved.json {
                    json_line(
                        &serde_json::json!({ "candidates": resolved.candidates }),
                    )?;
                } else {
                    for candidate in &resolved.candidates {
                        println!("{}", candidate.path);
                    }
                }
            }
            config::Command::Open { path, workspace_id } => {
                let opened = cli::open(&resolved, path, workspace_id.as_deref())
                    .unwrap_or_else(|e| {
                        if resolved.json && let Some(ids) = &e.workspace_ids {
                            eprintln!("{}", serde_json::json!({ "error": e.message, "workspace_ids": ids }));
                            exit(1);
                        }
                        fail(&e.to_string(), resolved.json)
                    });
                if resolved.json {
                    json_line(&opened)?;
                } else {
                    match opened {
                        cli::Opened::Tmux { path, session } => {
                            println!("{path} (tmux session {session})")
                        }
                        cli::Opened::Herdr { path, workspace_id } => {
                            println!("{path} (Herdr workspace {workspace_id})")
                        }
                    }
                }
            }
            config::Command::Clone {
                source,
                destination,
                add_parent,
            } => {
                match clone::run(&resolved, source, destination, *add_parent) {
                    Ok(outcome) => {
                        if resolved.json {
                            json_line(&serde_json::json!({
                                "source": source,
                                "destination": outcome.dest,
                                "config_updated": outcome.config_updated,
                                "discoverable": outcome.discoverable,
                            }))?;
                        }
                    }
                    Err(e) => fail(&e.to_string(), resolved.json),
                }
            }
            config::Command::WorktreeCreate {
                repo,
                branch,
                destination,
                new_branch,
                add_parent,
            } => {
                let outcome = worktree::create(
                    &resolved,
                    repo,
                    branch,
                    destination,
                    *new_branch,
                    *add_parent,
                )
                .unwrap_or_else(|e| fail(&e, resolved.json));
                if resolved.json {
                    json_line(&outcome)?;
                } else {
                    println!("{}", outcome.destination);
                }
            }
            config::Command::Delete {
                path,
                dry_run,
                permanent,
                force,
            } => {
                let request = delete::DeleteRequest {
                    path: path.clone(),
                    dry_run: *dry_run,
                    permanent: *permanent,
                    force: *force,
                };
                match delete::run(&resolved, &request) {
                    Ok(delete::DeleteOutcome::DryRun(report)) => {
                        if resolved.json {
                            json_line(
                                &serde_json::json!({ "preflight": report }),
                            )?;
                        }
                    }
                    Ok(delete::DeleteOutcome::Deleted { path, strategy }) => {
                        if resolved.json {
                            json_line(
                                &serde_json::json!({ "path": path, "strategy": strategy }),
                            )?;
                        }
                    }
                    Ok(delete::DeleteOutcome::Cancelled) => {
                        if resolved.json {
                            json_line(
                                &serde_json::json!({ "cancelled": true }),
                            )?;
                        }
                    }
                    Err(e) => {
                        if let delete::DeleteError::Blocked(ref report) = e
                            && resolved.json
                        {
                            eprintln!(
                                "{}",
                                serde_json::json!({ "error": "deletion blocked", "preflight": report })
                            );
                            exit(1);
                        }
                        fail(&e.to_string(), resolved.json);
                    }
                }
            }
        },
        Err(e) => fail(&e.to_string(), error_json),
    }

    Ok(())
}

fn json_line(value: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(io::stdout(), value).map_err(io::Error::other)?;
    println!();
    Ok(())
}

fn fail(message: &str, json: bool) -> ! {
    if json {
        eprintln!("{}", serde_json::json!({ "error": message }));
    } else {
        eprintln!("{message}");
    }
    exit(1);
}

/// Report an activation outcome after the terminal is restored. Success is
/// silent so the TUI exits cleanly; failure yields an actionable
/// diagnostic for stderr instead of panicking.
/// Logs to the state directory, and only when `RUST_LOG` asks for it, so
/// running contx never leaves a log file behind in the working directory.
fn init_logging() {
    if env::var_os("RUST_LOG").is_none() {
        return;
    }
    let Some(path) = log_file_path(&|name| env::var_os(name)) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        Builder::from_default_env()
            .target(Target::Pipe(Box::new(file)))
            .init();
    }
}

/// `$XDG_STATE_HOME/contx/app.log`, falling back to
/// `~/.local/state/contx/app.log`. Relative values are ignored, as the XDG
/// base directory spec requires.
fn log_file_path(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name: &str| {
        env(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let state = absolute("XDG_STATE_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".local/state")))?;
    Some(state.join("contx").join("app.log"))
}

fn report_activation(
    result: Result<ActivateResult, ActivateError>,
) -> Option<String> {
    result.err().map(|e| e.to_string())
}
