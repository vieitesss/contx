use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

use crate::fuzzy;

/// Deliberate user intent produced by key input. Selecting produces intent;
/// tmux activation stays outside this module.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    None,
    Exit,
    Activate(String),
    Clone,
    NewDir,
    Delete(String),
}

/// Session-selection workflow: key input, query editing, ranked matches,
/// selected-row rules, and user intent live in one place. The interface is
/// the test seam: feed key events, observe the query, ranked search
/// results, selected row, and intent. Rendering reads the observables;
/// the TUI acts on the intent.
#[derive(Debug, Clone)]
pub struct Selection {
    candidates: Vec<String>,
    query: String,
    matches: Vec<fuzzy::Match>,
    /// 1-indexed list index. Kept across refilter and reorder;
    /// clamped to the last match when nonempty results shrink;
    /// left unchanged while results are empty.
    selected_line: usize,
}

impl Selection {
    pub fn new(candidates: &[String]) -> Self {
        let matches = fuzzy::search(candidates, "");
        Self {
            candidates: candidates.to_vec(),
            query: String::new(),
            matches,
            selected_line: 1,
        }
    }

    /// Observable query text for the search prompt.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Ranked search results with UTF-8-safe match presentation.
    pub fn matches(&self) -> &[fuzzy::Match] {
        &self.matches
    }

    /// Original candidate catalog order, before fuzzy ranking.
    pub(crate) fn candidates(&self) -> &[String] {
        &self.candidates
    }

    /// Whether any session candidates exist, regardless of the
    /// current filter. Rendering uses this to tell an empty catalog
    /// apart from a filter miss; the query alone cannot, since an
    /// empty query on an empty catalog shows no matches either.
    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    /// Selected numeric row, 1-indexed.
    pub fn selected_line(&self) -> usize {
        self.selected_line
    }

    /// Apply one key press. Only key presses act; anything else is a no-op.
    /// Listed Ctrl chords act; any other Ctrl or Alt chord does nothing, so
    /// accidental chords cannot rewrite the query or selection.
    pub fn handle_key(&mut self, key: KeyEvent) -> Intent {
        if key.kind != KeyEventKind::Press {
            return Intent::None;
        }
        let modifiers = key.modifiers;
        if modifiers.contains(KeyModifiers::ALT) {
            if modifiers == KeyModifiers::ALT && key.code == KeyCode::Backspace
            {
                self.remove_word();
                self.refilter();
            }
            return Intent::None;
        }
        if modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('w') => {
                    self.remove_word();
                    self.refilter();
                }
                KeyCode::Char('c') => return Intent::Exit,
                KeyCode::Char('j') => self.move_sel(1),
                KeyCode::Char('k') => self.move_sel(-1),
                KeyCode::Char('g') | KeyCode::Char('b') => self.last(),
                KeyCode::Char('t') => self.first(),
                _ => {}
            }
            return Intent::None;
        }
        match key.code {
            KeyCode::Char(c) => {
                self.query.push(c);
                self.refilter();
            }
            KeyCode::Backspace => {
                let _ = self.query.pop();
                self.refilter();
            }
            KeyCode::Down => self.move_sel(1),
            KeyCode::Up => self.move_sel(-1),
            // Single-column list: horizontal keys stay no-ops.
            KeyCode::Left | KeyCode::Right => {}
            KeyCode::Home => self.first(),
            KeyCode::End => self.last(),
            KeyCode::Enter => {
                if let Some(m) =
                    self.matches.get(self.selected_line.saturating_sub(1))
                {
                    return Intent::Activate(m.entry.clone());
                }
            }
            _ => {}
        }
        Intent::None
    }

    fn remove_word(&mut self) {
        match self.query.rfind(' ') {
            Some(i) => self.query.truncate(i + 1),
            None => self.query.clear(),
        }
    }

    fn refilter(&mut self) {
        self.matches = fuzzy::search(&self.candidates, &self.query);
        if self.matches.is_empty() {
            return;
        }
        if self.selected_line > self.matches.len() {
            self.selected_line = self.matches.len();
        }
    }

    fn select_pos(&mut self, pos: usize) {
        if pos < self.matches.len() {
            self.selected_line = pos + 1;
        }
    }

    /// Select the match carrying `entry`, if present. The TUI
    /// walks grouped children in visual order through this, so
    /// headers and blanks — which never enter matches — can
    /// never be selected here either. Unknown entries leave the
    /// selection unchanged, like a move past either end.
    pub(crate) fn select_entry(&mut self, entry: &str) {
        if let Some(pos) = self.matches.iter().position(|m| m.entry == entry) {
            self.selected_line = pos + 1;
        }
    }

    /// Replace the catalog, keeping the query and refiltering.
    pub(crate) fn replace_candidates(&mut self, candidates: &[String]) {
        self.candidates = candidates.to_vec();
        self.refilter();
    }

    /// Linear move on the single-column list. No wrap: moving past
    /// either end leaves the selected row unchanged.
    fn move_sel(&mut self, dy: i32) {
        let n = self.matches.len();
        if n == 0 {
            return;
        }
        let next = self.selected_line as i32 + dy;
        if next >= 1 && (next as usize) <= n {
            self.selected_line = next as usize;
        }
    }

    fn first(&mut self) {
        if !self.matches.is_empty() {
            self.select_pos(0);
        }
    }

    fn last(&mut self) {
        if !self.matches.is_empty() {
            self.select_pos(self.matches.len() - 1);
        }
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
