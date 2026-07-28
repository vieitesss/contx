mod debug_pane;
mod message;
mod search;
mod sessions_list;

use crate::{config::Config, theme::Theme};
use debug_pane::DebugPane;
use message::Message;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::Widget,
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
    theme: Theme,
}

impl Tui {
    pub fn new(c: Config, theme: Theme) -> Self {
        let paths: Vec<&str> = c.paths.iter().map(String::as_str).collect();

        Tui {
            exit: false,
            search: Search::default(),
            sessions_list: SessionsList::new(&paths),
            debug_pane: DebugPane::new(crate::LOG_FILE),
            focused: Focus::default(),
            theme: theme,
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

    fn render(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }
}

impl Widget for &Tui {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // ━search━━
        // ┏list━━━┓
        // ┃       ┃
        // ┃       ┃
        // ┗━━━━━━━┛
        // ┏debug━━┓
        // ┗━━━━━━━┛

        let mut constraints =
            vec![Constraint::Length(1), Constraint::Percentage(20)];
        if env::var("TUI_DEBUG").is_ok() {
            constraints.push(Constraint::Fill(1));
        }

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        buf.set_style(area, Style::new().bg(self.theme.bg).fg(self.theme.fg));

        self.search.render(areas[0], buf);
        self.sessions_list.render(areas[1], buf);
        if env::var("TUI_DEBUG").is_ok() {
            self.debug_pane.render(areas[2], buf);
        }
    }
}
