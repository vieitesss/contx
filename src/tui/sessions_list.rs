use super::message::Message;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::Rect,
    text::Line,
    widgets::{Block, Paragraph, Widget},
};
use std::io;

#[derive(Default, Debug, Clone)]
pub struct SessionsList {
    pub paths: Vec<String>,
    filtered_indices: Vec<usize>,
    filter: String,
}

impl SessionsList {
    pub fn new(paths: &[&str]) -> Self {
        let ps: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        Self {
            paths: ps,
            filtered_indices: vec![],
            filter: String::new(),
        }
    }

    pub fn handle_message(&mut self, m: Message) -> Option<Message> {
        match m {
            Message::FilterSessions(s) => self.filter(&s),
            _ => {}
        };
        None
    }

    pub fn handle_events(&mut self) -> Result<Option<Message>, io::Error> {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') => return Ok(Some(Message::Exit)),
                    _ => {}
                }
            }
        }
        Ok(None)
    }

    fn filter(&mut self, s: &str) {
        self.filtered_indices = self
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| p.contains(s))
            .map(|(i, _)| i)
            .collect()
    }

    fn format_line<'a>(&self, line: &'a str) -> Line<'a> {
        Line::from(line)
    }
}

impl Widget for &SessionsList {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines: Vec<Line> = if self.filtered_indices.len() > 0 {
            self.filtered_indices
                .iter()
                .map(|&i| self.format_line(&self.paths[i]))
                .collect()
        } else {
            self.paths.iter().map(|p| self.format_line(p)).collect()
        };

        Paragraph::new(lines)
            .block(Block::bordered())
            .render(area, buf);
    }
}
