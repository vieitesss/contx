use std::io;

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

use crate::tui::{SessionsList, DebugPane};
use super::message::Message;

pub enum Component {
    Sessions(SessionsList),
    Debug(DebugPane)
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum ComponentKind {
    #[default]
    Sessions,
    Debug,
}

impl Component {
    pub fn kind(&self) -> ComponentKind {
        match self {
            Component::Sessions(_) => ComponentKind::Sessions,
            Component::Debug(_) => ComponentKind::Debug,
        }
    }

    pub fn handle_events(&mut self) -> Result<Message, io::Error> {
        match self {
            Component::Sessions(c) => c.handle_events(),
            Component::Debug(_) => Ok(Message::NAM),
        }
    }
}

impl Widget for &mut Component {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self {
            Component::Sessions(c) => c.render(area, buf),
            Component::Debug(c) => c.render(area, buf),
        };
    }
}
