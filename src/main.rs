mod config;
mod fuzzy;
mod herdr;
mod mux;
mod theme;
mod tmux;
mod tui;
mod utils;

use env_logger::{Builder, Target};
use std::{fs::OpenOptions, io, process::exit};
use terminal_colorsaurus::{QueryOptions, ThemeMode, theme_mode};

use config::Multiplexer;
use mux::{ActivateError, ActivateResult};
use tui::Tui;

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

pub const LOG_FILE: &str = "app.log";

fn main() -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE)
        .unwrap();

    Builder::from_default_env()
        .target(Target::Pipe(Box::new(file)))
        .init();

    let theme_mode = theme_mode_or_light(theme_mode(QueryOptions::default()));

    match config::resolve() {
        Ok(config::Startup::Help) => {
            print!("{}", config::USAGE);
        }
        Ok(config::Startup::Ready(resolved)) => {
            let selected = ratatui::run(|terminal| {
                Tui::new(&resolved.candidates, theme_mode).run(terminal)
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
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    }

    Ok(())
}

/// Report an activation outcome after the terminal is restored. Success is
/// silent so the TUI exits cleanly; failure yields an actionable
/// diagnostic for stderr instead of panicking.
fn report_activation(
    result: Result<ActivateResult, ActivateError>,
) -> Option<String> {
    result.err().map(|e| e.to_string())
}

/// Falls back to the light theme (the only one implemented) when the
/// terminal's theme mode cannot be queried, e.g. when there is no
/// usable terminal device.
fn theme_mode_or_light(
    result: Result<ThemeMode, terminal_colorsaurus::Error>,
) -> ThemeMode {
    result.unwrap_or_else(|e| {
        log::warn!(
            "failed to detect terminal theme mode: {e}; using light theme"
        );
        ThemeMode::Light
    })
}
