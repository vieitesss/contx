mod config;
mod fuzzy;
mod theme;
mod tmux;
mod tui;
mod utils;

use env_logger::{Builder, Target};
use std::{fs::OpenOptions, io, process::exit};
use terminal_colorsaurus::{QueryOptions, ThemeMode, theme_mode};

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

    match config::parse() {
        Ok(c) => {
            let paths = c.paths.unwrap_or_default();
            ratatui::run(|terminal| Tui::new(&paths, theme_mode).run(terminal))?;
        }
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    }

    Ok(())
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
