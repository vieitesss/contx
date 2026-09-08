mod git;
mod selection;
mod sessions_list;

use crate::{
    config::SessionCandidate,
    theme::Theme,
    tui::{
        git::start_poll,
        selection::{Intent, Selection},
        sessions_list::{
            SessionsListState, VisualMotion, apply_query_folds,
            apply_visual_motion, collapse_group, expand_group,
        },
    },
};
use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    crossterm::event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    },
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{StatefulWidget, Widget},
};
use std::{io, sync::mpsc, time::Duration};
use terminal_colorsaurus::ThemeMode;

use git::CandidateState;

const POLL_TICK: Duration = Duration::from_millis(100);

pub struct Tui {
    outcome: Option<Option<String>>,
    selection: Selection,
    sessions_list_state: SessionsListState,
    theme_mode: ThemeMode,
    git_rx: mpsc::Receiver<Vec<(String, CandidateState)>>,
}

impl Tui {
    pub fn new(candidates: &[SessionCandidate], theme_mode: ThemeMode) -> Self {
        let paths = SessionCandidate::paths(candidates);
        let mut sessions_list_state = SessionsListState::new(theme_mode);
        // Group keys and their resolved-candidate order ride
        // along for grouped presentation and visual motion.
        sessions_list_state.groups = candidates
            .iter()
            .map(|c| (c.path.clone(), c.group.clone()))
            .collect();
        let mut group_order = vec![];
        for c in candidates {
            if !group_order.contains(&c.group) {
                group_order.push(c.group.clone());
            }
        }
        sessions_list_state.group_order = group_order;
        sessions_list_state.home_discovery_group = candidates
            .iter()
            .find(|c| c.from_home_discovery)
            .map(|c| c.group.clone());
        let mut selection = Selection::new(&paths);
        apply_query_folds(&mut sessions_list_state, &mut selection);
        apply_visual_motion(
            &mut selection,
            &mut sessions_list_state,
            VisualMotion::First,
        );
        Tui {
            outcome: None,
            selection,
            sessions_list_state,
            theme_mode,
            git_rx: start_poll(paths),
        }
    }

    fn apply_git_snapshots(&mut self) {
        while let Ok(msg) = self.git_rx.try_recv() {
            // Drain into last-known state: `GitStates` owns the
            // merge-never-replace rule, so omitted candidates
            // stay known and unknown keys render as loading.
            self.sessions_list_state.git.apply(msg);
        }
    }

    fn handle_events(&mut self) -> io::Result<()> {
        if event::poll(POLL_TICK)? {
            if let Event::Key(key) = event::read()? {
                // Visual motions walk grouped stops (folded
                // headers and children). Fold keys expand or
                // collapse without going through Selection.
                // Query edits recompute folds; motion never
                // does, so active_header survives Up/Down.
                if let Some(motion) = visual_motion(&key) {
                    apply_visual_motion(
                        &mut self.selection,
                        &mut self.sessions_list_state,
                        motion,
                    );
                } else if !fold_key(
                    &mut self.selection,
                    &mut self.sessions_list_state,
                    &key,
                ) {
                    let before = self.selection.query().to_string();
                    match self.selection.handle_key(key) {
                        Intent::None => {}
                        Intent::Exit => self.outcome = Some(None),
                        Intent::Activate(candidate) => {
                            self.outcome = Some(Some(candidate));
                        }
                    }
                    if self.selection.query() != before {
                        apply_query_folds(
                            &mut self.sessions_list_state,
                            &mut self.selection,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Widget::render(self, area, buf);
    }

    /// Run the event and render loop until the user exits or activates a
    /// session candidate. Returns the selected candidate, if any; the
    /// caller activates it after `ratatui::run` restores the terminal,
    /// so tmux never runs inside the loop.
    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> io::Result<Option<String>> {
        while self.outcome.is_none() {
            self.apply_git_snapshots();
            terminal
                .draw(|frame| self.render(frame.area(), frame.buffer_mut()))?;
            self.handle_events()?;
        }
        Ok(self.outcome.take().flatten())
    }
}

impl Widget for &mut Tui {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let constraints = vec![Constraint::Length(1), Constraint::Fill(1)];

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let t = Theme::get(self.theme_mode);
        buf.set_style(area, Style::new().bg(t.bg).fg(t.fg));

        Line::from(vec![
            Span::styled("Search: ", Style::new().bg(t.bg).fg(t.operator)),
            Span::styled(
                self.selection.query(),
                Style::new().bg(t.bg).fg(t.fg),
            ),
            Span::styled("█", Style::new().bg(t.bg).fg(t.git_icon)),
        ])
        .style(Style::new().bg(t.bg).fg(t.fg))
        .render(areas[0], buf);
        (&self.selection).render(areas[1], buf, &mut self.sessions_list_state);
    }
}

/// Expand/collapse keys for grouped lists. Enter/Right/Space on
/// a folded header expand it; Left on a child collapses its
/// parent. Returns true when the key was consumed so it never
/// reaches Selection (folded-header Enter must not activate).
fn fold_key(
    sel: &mut Selection,
    state: &mut SessionsListState,
    key: &KeyEvent,
) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }
    if key.modifiers.contains(KeyModifiers::ALT)
        || key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return false;
    }
    if state.groups.is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Right | KeyCode::Char(' ')
            if state.active_header.is_some() =>
        {
            expand_group(state, sel);
            true
        }
        KeyCode::Left if state.active_header.is_some() => true,
        KeyCode::Left => {
            let Some(m) =
                sel.matches().get(sel.selected_line().saturating_sub(1))
            else {
                return false;
            };
            let Some(group) = state.groups.get(&m.entry).cloned() else {
                return false;
            };
            collapse_group(state, &group);
            true
        }
        _ => false,
    }
}

