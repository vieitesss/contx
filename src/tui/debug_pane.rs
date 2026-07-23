use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Widget},
};

#[derive(Default)]
pub struct DebugPane {
    log_file: String,
}

impl DebugPane {
    pub fn new(log_file: &str) -> Self {
        Self {
            log_file: log_file.to_string(),
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let logs = match std::fs::read_to_string(&self.log_file) {
            Ok(content) => content,
            Err(e) => format!("{e}"),
        };
        let lines = logs.split("\n").count();
        let vert_scroll = lines as u16 - area.height;
        Paragraph::new(logs)
            .scroll((vert_scroll, 0))
            .render(area, buf);
    }
}
