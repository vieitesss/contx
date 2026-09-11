mod action;
mod git;
mod hints;
mod picker;
mod selection;
mod sessions_list;
mod workers;

use crate::{
    config::{ResolvedConfig, SessionCandidate},
    delete,
    theme::Theme,
    tui::{
        action::{
            ActionDialog, DialogOutcome, FocusItem, PortablePty,
            REFRESH_PENDING_HINT, ToastKind,
        },
        git::start_poll,
        hints::{
            DialogHintState, HintChip, HintRow, HintSurface, format_hint_rows,
            shortcut_hints,
        },
        picker::Picker,
        selection::Intent,
        workers::{
            FsCloneProbe, RefreshEvent, RefreshKind, ThreadedConfigAppend,
            ThreadedInspect, ThreadedMutate, spawn_refresh,
        },
    },
};
use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{StatefulWidget, Widget},
};
use std::{
    io,
    sync::mpsc,
    time::{Duration, Instant},
};
use terminal_colorsaurus::ThemeMode;

use git::CandidateState;

const POLL_TICK: Duration = Duration::from_millis(100);
const TOAST_FOR: Duration = Duration::from_secs(3);
const CANCEL_TOAST_FOR: Duration = Duration::from_millis(1500);

struct Toast {
    kind: ToastKind,
    msg: String,
    at: Instant,
}

pub struct Tui {
    outcome: Option<Option<String>>,
    picker: Picker,
    theme_mode: ThemeMode,
    git_rx: mpsc::Receiver<Vec<(String, CandidateState)>>,
    config: Option<ResolvedConfig>,
    dialog: Option<ActionDialog>,
    toast: Option<Toast>,
    refresh_tx: mpsc::Sender<RefreshEvent>,
    refresh_rx: mpsc::Receiver<RefreshEvent>,
    refresh_in_flight: bool,
    /// Test-only: keep refresh requests pending instead of spawning
    /// the worker, so a test can deliver the event by hand.
    #[cfg(test)]
    hold_refresh: bool,
}

impl Tui {
    pub fn new(candidates: &[SessionCandidate], theme_mode: ThemeMode) -> Self {
        Self::build(candidates, None, theme_mode)
    }

    pub fn from_config(config: ResolvedConfig, theme_mode: ThemeMode) -> Self {
        let candidates = config.candidates.clone();
        Self::build(&candidates, Some(config), theme_mode)
    }

    fn build(
        candidates: &[SessionCandidate],
        config: Option<ResolvedConfig>,
        theme_mode: ThemeMode,
    ) -> Self {
        let paths = SessionCandidate::paths(candidates);
        let (refresh_tx, refresh_rx) = mpsc::channel();
        Tui {
            outcome: None,
            picker: Picker::new(candidates, theme_mode),
            theme_mode,
            git_rx: start_poll(paths),
            config,
            dialog: None,
            toast: None,
            refresh_tx,
            refresh_rx,
            refresh_in_flight: false,
            #[cfg(test)]
            hold_refresh: false,
        }
    }

    fn apply_git_snapshots(&mut self) {
        while let Ok(msg) = self.git_rx.try_recv() {
            // Drain into last-known state: `GitStates` owns the
            // merge-never-replace rule, so omitted candidates
            // stay known and unknown keys render as loading.
            self.picker.apply_git(msg);
        }
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let now = Instant::now();
        self.pump(now);
        if event::poll(POLL_TICK)?
            && let Event::Key(key) = event::read()?
        {
            self.handle_key(key);
            self.pump(now);
        }
        Ok(())
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != ratatui::crossterm::event::KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        {
            if let Some(dialog) = &mut self.dialog {
                dialog.interrupt_child();
            }
            self.outcome = Some(None);
            return;
        }
        if self.dialog.is_some() {
            if self.refresh_in_flight && self.refresh_blocks_key(&key) {
                if let Some(dialog) = &mut self.dialog {
                    dialog.set_hint(REFRESH_PENDING_HINT);
                }
                return;
            }
            let out = self.dialog.as_mut().and_then(|d| d.handle_key(key));
            self.after_dialog(out);
            return;
        }
        match self.picker.handle_key(key) {
            Intent::None => {}
            Intent::Exit => self.outcome = Some(None),
            Intent::Activate(candidate) => {
                self.outcome = Some(Some(candidate));
            }
            Intent::Clone => self.open_clone_dialog(),
            Intent::Delete(path) => self.open_delete_dialog(path),
        }
    }

