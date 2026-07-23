use std::io;

use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::Rect,
    widgets::{Block, List, ListState, StatefulWidget},
};
use super::message::Message;

#[derive(Default, Debug, Clone)]
pub struct SessionsList {
    pub paths: Vec<String>,
    state: ListState,
}

impl SessionsList {
    pub fn new(paths: &[&str], selected: Option<usize>) -> Self {
        let state = ListState::default().with_selected(selected);
        let ps = paths.iter().map(|p| p.to_string()).collect();

        Self {
            paths: ps,
            state: state,
        }
    }

    pub fn handle_events(&mut self) -> Result<Message, io::Error> {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') => return Ok(Message::Exit),
                    KeyCode::Char('j') => self.state.select_next(),
                    KeyCode::Char('k') => self.state.select_previous(),
                    _ => {}
                }
            }
        }
        Ok(Message::NAM)
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let list = List::new(self.paths.iter().map(String::as_str))
            .block(Block::bordered())
            .highlight_symbol("");

        StatefulWidget::render(list, area, buf, &mut self.state);
    }
}
