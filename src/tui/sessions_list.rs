use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph, StatefulWidget, Widget},
};
use terminal_colorsaurus::ThemeMode;

use crate::{fuzzy, theme::Theme, tmux, tui::message::Message, utils};

pub const NORMAL_STYLE: Style = Style::new();

#[derive(Default, Debug, Clone)]
pub enum SelectionDirection {
    Up,
    #[default]
    Down,
}

#[derive(Default, Debug, Clone)]
pub struct SessionsList {
    pub paths: Vec<String>,
    matches: Vec<fuzzy::Match>,
    filtering: String,
    selected_line: usize, // 1-indexed
    selection_dir: SelectionDirection,
}

impl SessionsList {
    pub fn new(paths: &[String]) -> Self {
        let ps: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        Self {
            paths: ps,
            matches: fuzzy::search(paths, ""),
            filtering: String::new(),
            selected_line: 1,
            selection_dir: SelectionDirection::default(),
        }
    }

    pub fn handle_message(&mut self, m: Message) -> Option<Message> {
        match m {
            Message::FilterSessions(s) => {
                self.matches = fuzzy::search(&self.paths, &s);
                self.filtering = s.to_string();

                let l = self.matches.len();
                if l < self.selected_line && l > 0 {
                    self.selected_line = l;
                    self.selection_dir = SelectionDirection::Up;
                }
            }
            Message::NextSession => {
                if self.selected_line < self.matches.len() {
                    self.selected_line += 1;
                    self.selection_dir = SelectionDirection::Down;
                }
            }
            Message::PrevSession => {
                if self.selected_line > 1 {
                    self.selected_line -= 1;
                    self.selection_dir = SelectionDirection::Up;
                }
            }
            Message::FirstSession => {
                self.selected_line = 1;
                self.selection_dir = SelectionDirection::Up;
            }
            Message::LastSession => {
                let l = self.matches.len();
                if l > 0 {
                    self.selected_line = l;
                    self.selection_dir = SelectionDirection::Down;
                }
            }
            Message::SelectSession => {
                if self.matches.is_empty() {
                    return None;
                }
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
}

pub struct SessionsListState {
    pub scroll_y: u16,
    pub scroll_offset: u16,
    pub theme_mode: ThemeMode,
}

impl SessionsListState {
    pub fn new(scroll_offset: u16, theme_mode: ThemeMode) -> Self {
        Self {
            scroll_y: 0,
            scroll_offset: scroll_offset,
            theme_mode: theme_mode,
        }
    }
    pub fn update_scroll(
        &mut self,
        height: u16,
        matches_len: u16,
        selected_line: u16,
    ) {
        let visible_lines = height.saturating_sub(2);
        if visible_lines == 0 {
            return;
        }

        let matches_len = matches_len;
        // never bigger than the window
        let offset =
            self.scroll_offset.min(visible_lines.saturating_sub(1) / 2);
        let max_scroll = matches_len.saturating_sub(visible_lines);

        if selected_line < self.scroll_y + 1 + offset {
            self.scroll_y = selected_line.saturating_sub(offset + 1);
        } else if selected_line
            > self.scroll_y + visible_lines.saturating_sub(offset)
        {
            self.scroll_y =
                selected_line - visible_lines.saturating_sub(offset);
        }
        self.scroll_y = self.scroll_y.min(max_scroll);
    }
}

impl StatefulWidget for &SessionsList {
    type State = SessionsListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let m = self.matches.clone();
        let lines: Vec<Line> = m
            .iter()
            .enumerate()
            .map(|(i, m)| {
                self.format_line(
                    m,
                    i + 1 == self.selected_line,
                    state.theme_mode,
                )
            })
            .collect();

        state.update_scroll(
            area.height,
            self.matches.len() as u16,
            self.selected_line as u16,
        );

        Paragraph::new(lines)
            .block(Block::bordered())
            .scroll((state.scroll_y, 0))
            .render(area, buf);
    }
}

#[cfg(test)]
#[path = "sessions_list_tests.rs"]
mod tests;
