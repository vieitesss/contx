use super::message::Message;
use crate::fuzzy;
use log::debug;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyEventKind},
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};
use std::io;

pub const NORMAL_STYLE: Style = Style::new();
pub const HL_STYLE: Style = Style::new().bg(Color::Yellow);

#[derive(Default, Debug, Clone)]
pub struct SessionsList {
    pub paths: Vec<String>,
    matches: Vec<fuzzy::Match>,
    filtering: String,
}

impl SessionsList {
    pub fn new(paths: &[&str]) -> Self {
        let ps: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        Self {
            paths: ps,
            matches: fuzzy::search(paths, ""),
            filtering: String::new(),
        }
    }

    pub fn handle_message(&mut self, m: Message) -> Option<Message> {
        match m {
            Message::FilterSessions(s) => {
                let ps: Vec<_> =
                    self.paths.iter().map(String::as_str).collect();
                self.matches = fuzzy::search(&ps, &s);
                self.filtering = s.to_string();
            }
            _ => {}
        };
        None
    }

    pub fn handle_events(&mut self) -> io::Result<Option<Message>> {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    _ => {}
                }
            }
        }
        Ok(None)
    }

    pub fn format_line<'a>(&self, m: &'a fuzzy::Match) -> Line<'a> {
        let mi_len = m.match_indexes.len();
        if mi_len == 0 {
            return Line::from(Span::from(&m.entry));
        }

        let mut spans: Vec<_> = vec![];
        let mut start = 0;
        let mut hl = m.match_indexes[0] == 0;
        let mut mi = 0;
        for (i, _) in m.entry.char_indices() {
            if m.match_indexes[mi] == i {
                if !hl {
                    // Save normal and start highlighting.
                    spans.push(
                        Span::from(&m.entry[start..i]).style(NORMAL_STYLE),
                    );
                    start = i;
                    hl = true;
                }
                mi += 1;
            } else {
                if hl {
                    spans.push(Span::from(&m.entry[start..i]).style(HL_STYLE));
                    start = i;
                    hl = false;
                }
            }
            if mi == mi_len {
                spans.push(Span::from(&m.entry[start..i + 1]).style(HL_STYLE));
                if i + 1 < m.entry.len() {
                    spans.push(
                        Span::from(&m.entry[i + 1..]).style(NORMAL_STYLE),
                    );
                }
                break;
            }
        }

        Line::from(spans)
    }
}

impl Widget for &SessionsList {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines: Vec<Line> =
            self.matches.iter().map(|m| self.format_line(m)).collect();

        Paragraph::new(lines)
            .block(Block::bordered())
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {

    use super::SessionsList;
    use crate::fuzzy;
    use ratatui::text::{Line, Span};

    #[test]
    fn formatting_1() {
        let m = fuzzy::Match {
            entry: String::from("/user/vieites/opt/zerobrew"),
            match_indexes: vec![18],
        };
        let sl = SessionsList::default();
        let l = sl.format_line(&m);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
                Span::from("z").style(super::HL_STYLE),
                Span::from("erobrew").style(super::NORMAL_STYLE),
            ]),
            l
        );

        let m = fuzzy::Match {
            entry: String::from("/user/ze"),
            match_indexes: vec![7],
        };
        let l = sl.format_line(&m);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/z").style(super::NORMAL_STYLE),
                Span::from("e").style(super::HL_STYLE),
            ]),
            l
        );

        let m = fuzzy::Match {
            entry: String::from("/user/ze"),
            match_indexes: vec![0],
        };
        let l = sl.format_line(&m);
        assert_eq!(
            Line::from(vec![
                Span::from("/").style(super::HL_STYLE),
                Span::from("user/ze").style(super::NORMAL_STYLE),
            ]),
            l
        );
    }

    #[test]
    fn formatting_2() {
        let m = fuzzy::Match {
            entry: String::from("/user/vieites/opt/zerobrew"),
            match_indexes: vec![18, 19],
        };

        let sl = SessionsList::default();
        let l = sl.format_line(&m);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
                Span::from("ze").style(super::HL_STYLE),
                Span::from("robrew").style(super::NORMAL_STYLE),
            ]),
            l
        );

        let m = fuzzy::Match {
            entry: String::from("/user/vieites/opt/zerobrew"),
            match_indexes: vec![18, 24],
        };
        let l = sl.format_line(&m);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
                Span::from("z").style(super::HL_STYLE),
                Span::from("erobr").style(super::NORMAL_STYLE),
                Span::from("e").style(super::HL_STYLE),
                Span::from("w").style(super::NORMAL_STYLE),
            ]),
            l
        );
    }
}
