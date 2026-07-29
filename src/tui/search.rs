use crate::{theme::Theme, tui::message::Message};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Widget,
};
use std::io;
use terminal_colorsaurus::ThemeMode;

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
                        }
                        if key.code == KeyCode::Char('c') {
                            return Ok(Some(Message::Exit));
                        }
                        if key.code == KeyCode::Char('j') {
                            return Ok(Some(Message::NextSession));
                        }
                        if key.code == KeyCode::Char('k') {
                            return Ok(Some(Message::PrevSession));
                        }
                        if key.code == KeyCode::Char('g')
                            || key.code == KeyCode::Char('b')
                        {
                            return Ok(Some(Message::LastSession));
                        }
                        if key.code == KeyCode::Char('t') {
                            return Ok(Some(Message::FirstSession));
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
                    KeyCode::Down => {
                        return Ok(Some(Message::NextSession));
                    }
                    KeyCode::Up => {
                        return Ok(Some(Message::PrevSession));
                    }
                    KeyCode::End => {
                        return Ok(Some(Message::LastSession));
                    }
                    KeyCode::Home => {
                        return Ok(Some(Message::FirstSession));
                    }
                    _ => {}
                }
                return Ok(self.send_filter());
            }
        }
        Ok(None)
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, theme_mode: ThemeMode) {
        Line::from(vec![
            Span::from("> "),
            Span::from(&self.text),
            Span::styled("█", Style::new().fg(Theme::get(theme_mode).accent)),
        ])
        .render(area, buf);
    }
}
