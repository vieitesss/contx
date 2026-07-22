use std::io;

use ratatui::{
    buffer::Buffer,
    crossterm::event::KeyCode,
    layout::Rect,
    widgets::{Block, List, ListState, StatefulWidget, Widget},
};

#[derive(Default, Debug, Clone)]
pub struct SessionsList {
    pub paths: Vec<String>,
    state: ListState,
}

pub fn normalize_paths(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| normalize_path(p)).collect()
}

fn normalize_path(path: &str) -> String {
    String::from(path)
}

impl SessionsList {
    pub fn new(paths: &[&str], selected: Option<usize>) -> Self {
        let state = ListState::default().with_selected(selected);

        Self {
            paths: normalize_paths(paths),
            state: state,
        }
    }

    pub fn handle_events(&mut self, keycode: KeyCode) -> io::Result<()> {
        match keycode {
            KeyCode::Char('j') => self.state.select_next(),
            KeyCode::Char('k') => self.state.select_previous(),
            _ => {}
        }
        Ok(())
    }
}

impl Widget for &mut SessionsList {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let list = List::new(self.paths.iter().map(String::as_str))
            .block(Block::bordered())
            .highlight_symbol("");

        StatefulWidget::render(list, area, buf, &mut self.state);
    }
}
