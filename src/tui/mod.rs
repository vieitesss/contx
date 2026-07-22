mod debug_pane;
mod sessions_list;

use crate::{config::Config, globals};
use debug_pane::DebugPane;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Direction, Layout},
};
use sessions_list::SessionsList;
use std::env;
use std::io;

#[derive(Debug, Default)]
enum Focus {
    #[default]
    Sessions,
}

#[derive(Default)]
pub struct Tui {
    exit: bool,
    sessions: SessionsList,
    debug_pane: DebugPane,
    focus: Focus,
}

impl Tui {
    pub fn new(c: Config) -> Self {
        let paths: Vec<&str> = c.paths.iter().map(String::as_str).collect();
        let selected: Option<usize> = Some(0);

        Tui {
            exit: false,
            sessions: SessionsList::new(&paths, selected),
            focus: Focus::default(),
            debug_pane: DebugPane::new(globals::LOG_FILE),
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

    fn handle_events(&mut self) -> io::Result<()> {
        let keypress_code: Option<KeyCode> = match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                Some(key_event.code)
            }
            _ => None,
        };
        if keypress_code.is_none() {
            return Ok(());
        }

        let keycode = keypress_code.unwrap();

        match keycode {
            KeyCode::Char('q') => self.exit(),
            _ => {}
        }

        match self.focus {
            Focus::Sessions => self.sessions.handle_events(keycode),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
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

        frame.render_widget(&mut self.sessions, areas[0]);

        if env::var("TUI_DEBUG").is_ok() {
            frame.render_widget(&self.debug_pane, areas[1]);
        }
    }
}
