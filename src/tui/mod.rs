mod message;
mod search;
mod sessions_list;

use crate::{
    config::Config, theme::Theme, tui::sessions_list::SessionsListState,
};
use message::Message;
use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{StatefulWidget, Widget},
};
use search::Search;
use sessions_list::SessionsList;
use std::io;
use terminal_colorsaurus::ThemeMode;

pub struct Tui {
    exit: bool,
    sessions_list: SessionsList,
    sessions_list_state: SessionsListState,
    search: Search,
    theme_mode: ThemeMode,
}

impl Tui {
    pub fn new(paths: &[&str], theme_mode: ThemeMode) -> Self {
        Tui {
            exit: false,
            search: Search::default(),
            sessions_list: SessionsList::new(paths),
            sessions_list_state: SessionsListState::new(3, theme_mode),
            theme_mode: theme_mode,
        }
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let mes: Option<Message> = self.search.handle_events()?;
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
            Message::NextSession
            | Message::PrevSession
            | Message::FirstSession
            | Message::LastSession
            | Message::SelectSession => self.sessions_list.handle_message(m),
            Message::TmuxError(e) => {
                panic!("tmux error: {e:?}");
            }
        };
        if next.is_some() {
            self.handle_message(next.unwrap());
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Widget::render(self, area, buf);
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal
                .draw(|frame| self.render(frame.area(), frame.buffer_mut()))?;
            self.handle_events()?;
        }
        Ok(())
    }
}

impl Widget for &mut Tui {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // ━search━━
        // ┏list━━━┓
        // ┃       ┃
        // ┃       ┃
        // ┗━━━━━━━┛

        let constraints = vec![Constraint::Length(1), Constraint::Fill(1)];

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let t = Theme::get(self.theme_mode);
        buf.set_style(area, Style::new().bg(t.bg).fg(t.fg));

        self.search.render(areas[0], buf, self.theme_mode);
        self.sessions_list
            .render(areas[1], buf, &mut self.sessions_list_state);
    }
}
