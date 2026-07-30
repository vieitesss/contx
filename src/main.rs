mod config;
mod fuzzy;
mod theme;
mod tmux;
mod tui;
mod utils;

use env_logger::{Builder, Target};
use std::fs::OpenOptions;
use std::io;
use std::process::exit;
use terminal_colorsaurus::{QueryOptions, theme_mode};

use tui::Tui;

pub const LOG_FILE: &str = "app.log";
pub const DEFAULT_CONFIG_FILE: &str = "~/personal/contx/config.toml";

fn main() -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE)
        .unwrap();

    Builder::from_default_env()
        .target(Target::Pipe(Box::new(file)))
        .init();

    let config_path = DEFAULT_CONFIG_FILE;
    let theme_mode = theme_mode(QueryOptions::default()).unwrap();

    match config::parse(config_path) {
        Ok(c) => {
            ratatui::run(|terminal| Tui::new(c, theme_mode).run(terminal))?;
        }
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    }

    Ok(())
}
