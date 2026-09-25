use crate::{
    config::SessionCandidate,
    theme::Theme,
    tui::{
        git::CandidateState,
        selection::{Intent, Selection},
        sessions_list::{
            SessionsListState, VisualMotion, VisualTarget, apply_query_folds,
            apply_visual_motion, collapse_group, expand_group,
            group_run_for_entry, placed_run_keys, visual_entries,
        },
    },
};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use std::collections::HashSet;

/// Project-target picker workflow: session-candidate catalog,
/// query, layout, and focus live here. The TUI and tests drive
/// the same interface — keys and Git snapshots in, visible
/// stops, fold state, focus, and activation intent out.
/// Multiplexer open stays outside.
/// In-picker action menu opened by Ctrl-X, then a sequential key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionMenu {
    /// None = no explicit navigation yet, 0 = clone, 1 = new
    /// directory, 2 = delete.
    pub selected: Option<usize>,
    pub delete_enabled: bool,
}

pub(crate) struct Picker {
    pub(crate) selection: Selection,
    pub(crate) list: SessionsListState,
    menu: Option<ActionMenu>,
}

impl Picker {
    pub(crate) fn new(candidates: &[SessionCandidate], theme: Theme) -> Self {
        let paths = SessionCandidate::paths(candidates);
        let mut list = SessionsListState::new(theme);
        list.groups = candidates
            .iter()
            .map(|c| (c.path.clone(), c.group.clone()))
            .collect();
        let mut group_order = vec![];
        for c in candidates {
            if !group_order.contains(&c.group) {
                group_order.push(c.group.clone());
            }
        }
        list.group_order = group_order;
        list.home_discovery_group = candidates
            .iter()
            .find(|c| c.from_home_discovery)
            .map(|c| c.group.clone());
        let mut selection = Selection::new(&paths);
        apply_query_folds(&mut list, &mut selection);
        apply_visual_motion(&mut selection, &mut list, VisualMotion::First);
        Picker {
            selection,
            list,
            menu: None,
        }
    }

