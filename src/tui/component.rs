use std::io;

use super::message::Message;
use crate::tui::{DebugPane, Search, SessionsList};
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

pub enum Component {
    Search(Search),
    Sessions(SessionsList),
    Debug(DebugPane),
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum ComponentKind {
    #[default]
    Search,
    Sessions,
    Debug,
}

impl Component {
    pub fn kind(&self) -> ComponentKind {
        match self {
            Component::Sessions(_) => ComponentKind::Sessions,
            Component::Debug(_) => ComponentKind::Debug,
            Component::Search(_) => ComponentKind::Search,
        }
    }

    pub fn handle_events(&mut self) -> Result<Message, io::Error> {
        match self {
            Component::Search(c) => c.handle_events(),
            Component::Sessions(c) => c.handle_events(),
            Component::Debug(_) => Ok(Message::NAM),
        }
    }
}

impl Widget for &mut Component {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self {
            Component::Search(c) => c.render(area, buf),
            Component::Sessions(c) => c.render(area, buf),
            Component::Debug(c) => c.render(area, buf),
        };
    }
}
