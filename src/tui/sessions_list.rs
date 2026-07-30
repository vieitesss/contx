use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};
use terminal_colorsaurus::ThemeMode;

use crate::{fuzzy, theme::Theme, tmux, tui::message::Message, utils};

pub const NORMAL_STYLE: Style = Style::new();

#[derive(Default, Debug, Clone)]
pub struct SessionsList {
    pub paths: Vec<String>,
    matches: Vec<fuzzy::Match>,
    filtering: String,
    selected_line: usize, // 1-indexed
}

impl SessionsList {
    pub fn new(paths: &[&str]) -> Self {
        let ps: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        Self {
            paths: ps,
            matches: fuzzy::search(paths, ""),
            filtering: String::new(),
            selected_line: 1,
        }
    }

    pub fn handle_message(&mut self, m: Message) -> Option<Message> {
        match m {
            Message::FilterSessions(s) => {
                let ps: Vec<_> =
                    self.paths.iter().map(String::as_str).collect();
                self.matches = fuzzy::search(&ps, &s);
                self.filtering = s.to_string();

                let l = self.matches.len();
                if l < self.selected_line && l > 0 {
                    self.selected_line = l;
                }
            }
            Message::NextSession => {
                if self.selected_line < self.matches.len() {
                    self.selected_line += 1;
                }
            }
            Message::PrevSession => {
                if self.selected_line > 1 {
                    self.selected_line -= 1;
                }
            }
            Message::FirstSession => {
                self.selected_line = 1;
            }
            Message::LastSession => {
                let l = self.matches.len();
                if l > 0 {
                    self.selected_line = l;
                }
            }
            Message::SelectSession => {
                let entry = &self.matches[self.selected_line - 1].entry;
                let session_name = utils::path_to_tmux_session_name(entry);
                if let Err(e) = tmux::open(&session_name, entry) {
                    return Some(Message::TmuxError(e));
                } else {
                    return Some(Message::Exit);
                }
            }
            _ => {}
        };
        None
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

    pub fn format_line<'a>(
        &self,
        m: &'a fuzzy::Match,
        selected: bool,
        theme_mode: ThemeMode,
    ) -> Line<'a> {
        let mi_len = m.match_indices.len();
        let mut line = Line::default();
        let theme = Theme::get(theme_mode);

        if selected {
            line = line.style(Style::new().bg(theme.bg_alt));
        }

        if mi_len == 0 {
            line.push_span(Span::from(&m.entry));
            return line;
        }

        let hl_segments = SessionsList::get_hl_segments(&m.match_indices);

        let mut normal_start = 0;
        let hl = Style::new().fg(theme.accent);
        for i in hl_segments.iter() {
            line.push_span(
                Span::from(&m.entry[normal_start..i.0]).style(NORMAL_STYLE),
            );
            line.push_span(Span::from(&m.entry[i.0..=i.1]).style(hl));
            normal_start = i.1 + 1;
        }
        line.push_span(
            Span::from(&m.entry[normal_start..]).style(NORMAL_STYLE),
        );

        line
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, theme_mode: ThemeMode) {
        let lines: Vec<Line> = self
            .matches
            .iter()
            .enumerate()
            .map(|(i, m)| {
                self.format_line(m, i + 1 == self.selected_line, theme_mode)
            })
            .collect();

        Paragraph::new(lines)
            .block(Block::bordered())
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {

    use super::SessionsList;
    use crate::{fuzzy, theme::Theme};
    use ratatui::{
        style::Style,
        text::{Line, Span},
    };
    use terminal_colorsaurus::ThemeMode;

    const THEME_MODE: ThemeMode = ThemeMode::Light;
    const HL: Style = Style::new().fg(Theme::LIGHT.accent);

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
        let l = sl.format_line(&m, false, THEME_MODE);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
                Span::from("z").style(HL),
                Span::from("erobrew").style(super::NORMAL_STYLE),
            ]),
            l
        );

        let m = fuzzy::Match {
            entry: String::from("/user/ze"),
            match_indices: vec![7],
        };
        let l = sl.format_line(&m, false, THEME_MODE);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/z").style(super::NORMAL_STYLE),
                Span::from("e").style(HL),
                Span::default(),
            ]),
            l
        );

        let m = fuzzy::Match {
            entry: String::from("/user/ze"),
            match_indices: vec![0],
        };
        let l = sl.format_line(&m, false, THEME_MODE);
        assert_eq!(
            Line::from(vec![
                Span::default(),
                Span::from("/").style(HL),
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
        let l = sl.format_line(&m, false, THEME_MODE);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
                Span::from("ze").style(HL),
                Span::from("robrew").style(super::NORMAL_STYLE),
            ]),
            l
        );

        let m = fuzzy::Match {
            entry: String::from("/user/vieites/opt/zerobrew"),
            match_indices: vec![18, 24],
        };
        let l = sl.format_line(&m, false, THEME_MODE);
        assert_eq!(
            Line::from(vec![
                Span::from("/user/vieites/opt/").style(super::NORMAL_STYLE),
                Span::from("z").style(HL),
                Span::from("erobr").style(super::NORMAL_STYLE),
                Span::from("e").style(HL),
                Span::from("w").style(super::NORMAL_STYLE),
            ]),
            l
        );
    }
}
