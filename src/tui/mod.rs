mod action;
mod git;
mod picker;
mod selection;
mod sessions_list;
mod workers;

use crate::{
    config::{ResolvedConfig, SessionCandidate},
    delete,
    theme::Theme,
    tui::{
        action::{ActionDialog, DialogOutcome, PortablePty, ToastKind},
        git::start_poll,
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

        let menu = self.picker.menu().is_some();
        let mut constraints = vec![Constraint::Length(1)];
        if menu {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Fill(1));

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let t = Theme::get(self.theme_mode);
        buf.set_style(area, Style::new().bg(t.bg).fg(t.fg));

        Line::from(vec![
            Span::styled("Search: ", Style::new().bg(t.bg).fg(t.operator)),
            Span::styled(self.picker.query(), Style::new().bg(t.bg).fg(t.fg)),
            Span::styled("█", Style::new().bg(t.bg).fg(t.git_icon)),
        ])
        .style(Style::new().bg(t.bg).fg(t.fg))
        .render(areas[0], buf);
        if let Some(menu) = self.picker.menu().cloned() {
            action_menu_line(menu.selected, menu.delete_enabled, t)
                .render(areas[1], buf);
            (&self.picker.selection).render(
                areas[2],
                buf,
                &mut self.picker.list,
            );
        } else {
            (&self.picker.selection).render(
                areas[1],
                buf,
                &mut self.picker.list,
            );
        }
        if let Some(dialog) = &self.dialog {
            action::render_dialog(dialog, area, buf);
        }
        if let Some(toast) = &self.toast {
            action::render_toast(area, buf, toast.kind, &toast.msg);
        }
    }
}

fn action_menu_line(
    selected: usize,
    delete_enabled: bool,
    t: Theme,
) -> Line<'static> {
    let item = |label: &'static str, idx: usize, enabled: bool| {
        let selected = selected == idx;
        let mut style = Style::new().bg(t.bg);
        if !enabled {
            style = style.fg(t.comment);
        } else if selected {
            style = style.fg(t.accent).add_modifier(Modifier::BOLD);
        } else {
            style = style.fg(t.fg);
        }
        Span::styled(label.to_string(), style)
    };
    Line::from(vec![
        Span::styled("Actions: ", Style::new().bg(t.bg).fg(t.operator)),
        item("clone", 0, true),
        Span::styled("  ", Style::new().bg(t.bg).fg(t.fg)),
        item("delete", 1, delete_enabled),
    ])
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
