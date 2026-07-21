use crate::{config::Config, globals};
use log::debug;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Direction, Layout},
    widgets::{Block, List, ListState, Paragraph},
};
use std::env;
use std::io;

#[derive(Debug, Default)]
pub struct Tui {
    c: Config,
    selected_path: usize,
    exit: bool,
}

impl Tui {
    pub fn new(c: Config) -> Tui {
        Tui {
            c: c,
            selected_path: 0,
            exit: false,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn select_path(&mut self, count: isize) {
        let s = (self.selected_path as isize + count)
            .rem_euclid(self.c.paths.len() as isize);
        self.selected_path = s as usize;
        debug!("Tui.selected_path = {}", self.selected_path);
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                match key_event.code {
                    KeyCode::Char('q') => self.exit(),
                    KeyCode::Char('j') => self.select_path(1),
                    KeyCode::Char('k') => self.select_path(-1),
                    _ => {}
                }
            }
            _ => (),
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        // ┏━━━━━━━┓
        // ┃ list  ┃
        // ┃       ┃
        // ┃       ┃
        // ┃       ┃
        // ┗━━━━━━━┛
        // ┏━━━━━━━┓
        // ┃ debug ┃
        // ┗━━━━━━━┛

        let mut constraints = vec![Constraint::Fill(1)];
        if env::var("TUI_DEBUG").is_ok() {
            constraints = vec![Constraint::Fill(1), Constraint::Max(10)];
        }

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(frame.area());

        // A `Map` can be provided as the items to a ratatui `List` because it
        // implements `Iterator`, and its `Items` can be casted into
        // `ListItem`s, that are the elements from the ratatui `List`.

        // let items = self.tmux_sessions.iter().map(|s| s.name.as_str());
        let paths = self.c.paths.iter().map(|p| p.as_str());
        let mut state =
            ListState::default().with_selected(Some(self.selected_path));
        let list = List::new(paths)
            .block(Block::bordered())
            .highlight_symbol("");
        frame.render_stateful_widget(list, areas[0], &mut state);

        if env::var("TUI_DEBUG").is_ok() {
            let logs = match std::fs::read_to_string(globals::LOG_FILE) {
                Ok(content) => content,
                Err(e) => format!("{e}"),
            };
            let lines = logs.split("\n").count();
            let vert_scroll = lines as u16 - areas[1].height;
            let debug = Paragraph::new(logs).scroll((vert_scroll, 0));
            frame.render_widget(debug, areas[1]);
        }
    }
}