    pub(crate) fn pump(&mut self, now: Instant) {
        self.expire_toast(now);
        if let Some(dialog) = &mut self.dialog {
            dialog.tick(now);
            let out = dialog.pump();
            self.after_dialog(out);
        }
        self.drain_refresh();
        self.maybe_start_refresh();
    }

    fn restart_poll(&mut self, candidates: &[SessionCandidate]) {
        self.git_rx = start_poll(SessionCandidate::paths(candidates));
    }

    /// Keys that would drop the dialog while the catalog refresh it
    /// started is still in flight: Escape (cancel) and Enter on the
    /// Acknowledge stop. The picker update must land first, so the
    /// dialog stays open until `on_refresh` clears the pending state.
    fn refresh_blocks_key(&self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => self
                .dialog
                .as_ref()
                .is_some_and(|d| d.item() == Some(FocusItem::Ack)),
            _ => false,
        }
    }

    fn open_clone_dialog(&mut self) {
        let parent = self.picker.clone_dest_parent();
        let mut dialog = ActionDialog::open_clone(
            parent,
            FsCloneProbe {
                config: self.config.clone(),
            },
        );
        dialog.set_transport(Box::new(PortablePty));
        if let Some(cfg) = &self.config {
            dialog.set_config_append(Box::new(ThreadedConfigAppend::new(
                cfg.clone(),
            )));
        }
        self.dialog = Some(dialog);
    }

    fn open_delete_dialog(&mut self, path: String) {
        let mut dialog = ActionDialog::open_delete(path.clone());
        dialog.set_transport(Box::new(PortablePty));
        if let Some(cfg) = &self.config {
            let permanent = cfg.permanent_delete;
            if let Ok(class) = delete::classify(&path) {
                let strategy = delete::strategy(class, false, permanent);
                // PTY fetch is requested later by the inspect worker only
                // when a standalone repository actually has remotes.
                dialog.set_delete_plan(class, strategy, false);
            }
            dialog.set_inspect_worker(Box::new(ThreadedInspect::new(cfg)));
            dialog.set_mutate_worker(Box::new(ThreadedMutate::new(
                cfg.candidates.clone(),
            )));
        }
        self.dialog = Some(dialog);
    }

    fn after_dialog(&mut self, out: Option<DialogOutcome>) {
        match out {
            Some(DialogOutcome::Cancelled) => {
                self.dialog = None;
                self.refresh_in_flight = false;
                self.show_toast(ToastKind::Cancel, "cancelled");
            }
            Some(DialogOutcome::Failed { .. }) => {
                self.dialog = None;
                self.refresh_in_flight = false;
            }
            Some(DialogOutcome::Completed { .. })
                if out.as_ref().is_some_and(DialogOutcome::needs_ack) =>
            {
                self.dialog = None;
                self.refresh_in_flight = false;
            }
            None | Some(DialogOutcome::Completed { .. }) => {}
        }
        self.maybe_start_refresh();
    }

    fn maybe_start_refresh(&mut self) {
        if self.refresh_in_flight {
            return;
        }
        let config_path = self.config.as_ref().map(|c| c.config_path.clone());
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        if !dialog.consume_refresh_request() {
            return;
        }
        let path = dialog.mutation_path().unwrap_or("").to_string();
        let generation = dialog.generation();
        let is_clone = dialog.is_clone();
        let config_error = match dialog.outcome() {
            Some(DialogOutcome::Completed { config_error, .. }) => {
                config_error.clone()
            }
            _ => None,
        };
        let kind = if is_clone {
            RefreshKind::Clone {
                dest: path,
                config_error,
            }
        } else {
            RefreshKind::Delete { path }
        };
        let Some(config_path) = config_path else {
            self.finish_refresh_without_config(kind);
            return;
        };
        self.refresh_in_flight = true;
        #[cfg(test)]
        if self.hold_refresh {
            return;
        }
        spawn_refresh(self.refresh_tx.clone(), generation, kind, config_path);
    }

    fn finish_refresh_without_config(&mut self, kind: RefreshKind) {
        let sticky = matches!(
            &kind,
            RefreshKind::Clone {
                config_error: Some(_),
                ..
            }
        );
        if sticky {
            self.refresh_in_flight = false;
            return;
        }
        let msg = match kind {
            RefreshKind::Clone { dest, .. } => format!("cloned to `{dest}`"),
            RefreshKind::Delete { path } => format!("deleted `{path}`"),
        };
        self.dialog = None;
        self.refresh_in_flight = false;
        self.show_toast(ToastKind::Success, &msg);
    }

    fn drain_refresh(&mut self) {
        while let Ok(ev) = self.refresh_rx.try_recv() {
            self.on_refresh(ev);
        }
    }

    fn on_refresh(&mut self, ev: RefreshEvent) {
        let dialog_gen = self.dialog.as_ref().map(ActionDialog::generation);
        let Some(generation) = dialog_gen else {
            return;
        };
        if ev.generation != generation {
            return;
        }
        match ev.result {
            Ok(cands) => {
                match &ev.kind {
                    RefreshKind::Clone { dest, .. } => {
                        self.picker.refresh_after_clone(&cands, dest);
                    }
                    RefreshKind::Delete { path } => {
                        self.picker.refresh_after_delete(&cands, path);
                    }
                }
                self.restart_poll(&cands);
                if let Some(cfg) = &mut self.config {
                    cfg.candidates = cands;
                }
                let sticky = match &ev.kind {
                    RefreshKind::Clone {
                        config_error: Some(_),
                        ..
                    } => true,
                    _ => self
                        .dialog
                        .as_ref()
                        .and_then(ActionDialog::outcome)
                        .is_some_and(DialogOutcome::needs_ack),
                };
                self.refresh_in_flight = false;
                if let Some(dialog) = &mut self.dialog {
                    dialog.clear_refresh_hint();
                }
                if sticky {
                    return;
                }
                let msg = match ev.kind {
                    RefreshKind::Clone { dest, .. } => {
                        format!("cloned to `{dest}`")
                    }
                    RefreshKind::Delete { path } => {
                        format!("deleted `{path}`")
                    }
                };
                self.dialog = None;
                self.show_toast(ToastKind::Success, &msg);
            }
            Err(e) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.apply_refresh_error(e);
                    dialog.clear_refresh_hint();
                }
                self.refresh_in_flight = false;
            }
        }
    }

    fn show_toast(&mut self, kind: ToastKind, msg: &str) {
        self.toast = Some(Toast {
            kind,
            msg: msg.to_string(),
            at: Instant::now(),
        });
    }

    fn expire_toast(&mut self, now: Instant) {
        let ttl = match self.toast.as_ref().map(|t| t.kind) {
            Some(ToastKind::Success) => TOAST_FOR,
            Some(ToastKind::Cancel) => CANCEL_TOAST_FOR,
            None => return,
        };
        if let Some(t) = &self.toast
            && now.saturating_duration_since(t.at) >= ttl
        {
            self.toast = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn query(&self) -> &str {
        self.picker.query()
    }

    #[cfg(test)]
    pub(crate) fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    #[cfg(test)]
    pub(crate) fn toast_message(&self) -> Option<&str> {
        self.toast.as_ref().map(|t| t.msg.as_str())
    }

    #[cfg(test)]
    pub(crate) fn open_delete_for_test(&mut self, path: String) {
        self.open_delete_dialog(path);
    }

    #[cfg(test)]
    pub(crate) fn set_dialog_transport(&mut self, transport: action::FakePty) {
        if let Some(dialog) = &mut self.dialog {
            dialog.set_transport(Box::new(transport));
        }
    }

    #[cfg(test)]
    pub(crate) fn delete_findings(&self) -> Vec<String> {
        self.dialog
            .as_ref()
            .map(ActionDialog::delete_findings)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn mark_dialog_child_started_for_test(&mut self) {
        if let Some(dialog) = &mut self.dialog {
            dialog.mark_child_started();
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_refresh_for_test(&mut self, hold: bool) {
        self.hold_refresh = hold;
    }

    #[cfg(test)]
    pub(crate) fn set_dialog_awaiting_for_test(
        &mut self,
        awaiting: crate::tui::hints::DialogAwaiting,
    ) {
        if let Some(dialog) = &mut self.dialog {
            dialog.set_awaiting_for_test(awaiting);
        }
    }

    fn hint_surface(&self) -> HintSurface {
        if let Some(dialog) = &self.dialog {
            return HintSurface::Dialog(DialogHintState {
                running: dialog.running(),
                cancel: dialog.cancel_state(),
                item_count: dialog.items().len(),
                sticky: dialog.outcome().is_some_and(DialogOutcome::needs_ack),
                awaiting: dialog.awaiting(),
                typing: dialog.typing(),
                inspect_enabled: dialog.inspect_enabled(),
                add_parent_focused: dialog.add_parent_focused(),
                refresh_pending: self.refresh_in_flight,
            });
        }
        if let Some(menu) = self.picker.menu() {
            return HintSurface::PickerPrefix {
                delete_enabled: menu.delete_enabled,
                selected: menu.selected,
            };
        }
        HintSurface::PickerIdle {
            expand: self.picker.can_expand(),
            collapse: self.picker.can_collapse(),
        }
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

        let t = Theme::get(self.theme_mode);
        buf.set_style(area, Style::new().bg(t.bg).fg(t.fg));

        let chips = shortcut_hints(self.hint_surface());
        // Small-height policy: hints never starve the picker. The
        // budget leaves Search (one row) plus one two-row list stop
        // whenever the height allows it, and always keeps one hint
        // row visible at height two. Hints pack, cap, and ellipsize
        // to fit; shorter terminals degrade top to bottom (search,
        // list, hints).
        let max_rows = area.height.saturating_sub(3).max(1) as usize;
        let rows = if area.height >= 2 {
            format_hint_rows(&chips, area.width as usize, max_rows)
        } else {
            Vec::new()
        };
        let (content, hint) = if area.height >= 2 {
            let hint_h = rows.len().max(1) as u16;
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Fill(1), Constraint::Length(hint_h)])
                .split(area);
            (parts[0], Some(parts[1]))
        } else {
            (area, None)
        };

        let constraints = [Constraint::Length(1), Constraint::Fill(1)];

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(content);

        Line::from(vec![
            Span::styled("Search: ", Style::new().bg(t.bg).fg(t.operator)),
            Span::styled(self.picker.query(), Style::new().bg(t.bg).fg(t.fg)),
            Span::styled("█", Style::new().bg(t.bg).fg(t.git_icon)),
        ])
        .style(Style::new().bg(t.bg).fg(t.fg))
        .render(areas[0], buf);
        (&self.picker.selection).render(areas[1], buf, &mut self.picker.list);
        if let Some(dialog) = &self.dialog {
            action::render_dialog(dialog, content, buf);
        }
        if let Some(toast) = &self.toast {
            action::render_toast(content, buf, toast.kind, &toast.msg);
        }
        if let Some(hint) = hint {
            for (i, row) in rows.iter().enumerate() {
                let y = hint.y.saturating_add(i as u16);
                if y >= hint.y.saturating_add(hint.height) {
                    break;
                }
                let rect = Rect {
                    x: hint.x,
                    y,
                    width: hint.width,
                    height: 1,
                };
                hint_bar_line(row, hint.width as usize, t).render(rect, buf);
            }
        }
    }
}

fn hint_bar_line(row: &HintRow, width: usize, t: Theme) -> Line<'static> {
    let text = row.text(width);
    let st = |fg| Style::new().bg(t.bg).fg(fg);
    let overlong = row.chips.len() == 1 && row.chips[0].width() > width;
    if overlong || text.chars().count() < row_join_len(row) {
        return Line::from(Span::styled(text, st(t.operator)));
    }
    let mut spans = Vec::new();
    for (i, chip) in row.chips.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", st(t.operator)));
        }
        spans.extend(chip_spans(chip, t));
    }
    if row.ellipsis {
        spans.push(Span::styled("…", st(t.operator)));
    }
    Line::from(spans)
}

fn chip_spans(chip: &HintChip, t: Theme) -> Vec<Span<'static>> {
    let key_style = if !chip.enabled {
        Style::new()
            .bg(t.bg_alt)
            .fg(t.comment)
            .add_modifier(Modifier::DIM)
    } else if chip.selected {
        Style::new()
            .bg(t.bg_alt)
            .fg(t.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().bg(t.bg_alt).fg(t.comment)
    };
    let label_style = if chip.enabled {
        Style::new().bg(t.bg).fg(t.operator)
    } else {
        Style::new()
            .bg(t.bg)
            .fg(t.comment)
            .add_modifier(Modifier::DIM)
    };
    match chip.label.split_once(' ') {
        Some((key, label)) => vec![
            Span::styled(format!(" {key} "), key_style),
            Span::styled(label.to_string(), label_style),
        ],
        None => vec![Span::styled(format!(" {} ", chip.label), key_style)],
    }
}

fn row_join_len(row: &HintRow) -> usize {
    let n: usize = row.chips.iter().map(HintChip::width).sum::<usize>()
        + row.chips.len().saturating_sub(1);
    if row.ellipsis { n + 1 } else { n }
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
