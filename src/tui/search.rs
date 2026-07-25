use crate::tui::message::Message;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Widget,
};
use std::io;

const ARROW_STYLE: Style = Style::new().blue();

#[derive(Default)]
pub struct Search {
    text: String,
}

impl Search {
    fn remove_word(&mut self) {
        match self.text.rfind(' ') {
            Some(i) => self.text.truncate(i + 1),
            None => self.text.clear(),
        }
    }

    fn send_filter(&self) -> Option<Message> {
        Some(Message::FilterSessions(self.text.clone()))
    }

    pub fn handle_events(&mut self) -> io::Result<Option<Message>> {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.modifiers {
                    KeyModifiers::ALT => {
                        if key.code == KeyCode::Backspace {
                            self.remove_word();
                            return Ok(self.send_filter());
                        }
                        return Ok(None);
                    }
                    KeyModifiers::CONTROL => {
                        if key.code == KeyCode::Char('w') {
                            self.remove_word();
                            return Ok(self.send_filter());
                        } else if key.code == KeyCode::Char('c') {
                            return Ok(Some(Message::Exit));
                        }
                    }
                    _ => {}
                }
                match key.code {
                    KeyCode::Char(c) => {
                        self.text.push(c);
                    }
                    KeyCode::Backspace => {
                        let _ = self.text.pop();
                    }
                    _ => {}
                }
                return Ok(self.send_filter());
            }
        }
        Ok(None)
    }
}

impl Widget for &Search {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Line::from(vec![
            Span::styled("> ", ARROW_STYLE),
            Span::from(&self.text),
            Span::from("█"),
        ])
        .render(area, buf);
    }
}
