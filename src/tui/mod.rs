mod debug_pane;
mod message;
mod search;
mod sessions_list;

use crate::config::Config;
use debug_pane::DebugPane;
use message::Message;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
};
use search::Search;
use sessions_list::SessionsList;
use std::env;
use std::io;

#[derive(Default)]
enum Focus {
    SessionsList,
    DebugPane,
    #[default]
    Search,
}

#[derive(Default)]
pub struct Tui {
    exit: bool,
    sessions_list: SessionsList,
    debug_pane: DebugPane,
    search: Search,
    focused: Focus,
}

impl Tui {
    pub fn new(c: Config) -> Self {
        let paths: Vec<&str> = c.paths.iter().map(String::as_str).collect();

        Tui {
            exit: false,
            search: Search::default(),
            sessions_list: SessionsList::new(&paths),
            debug_pane: DebugPane::new(crate::LOG_FILE),
            focused: Focus::default(),
        }
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let mes: Option<Message> = match self.focused {
            Focus::SessionsList => self.sessions_list.handle_events()?,
            Focus::Search => self.search.handle_events()?,
            Focus::DebugPane => None,
        };
        if mes.is_some() {
            self.handle_message(mes.unwrap())
        }
        Ok(())
    }

    fn handle_message(&mut self, m: Message) {
        let next = match m {
            Message::Exit => {
                self.exit();
                return;
            }
            Message::FilterSessions(_) => self.sessions_list.handle_message(m),
        };
        if next.is_some() {
            self.handle_message(next.unwrap());
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn render(&mut self, frame: &mut Frame) {
        // ━search━━
        // ┏list━━━┓
        // ┃       ┃
        // ┃       ┃
        // ┗━━━━━━━┛
        // ┏debug━━┓
        // ┗━━━━━━━┛

        let mut constraints = vec![Constraint::Length(1), Constraint::Fill(1)];
        if env::var("TUI_DEBUG").is_ok() {
            constraints.push(Constraint::Max(10));
        }

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(frame.area());

        frame.render_widget(&self.search, areas[0]);
        frame.render_widget(&self.sessions_list, areas[1]);
        if env::var("TUI_DEBUG").is_ok() {
            frame.render_widget(&self.debug_pane, areas[2]);
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }
}
