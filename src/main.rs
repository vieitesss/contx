mod config;
mod fuzzy;
mod theme;
mod tmux;
mod tui;
mod utils;

use env_logger::{Builder, Target};
use std::{fs::OpenOptions, io, process::exit};
use terminal_colorsaurus::{QueryOptions, theme_mode};

use tui::Tui;

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

    let theme_mode = theme_mode(QueryOptions::default()).unwrap();

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
