/// UTF-8 text field with a character-index cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Field {
    text: String,
    cursor: usize,
}

impl Field {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn insert(&mut self, c: char) {
        let mut chars: Vec<char> = self.text.chars().collect();
        let i = self.cursor.min(chars.len());
        chars.insert(i, c);
        self.text = chars.into_iter().collect();
        self.cursor = i + 1;
    }

    pub(crate) fn backspace(&mut self) {
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.cursor == 0 || chars.is_empty() {
            return;
        }
        let i = self.cursor - 1;
        chars.remove(i);
        self.text = chars.into_iter().collect();
        self.cursor = i;
    }

    pub(crate) fn delete_previous_word(&mut self) {
        let mut chars: Vec<char> = self.text.chars().collect();
        let cursor = self.cursor.min(chars.len());
        let mut start = cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        if start != cursor {
            chars.drain(start..cursor);
            self.text = chars.into_iter().collect();
            self.cursor = start;
        }
    }

    pub(crate) fn delete(&mut self) {
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.cursor >= chars.len() {
            return;
        }
        chars.remove(self.cursor);
        self.text = chars.into_iter().collect();
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_right(&mut self) {
        let len = self.text.chars().count();
        if self.cursor < len {
            self.cursor += 1;
        }
    }

    pub(crate) fn home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    pub(crate) fn set_str(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.text.chars().count();
    }

    fn clamp(&mut self) {
        let len = self.text.chars().count();
        if self.cursor > len {
            self.cursor = len;
        }
    }
}

impl Field {
    pub(crate) fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
        self.clamp();
    }
}
