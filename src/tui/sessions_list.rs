use super::message::Message;
use crate::fuzzy;
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

    fn get_hl_segments(indexes: &[usize]) -> Vec<(usize, usize)> {
        let mut segments: Vec<(usize, usize)> = vec![];
        let mut seg: (usize, usize) = (indexes[0], indexes[0]);
        for &idx in indexes[1..].iter() {
            if idx == seg.1 + 1 {
                seg.1 = idx;
                continue;
            } else {
                segments.push(seg);
                seg = (idx, idx);
            }
        }
        segments.push(seg);

        segments
    }

    pub fn format_line<'a>(&self, m: &'a fuzzy::Match) -> Line<'a> {
        let mi_len = m.match_indices.len();
        if mi_len == 0 {
            return Line::from(Span::from(&m.entry));
        }

        let hl_segments = SessionsList::get_hl_segments(&m.match_indices);

        let mut spans = vec![];
        let mut normal_start = 0;
        for i in hl_segments.iter() {
            spans.push(
                Span::from(&m.entry[normal_start..i.0]).style(NORMAL_STYLE),
            );
            spans.push(Span::from(&m.entry[i.0..=i.1]).style(HL_STYLE));
            normal_start = i.1 + 1;
        }
        spans.push(Span::from(&m.entry[normal_start..]).style(NORMAL_STYLE));

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
    fn segments() {
        let idxs: Vec<usize> = vec![0, 1, 3, 5, 6, 7, 9, 11];
        let seg = SessionsList::get_hl_segments(&idxs);
        assert_eq![vec![(0, 1), (3, 3), (5, 7), (9, 9), (11, 11)], seg];

        let idxs: Vec<usize> = vec![2, 3, 4, 5, 6, 7, 9, 10];
        let seg = SessionsList::get_hl_segments(&idxs);
        assert_eq![vec![(2, 7), (9, 10)], seg];

        let idxs: Vec<usize> = vec![3, 6, 9];
        let seg = SessionsList::get_hl_segments(&idxs);
        assert_eq![vec![(3, 3), (6, 6), (9, 9)], seg];
    }

    #[test]
    fn formatting_1() {
        let m = fuzzy::Match {
            entry: String::from("/user/vieites/opt/zerobrew"),
            match_indices: vec![18],
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
            match_indices: vec![7],
        };
        let l = sl.format_line(&m);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/z").style(super::NORMAL_STYLE),
                Span::from("e").style(super::HL_STYLE),
                Span::default(),
            ]),
            l
        );

        let m = fuzzy::Match {
            entry: String::from("/user/ze"),
            match_indices: vec![0],
        };
        let l = sl.format_line(&m);
        assert_eq!(
            Line::from(vec![
                Span::default(),
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
            match_indices: vec![18, 19],
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
            match_indices: vec![18, 24],
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