/// Visual motion for one key press, if it is one. Mirrors the
/// motion half of the selection key seam — plain arrows and
/// Home/End, plus the Ctrl-J/K/T/G/B aliases — so the event
/// loop walks grouped children while typing, word deletion,
/// exit, and activation keep their seam. Alt chords, releases,
/// and anything else are not motions.
fn visual_motion(key: &KeyEvent) -> Option<VisualMotion> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let modifiers = key.modifiers;
    if modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('j') => Some(VisualMotion::Next),
            KeyCode::Char('k') => Some(VisualMotion::Prev),
            KeyCode::Char('t') => Some(VisualMotion::First),
            KeyCode::Char('g') | KeyCode::Char('b') => Some(VisualMotion::Last),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Up => Some(VisualMotion::Prev),
        KeyCode::Down => Some(VisualMotion::Next),
        KeyCode::Home => Some(VisualMotion::First),
        KeyCode::End => Some(VisualMotion::Last),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{VisualMotion, visual_motion};
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn arrows_and_home_end_are_visual_motions() {
        assert_eq!(visual_motion(&key(KeyCode::Up)), Some(VisualMotion::Prev));
        assert_eq!(
            visual_motion(&key(KeyCode::Down)),
            Some(VisualMotion::Next)
        );
        assert_eq!(
            visual_motion(&key(KeyCode::Home)),
            Some(VisualMotion::First)
        );
        assert_eq!(visual_motion(&key(KeyCode::End)), Some(VisualMotion::Last));
    }

    #[test]
    fn vim_aliases_are_visual_motions() {
        assert_eq!(
            visual_motion(&ctrl(KeyCode::Char('j'))),
            Some(VisualMotion::Next)
        );
        assert_eq!(
            visual_motion(&ctrl(KeyCode::Char('k'))),
            Some(VisualMotion::Prev)
        );
        assert_eq!(
            visual_motion(&ctrl(KeyCode::Char('t'))),
            Some(VisualMotion::First)
        );
        assert_eq!(
            visual_motion(&ctrl(KeyCode::Char('g'))),
            Some(VisualMotion::Last)
        );
        assert_eq!(
            visual_motion(&ctrl(KeyCode::Char('b'))),
            Some(VisualMotion::Last)
        );
    }

    #[test]
    fn editing_exit_and_alt_chords_are_not_motions() {
        // Typing, word deletion, and exit keep the key seam.
        assert_eq!(visual_motion(&key(KeyCode::Char('a'))), None);
        assert_eq!(visual_motion(&key(KeyCode::Enter)), None);
        assert_eq!(visual_motion(&ctrl(KeyCode::Char('w'))), None);
        assert_eq!(visual_motion(&ctrl(KeyCode::Char('c'))), None);
        assert_eq!(
            visual_motion(&KeyEvent::new(KeyCode::Up, KeyModifiers::ALT)),
            None
        );
        // Releases never move, like the selection seam.
        assert_eq!(
            visual_motion(&KeyEvent::new_with_kind(
                KeyCode::Down,
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            None
        );
    }
}