    /// One key: action menu, visual motion, fold, query edit, or intent.
    /// Enter on a focused header expands and never activates.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Intent {
        if key.kind != KeyEventKind::Press {
            return Intent::None;
        }
        if self.menu.is_some() {
            return self.handle_menu_key(&key);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('x') | KeyCode::Char('X'))
        {
            self.open_menu();
            return Intent::None;
        }
        if let Some(motion) = visual_motion(&key) {
            apply_visual_motion(&mut self.selection, &mut self.list, motion);
            return Intent::None;
        }
        if fold_key(&mut self.selection, &mut self.list, &key) {
            return Intent::None;
        }
        let before = self.selection.query().to_string();
        let intent = self.selection.handle_key(key);
        if self.selection.query() != before {
            apply_query_folds(&mut self.list, &mut self.selection);
        }
        intent
    }

    fn open_menu(&mut self) {
        self.menu = Some(ActionMenu {
            selected: None,
            delete_enabled: self.delete_enabled(),
        });
    }

    fn handle_menu_key(&mut self, key: &KeyEvent) -> Intent {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('x') | KeyCode::Char('X'))
        {
            self.menu = None;
            return Intent::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('c')
        {
            self.menu = None;
            return Intent::Exit;
        }
        match key.code {
            KeyCode::Esc => {
                self.menu = None;
                Intent::None
            }
            KeyCode::Char('c') | KeyCode::Char('C')
                if key.modifiers.is_empty() =>
            {
                self.menu = None;
                Intent::Clone
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.is_empty() =>
            {
                self.menu = None;
                Intent::NewDir
            }
            KeyCode::Char('d') | KeyCode::Char('D')
                if key.modifiers.is_empty() =>
            {
                self.take_delete()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(menu) = &mut self.menu {
                    menu.selected =
                        Some(menu.selected.map_or(0, |s| s.saturating_sub(1)));
                }
                Intent::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(menu) = &mut self.menu {
                    menu.selected = match menu.selected {
                        None | Some(0) => Some(1),
                        Some(1) if menu.delete_enabled => Some(2),
                        Some(s) => Some(s),
                    };
                }
                Intent::None
            }
            KeyCode::Enter => {
                let selected = self.menu.as_ref().and_then(|m| m.selected);
                match selected {
                    Some(2) => self.take_delete(),
                    Some(1) => {
                        self.menu = None;
                        Intent::NewDir
                    }
                    Some(0) | None => {
                        self.menu = None;
                        Intent::Clone
                    }
                    _ => Intent::None,
                }
            }
            _ => Intent::None,
        }
    }

    fn take_delete(&mut self) -> Intent {
        let enabled = self.menu.as_ref().is_some_and(|m| m.delete_enabled);
        self.menu = None;
        if !enabled {
            return Intent::None;
        }
        match self.focus() {
            Some(VisualTarget::Child(path)) => Intent::Delete(path),
            _ => Intent::None,
        }
    }

    pub(crate) fn delete_enabled(&self) -> bool {
        matches!(self.focus(), Some(VisualTarget::Child(_)))
    }

    /// Folded header focused: Enter/Right/Space expands it.
    pub(crate) fn can_expand(&self) -> bool {
        matches!(self.focus(), Some(VisualTarget::Header(..)))
    }

    /// Child focused under a grouped list: Left collapses its parent.
    pub(crate) fn can_collapse(&self) -> bool {
        matches!(self.focus(), Some(VisualTarget::Child(_)))
            && !self.list.groups.is_empty()
    }

    pub(crate) fn menu(&self) -> Option<&ActionMenu> {
        self.menu.as_ref()
    }

    /// Group path used as the picker clone destination parent.
    /// Child → its group; focused header → that group key; none → unset.
    pub(crate) fn clone_dest_parent(&self) -> Option<String> {
        match self.focus() {
            Some(VisualTarget::Child(path)) => {
                self.list.groups.get(&path).cloned()
            }
            Some(VisualTarget::Header(idx, _)) => {
                self.list.group_order.get(idx).cloned()
            }
            None => None,
        }
    }

    /// Replace the catalog, preserving the query and query-based folds.
    pub(crate) fn replace_catalog(&mut self, candidates: &[SessionCandidate]) {
        let paths = SessionCandidate::paths(candidates);
        let keep: HashSet<String> = paths.iter().cloned().collect();
        self.list.git.retain_paths(&keep);
        self.list.groups = candidates
            .iter()
            .map(|c| (c.path.clone(), c.group.clone()))
            .collect();
        let mut group_order = vec![];
        for c in candidates {
            if !group_order.contains(&c.group) {
                group_order.push(c.group.clone());
            }
        }
        self.list.group_order = group_order;
        self.list.home_discovery_group = candidates
            .iter()
            .find(|c| c.from_home_discovery)
            .map(|c| c.group.clone());
        self.selection.replace_candidates(&paths);
        apply_query_folds(&mut self.list, &mut self.selection);
    }

    /// Rediscover after delete: keep query, restore expanded groups,
    /// focus the nearest remaining visible stop.
    pub(crate) fn refresh_after_delete(
        &mut self,
        candidates: &[SessionCandidate],
        deleted: &str,
    ) {
        let old_stops = self.stops();
        let expanded = self.expanded_group_keys();
        self.replace_catalog(candidates);
        self.restore_expanded(&expanded);
        self.focus_nearest(&old_stops, deleted);
    }

    /// Rediscover after clone. Focuses `dest` only if it is a catalog
    /// candidate matching the current query. Returns whether it was focused.
    pub(crate) fn refresh_after_clone(
        &mut self,
        candidates: &[SessionCandidate],
        dest: &str,
    ) -> bool {
        let dest_id = crate::config::identity(dest);
        let catalog_path = candidates
            .iter()
            .find(|c| crate::config::identity(&c.path) == dest_id)
            .map(|c| c.path.clone());
        let expanded = self.expanded_group_keys();
        self.replace_catalog(candidates);
        self.restore_expanded(&expanded);
        let Some(path) = catalog_path else {
            return false;
        };
        if !self.selection.matches().iter().any(|m| m.entry == path) {
            return false;
        }
        self.expand_and_focus(&path);
        true
    }

    fn expanded_group_keys(&self) -> HashSet<String> {
        self.list
            .group_order
            .iter()
            .filter(|g| !self.list.folded.contains(&(g.to_string(), 0)))
            .cloned()
            .collect()
    }

    fn restore_expanded(&mut self, keys: &HashSet<String>) {
        if !self.selection.query().is_empty() {
            return;
        }
        for key in keys {
            if self.list.group_order.iter().any(|g| g == key) {
                self.list.folded.remove(&(key.clone(), 0));
            }
        }
    }

    fn expand_and_focus(&mut self, path: &str) {
        if let Some(group) = self.list.groups.get(path).cloned() {
            self.list.folded.remove(&(group, 0));
        }
        self.list.active_header = None;
        self.selection.select_entry(path);
    }

    fn focus_nearest(&mut self, old_stops: &[VisualTarget], deleted: &str) {
        let new_stops = self.stops();
        if new_stops.is_empty() {
            self.list.active_header = None;
            return;
        }
        let old_i = old_stops
            .iter()
            .position(|s| matches!(s, VisualTarget::Child(p) if p == deleted));
        if let Some(i) = old_i {
            let before = old_stops[..i].iter().rev();
            let after = old_stops[i + 1..].iter();
            for stop in before.chain(after) {
                if new_stops.contains(stop) {
                    self.land(stop);
                    return;
                }
            }
        }
        self.land(&new_stops[0]);
    }

    /// Merge a Git snapshot into last-known state, then reconcile
    /// folds and focus. Does not re-run query folds. Never wraps,
    /// invents a row, or activates.
    pub(crate) fn apply_git(
        &mut self,
        snapshot: Vec<(String, CandidateState)>,
    ) {
        let prev = self.focus();
        let prev_key = match &prev {
            Some(VisualTarget::Header(idx, _)) => {
                self.list.group_order.get(*idx).cloned()
            }
            _ => None,
        };
        self.list.git.apply(snapshot);
        let placed: HashSet<(String, usize)> =
            placed_run_keys(&self.selection, &self.list)
                .into_iter()
                .collect();
        self.list.folded.retain(|k| placed.contains(k));
        self.reconcile_focus(prev, prev_key);
    }

    fn reconcile_focus(
        &mut self,
        prev: Option<VisualTarget>,
        prev_key: Option<String>,
    ) {
        let stops = self.stops();
        if stops.is_empty() {
            self.list.active_header = None;
            return;
        }
        if let Some(focus) = &prev
            && stops.contains(focus)
        {
            self.land(focus);
            return;
        }
        if let Some(focus) = &prev
            && matches!(focus, VisualTarget::Header(..))
            && let Some(key) = &prev_key
            && let Some(header) = stops.iter().find(|stop| {
                matches!(stop, VisualTarget::Header(idx, _) if self.list.group_order.get(*idx) == Some(key))
            })
        {
            let header = header.clone();
            self.land(&header);
            return;
        }
        apply_visual_motion(
            &mut self.selection,
            &mut self.list,
            VisualMotion::First,
        );
    }

    fn land(&mut self, target: &VisualTarget) {
        match target {
            VisualTarget::Child(entry) => {
                self.list.active_header = None;
                self.selection.select_entry(entry);
            }
            VisualTarget::Header(idx, run) => {
                self.list.active_header = Some((*idx, *run));
            }
        }
    }

    pub(crate) fn query(&self) -> &str {
        self.selection.query()
    }

    pub(crate) fn stops(&self) -> Vec<VisualTarget> {
        visual_entries(&self.selection, &self.list)
    }

    /// Focused visual stop: a folded header or a child entry.
    pub(crate) fn focus(&self) -> Option<VisualTarget> {
        match self.list.active_header {
            Some((idx, run)) => Some(VisualTarget::Header(idx, run)),
            None => self
                .selection
                .matches()
                .get(self.selection.selected_line().saturating_sub(1))
                .map(|m| VisualTarget::Child(m.entry.clone())),
        }
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
            let Some((group, run)) = group_run_for_entry(sel, state, &m.entry)
            else {
                return false;
            };
            collapse_group(state, &group, run);
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
pub(crate) fn visual_motion(key: &KeyEvent) -> Option<VisualMotion> {
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
#[path = "picker_tests.rs"]
mod tests;
