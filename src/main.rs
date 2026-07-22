mod config;
mod globals;
mod tmux;
mod tui;

use env_logger::{Builder, Target};
use std::fs::OpenOptions;
use std::io;
use std::process::exit;
use tui::Tui;

fn main() -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(globals::LOG_FILE)
        .unwrap();

    Builder::from_default_env()
        .target(Target::Pipe(Box::new(file)))
        .init();

    let config_path = globals::DEFAULT_CONFIG_FILE;
    match config::parse(config_path) {
        Ok(c) => {
            ratatui::run(|terminal| Tui::new(c).run(terminal))?;
        }
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    }

    Ok(())
}
