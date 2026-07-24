use super::message::Message;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyEventKind},
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};
use std::io;

const LINE_HL: Style = Style::new().bg(Color::Yellow);

#[derive(Default, Debug, Clone)]
pub struct SessionsList {
    pub paths: Vec<String>,
    filtered_indices: Vec<usize>,
    filtering: String,
}

impl SessionsList {
    pub fn new(paths: &[&str]) -> Self {
        let ps: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        let len = ps.len();
        Self {
            paths: ps,
            filtered_indices: (0..len).collect(),
            filtering: String::new(),
        }
    }

    pub fn handle_message(&mut self, m: Message) -> Option<Message> {
        match m {
            Message::FilterSessions(s) => {
                self.filter(&s);
                self.filtering = s;
            }
            _ => {}
        };
        None
    }

    pub fn handle_events(&mut self) -> Result<Option<Message>, io::Error> {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    _ => {}
                }
            }
        }
        Ok(None)
    }

    fn filter(&mut self, s: &str) {
        let lower = &s.to_lowercase();
        self.filtered_indices = self
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| p.to_lowercase().contains(lower))
            .map(|(i, _)| i)
            .collect()
    }

    fn format_line<'a>(&self, line: &'a str) -> Line<'a> {
        let filter_len = self.filtering.len();
        if filter_len == 0 {
            return Line::from(line);
        }
        if let Some(hl_start) = line.to_lowercase().find(&self.filtering) {
            let hl_end = filter_len + hl_start;
            Line::from(vec![
                Span::from(line[..hl_start].to_string()),
                Span::from(line[hl_start..hl_end].to_string()).style(LINE_HL),
                Span::from(line[hl_end..].to_string()),
            ])
        } else {
            Line::from(line)
        }
    }
}

impl Widget for &SessionsList {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines: Vec<Line> = self
            .filtered_indices
            .iter()
            .map(|&i| self.format_line(&self.paths[i]))
            .collect();

        Paragraph::new(lines)
            .block(Block::bordered())
            .render(area, buf);
    }
}
