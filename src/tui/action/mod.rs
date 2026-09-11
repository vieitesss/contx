//! Progressive action dialog: field/focus/stage machine.
//! Wired into the TUI in a later node.

#![allow(dead_code)]

mod clone_flow;
mod delete_flow;
mod field;
mod prompt;
mod pty;
mod render;
mod state;

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

use crate::clone;

use crate::delete::{DeleteClass, DeleteStrategy, FetchResult};
use clone_flow::CloneJob;
use delete_flow::{DeleteJob, DeletePtyPhase};
use field::Field;
use prompt::{Interaction, PromptDecoder, PromptKind};
use pty::{PtyEvent, PtySession, PtySize, PtyTransport};
use state::{CloneAuth, CloneForm, DeleteForm};

pub(crate) use clone_flow::{ConfigAppend, ConfigAppendEvent};
pub(crate) use delete_flow::{
    InspectEvent, InspectWorker, MutateEvent, MutateFail, MutateKind,
    MutateWorker, RecordedFetch, inspect_event_from_preflight,
};
#[cfg(test)]
pub(crate) use pty::FakePty;
pub(crate) use pty::PortablePty;
pub(crate) use render::{ToastKind, render_dialog, render_toast};
pub(crate) use state::{
    CancelState, CloneStage, DeleteConfirm, DeleteStage, DialogOutcome,
    FocusItem,
};

/// Ctrl-G interrupt grace before Force Stop is offered.
pub(crate) const GRACE_FOR: Duration = Duration::from_millis(900);

/// Probe used for inline clone destination validation. Later nodes
/// supply filesystem/config implementations; tests inject scripts.
pub(crate) trait CloneDestProbe {
    fn exists(&self, abs: &str) -> bool;
    fn covered(&self, abs: &str) -> bool;
    fn env(&self, name: &str) -> Option<OsString>;
}

enum Op {
    Clone {
        form: CloneForm,
        probe: Box<dyn CloneDestProbe>,
    },
    Delete {
        form: DeleteForm,
    },
}

/// In-picker clone/delete dialog.
pub(crate) struct ActionDialog {
    op: Op,
    focus: usize,
    selected_stage: usize,
    inspect: Option<usize>,
    view_scroll: usize,
    generation: u64,
    hint: String,
    outcome: Option<DialogOutcome>,
    cancel: CancelState,
    grace_at: Option<Instant>,
    pty: Option<Box<dyn PtySession>>,
    job_generation: u64,
    log: Vec<String>,
    transport: Option<Box<dyn PtyTransport>>,
    config_append: Option<Box<dyn ConfigAppend>>,
    decoder: PromptDecoder,
    clone_job: Option<CloneJob>,
    refresh_requested: bool,
    awaiting_config: bool,
    inspect_worker: Option<Box<dyn InspectWorker>>,
    mutate_worker: Option<Box<dyn MutateWorker>>,
    delete_job: Option<DeleteJob>,
    awaiting_inspect: bool,
    awaiting_mutate: bool,
    mutation_path: Option<String>,
}

impl ActionDialog {
    pub(crate) fn open_clone(
        parent: Option<String>,
        probe: impl CloneDestProbe + 'static,
    ) -> Self {
        let mut dialog = Self {
            op: Op::Clone {
                form: CloneForm::new(parent),
                probe: Box::new(probe),
            },
            focus: 0,
            selected_stage: 0,
            inspect: None,
            view_scroll: 0,
            generation: 1,
            hint: String::new(),
            outcome: None,
            cancel: CancelState::Idle,
            grace_at: None,
            pty: None,
            job_generation: 0,
            log: vec![],
            transport: None,
            config_append: None,
            decoder: PromptDecoder::new(24, 80),
            clone_job: None,
            refresh_requested: false,
            awaiting_config: false,
            inspect_worker: None,
            mutate_worker: None,
            delete_job: None,
            awaiting_inspect: false,
            awaiting_mutate: false,
            mutation_path: None,
        };
        dialog.focus_item(FocusItem::Source);
        dialog
    }

    pub(crate) fn open_delete(path: String) -> Self {
        let mut dialog = Self {
            op: Op::Delete {
                form: DeleteForm::new(path),
            },
            focus: 0,
            selected_stage: 0,
            inspect: None,
            view_scroll: 0,
            generation: 1,
            hint: String::new(),
            outcome: None,
            cancel: CancelState::Idle,
            grace_at: None,
            pty: None,
            job_generation: 0,
            log: vec![],
            transport: None,
            config_append: None,
            decoder: PromptDecoder::new(24, 80),
            clone_job: None,
            refresh_requested: false,
            awaiting_config: false,
            inspect_worker: None,
            mutate_worker: None,
            delete_job: None,
            awaiting_inspect: false,
            awaiting_mutate: false,
            mutation_path: None,
        };
        dialog.focus_item(FocusItem::Action);
        dialog
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn hint(&self) -> &str {
        &self.hint
    }

    pub(crate) fn cancel_state(&self) -> CancelState {
        self.cancel
    }

    pub(crate) fn refresh_requested(&self) -> bool {
        self.refresh_requested
    }

    pub(crate) fn consume_refresh_request(&mut self) -> bool {
        let requested = self.refresh_requested;
        self.refresh_requested = false;
        requested
    }

    pub(crate) fn mutation_path(&self) -> Option<&str> {
        self.mutation_path.as_deref()
    }

    pub(crate) fn is_clone(&self) -> bool {
        matches!(self.op, Op::Clone { .. })
    }

    pub(crate) fn interrupt_child(&mut self) {
        if let Some(pty) = &mut self.pty {
            let _ = pty.interrupt();
        }
    }

    pub(crate) fn apply_refresh_error(&mut self, err: String) {
        match &mut self.outcome {
            Some(DialogOutcome::Completed { refresh_error, .. }) => {
                *refresh_error = Some(err);
            }
            _ => self.present_error(format!("refresh failed: {err}")),
        }
    }

    pub(crate) fn log(&self) -> &[String] {
        &self.log
    }

    pub(crate) fn set_transport(&mut self, transport: Box<dyn PtyTransport>) {
        self.transport = Some(transport);
    }

    #[cfg(test)]
    pub(crate) fn delete_findings(&self) -> Vec<String> {
        match &self.op {
            Op::Delete { form } => form.findings.clone(),
            _ => vec![],
        }
    }

    pub(crate) fn set_config_append(&mut self, worker: Box<dyn ConfigAppend>) {
        self.config_append = Some(worker);
    }

    pub(crate) fn set_inspect_worker(
        &mut self,
        worker: Box<dyn InspectWorker>,
    ) {
        self.inspect_worker = Some(worker);
    }

    pub(crate) fn set_mutate_worker(&mut self, worker: Box<dyn MutateWorker>) {
        self.mutate_worker = Some(worker);
    }

    pub(crate) fn set_delete_plan(
        &mut self,
        class: DeleteClass,
        strategy: DeleteStrategy,
        needs_fetch: bool,
    ) {
        let path = {
            let Op::Delete { form } = &mut self.op else {
                return;
            };
            form.class = delete_flow::class_label(class).into();
            form.strategy = delete_flow::strategy_label(strategy).into();
            form.confirm = delete_flow::confirm_for(strategy);
            form.path.clone()
        };
        self.delete_job = Some(DeleteJob {
            path,
            class,
            strategy,
            needs_fetch,
            phase: DeletePtyPhase::Idle,
        });
    }

    pub(crate) fn outcome(&self) -> Option<&DialogOutcome> {
        self.outcome.as_ref()
    }

    pub(crate) fn stage_n(&self) -> usize {
        match self.op {
            Op::Clone { .. } => 4,
            Op::Delete { .. } => 5,
        }
    }

    pub(crate) fn current_stage(&self) -> usize {
        match &self.op {
            Op::Clone { form, .. } => form.stage.index(),
            Op::Delete { form } => form.stage.index(),
        }
    }

    pub(crate) fn stage_summary(&self, i: usize) -> String {
        let cur = self.current_stage();
        if i > cur {
            return "waiting".into();
        }
        match &self.op {
            Op::Clone { form, .. } => match i {
                0 => {
                    if form.source.text().is_empty()
                        && form.dest.text().is_empty()
                    {
                        "empty".into()
                    } else {
                        let dest = self
                            .abs_dest()
                            .map(|a| a.to_string())
                            .unwrap_or_else(|_| form.dest.text().to_string());
                        format!(
                            "{} → {}",
                            trunc_summary(form.source.text(), 18),
                            dest
                        )
                    }
                }
                1 => {
                    if i == cur {
                        match form.auth {
                            CloneAuth::HostKey => "host key (native)".into(),
                            CloneAuth::Passphrase => {
                                "passphrase (native)".into()
                            }
                            CloneAuth::Username => "username (native)".into(),
                            CloneAuth::None => "waiting for git".into(),
                        }
                    } else {
                        "host key accepted".into()
                    }
                }
                2 => {
                    if i == cur {
                        match self.cancel {
                            CancelState::Grace => "cancelling…".into(),
                            CancelState::ForceReady => {
                                "force stop available".into()
                            }
                            CancelState::Idle => "cloning".into(),
                        }
                    } else {
                        "clone finished".into()
                    }
                }
                _ => match &self.outcome {
                    Some(DialogOutcome::Failed { .. }) => "error".into(),
                    Some(DialogOutcome::Completed {
                        config_error: Some(_),
                        ..
                    })
                    | Some(DialogOutcome::Completed {
                        refresh_error: Some(_),
                        ..
                    }) => "error".into(),
                    _ => String::new(),
                },
            },
            Op::Delete { form } => match i {
                0 => format!("{} · {}", form.path, form.strategy),
                1 => {
                    if i == cur {
                        match self.cancel {
                            CancelState::Grace => "cancelling fetch…".into(),
                            CancelState::ForceReady => {
                                "force stop available".into()
                            }
                            CancelState::Idle => "fetching".into(),
                        }
                    } else {
                        "fetch performed".into()
                    }
                }
                2 => {
                    if form.blocked {
                        if i == cur {
                            "hard blocker".into()
                        } else {
                            "blocked".into()
                        }
                    } else if i == cur {
                        "overridable warnings".into()
                    } else {
                        "warnings accepted".into()
                    }
                }
                3 => form.strategy.clone(),
                _ => {
                    if matches!(
                        self.outcome,
                        Some(DialogOutcome::Failed { .. })
                    ) {
                        "error".into()
                    } else if i == cur {
                        match self.cancel {
                            CancelState::Grace => "cancelling…".into(),
                            CancelState::ForceReady => {
                                "force stop available".into()
                            }
                            CancelState::Idle => form.strategy.clone(),
                        }
                    } else {
                        String::new()
                    }
                }
            },
        }
    }

    pub(crate) fn stage_title(&self, i: usize) -> &'static str {
        match &self.op {
            Op::Clone { .. } => match i {
                0 => CloneStage::SourceDest.title(),
                1 => CloneStage::Authenticate.title(),
                2 => CloneStage::Clone.title(),
                _ => CloneStage::Result.title(),
            },
            Op::Delete { .. } => match i {
                0 => DeleteStage::Target.title(),
                1 => DeleteStage::RemoteVerification.title(),
                2 => DeleteStage::Findings.title(),
                3 => DeleteStage::Confirm.title(),
                _ => DeleteStage::Delete.title(),
            },
        }
    }

    pub(crate) fn git_started(&self) -> bool {
        match &self.op {
            Op::Clone { form, .. } => form.git_started(),
            Op::Delete { form } => form.git_started(),
        }
    }

    pub(crate) fn running(&self) -> bool {
        match &self.op {
            Op::Clone { form, .. } => form.running,
            Op::Delete { form } => form.running,
        }
    }

    pub(crate) fn selected_stage(&self) -> usize {
        self.selected_stage
    }

    pub(crate) fn inspect(&self) -> Option<usize> {
        self.inspect
    }

    pub(crate) fn add_parent(&self) -> bool {
        match &self.op {
            Op::Clone { form, .. } => form.add_parent,
            Op::Delete { .. } => false,
        }
    }

    pub(crate) fn source(&self) -> &str {
        match &self.op {
            Op::Clone { form, .. } => form.source.text(),
            Op::Delete { .. } => "",
        }
    }

    pub(crate) fn dest(&self) -> &str {
        match &self.op {
            Op::Clone { form, .. } => form.dest.text(),
            Op::Delete { .. } => "",
        }
    }

    pub(crate) fn prompt(&self) -> &str {
        match &self.op {
            Op::Clone { form, .. } => form.prompt.text(),
            Op::Delete { form } => form.prompt.text(),
        }
    }

    pub(crate) fn perm(&self) -> &str {
        match &self.op {
            Op::Clone { .. } => "",
            Op::Delete { form } => form.perm.text(),
        }
    }

    pub(crate) fn cursor(&self) -> usize {
        self.active_field().map(Field::cursor).unwrap_or(0)
    }

    pub(crate) fn items(&self) -> Vec<FocusItem> {
        if self.sticky() {
            return vec![FocusItem::Ack];
        }
        if self.awaiting_config || self.awaiting_inspect || self.awaiting_mutate
        {
            return vec![];
        }
        if self.running() {
            return match self.cancel {
                CancelState::Idle => {
                    let mut v = Vec::new();
                    if let Op::Clone { form, .. } = &self.op {
                        match form.auth {
                            CloneAuth::Passphrase | CloneAuth::Username => {
                                v.push(FocusItem::Prompt);
                                v.push(FocusItem::Action);
                            }
                            CloneAuth::HostKey => {
                                v.push(FocusItem::RejectKey);
                                v.push(FocusItem::AcceptKey);
                            }
                            CloneAuth::None => {}
                        }
                    }
                    v.push(FocusItem::RequestCancel);
                    v
                }
                CancelState::Grace => vec![],
                CancelState::ForceReady => vec![FocusItem::ForceStop],
            };
        }
        match &self.op {
            Op::Clone { form, .. } => match form.stage {
                CloneStage::SourceDest => {
                    let mut v = vec![FocusItem::Source, FocusItem::Dest];
                    if self.show_add_parent() {
                        v.push(FocusItem::AddParent);
                    }
                    v.push(FocusItem::Cancel);
                    v.push(FocusItem::Action);
                    v
                }
                CloneStage::Result => vec![FocusItem::Ack],
                CloneStage::Authenticate | CloneStage::Clone => vec![],
            },
            Op::Delete { form } => match form.stage {
                DeleteStage::Target => {
                    vec![FocusItem::Cancel, FocusItem::Action]
                }
                DeleteStage::Findings if form.blocked => {
                    vec![FocusItem::Warnings, FocusItem::Ack]
                }
                DeleteStage::Findings => {
                    vec![
                        FocusItem::Warnings,
                        FocusItem::Cancel,
                        FocusItem::Action,
                    ]
                }
                DeleteStage::Confirm => match form.confirm {
                    DeleteConfirm::Permanent => {
                        vec![
                            FocusItem::PermPath,
                            FocusItem::Cancel,
                            FocusItem::Action,
                        ]
                    }
                    DeleteConfirm::Trash | DeleteConfirm::Worktree => {
                        vec![FocusItem::Cancel, FocusItem::Action]
                    }
                },
                DeleteStage::RemoteVerification | DeleteStage::Delete => {
                    vec![]
                }
            },
        }
    }

    pub(crate) fn item(&self) -> Option<FocusItem> {
        self.items().get(self.focus).copied()
    }

    pub(crate) fn clone_validation_error(&self) -> Option<String> {
        let Op::Clone { form, probe } = &self.op else {
            return None;
        };
        if form.source.text().trim().is_empty() {
            return Some("source is required".into());
        }
        if form.dest.text().trim().is_empty() {
            return Some("destination is required".into());
        }
        match self.abs_dest() {
            Err(e) => Some(e.to_string()),
            Ok(abs) if probe.exists(&abs) => {
                Some(format!("destination already exists: {abs}"))
            }
            Ok(_) => None,
        }
    }

    pub(crate) fn handle_key(
        &mut self,
        key: KeyEvent,
    ) -> Option<DialogOutcome> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let typing = self.typing();
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if matches!(key.code, KeyCode::Char('g') | KeyCode::Char('G')) {
                self.request_cancel();
            }
            return None;
        }
        if self.forwards_keys() && is_pty_input(&key) {
            self.forward_key(&key);
            return None;
        }
        match key.code {
            KeyCode::Esc => self.escape(),
            KeyCode::Tab => {
                self.tab(key.modifiers.contains(KeyModifiers::SHIFT));
                None
            }
            KeyCode::BackTab => {
                self.tab(true);
                None
            }
            KeyCode::Enter => self.enter(),
            KeyCode::Up => {
                self.scroll(false);
                None
            }
            KeyCode::Down => {
                self.scroll(true);
                None
            }
            KeyCode::Left if typing => {
                if let Some(field) = self.active_field_mut() {
                    field.move_left();
                }
                None
            }
            KeyCode::Right if typing => {
                if let Some(field) = self.active_field_mut() {
                    field.move_right();
                }
                None
            }
            KeyCode::Home if typing => {
                if let Some(field) = self.active_field_mut() {
                    field.home();
                }
                None
            }
            KeyCode::End if typing => {
                if let Some(field) = self.active_field_mut() {
                    field.end();
                }
                None
            }
            KeyCode::Backspace if typing => {
                let item = self.item();
                if let Some(field) = self.active_field_mut() {
                    field.backspace();
                }
                self.after_clone_field_edit(item);
                None
            }
            KeyCode::Delete if typing => {
                let item = self.item();
                if let Some(field) = self.active_field_mut() {
                    field.delete();
                }
                self.after_clone_field_edit(item);
                None
            }
            KeyCode::Char('[') if !typing => {
                self.select_stage(false);
                None
            }
            KeyCode::Char(']') if !typing => {
                self.select_stage(true);
                None
            }
            KeyCode::Char('i') | KeyCode::Char('I') if !typing => {
                self.toggle_inspect();
                None
            }
            KeyCode::Char(' ')
                if !typing && self.item() == Some(FocusItem::AddParent) =>
            {
                self.toggle_add_parent();
                None
            }
            KeyCode::Char(' ') if !typing => {
                self.toggle_inspect();
                None
            }
            KeyCode::Char(c) if typing && !c.is_control() => {
                let item = self.item();
                if let Some(field) = self.active_field_mut() {
                    field.insert(c);
                }
                self.after_clone_field_edit(item);
                None
            }
            _ => None,
        }
    }

    fn after_clone_field_edit(&mut self, item: Option<FocusItem>) {
        let Op::Clone { form, .. } = &mut self.op else {
            return;
        };
        match item {
            Some(FocusItem::Dest) => form.dest_edited = true,
            Some(FocusItem::Source) if !form.dest_edited => {
                let text =
                    match clone::default_clone_dest_name(form.source.text()) {
                        Some(name) if form.parent.is_some() => name.to_string(),
                        Some(name) => format!("~/{name}"),
                        None => String::new(),
                    };
                form.dest.set_str(&text);
            }
            _ => {}
        }
    }

    /// Stub used by later orchestration (and tests): freeze the form
    /// and mark a child as running on the next stage. Does not bump
    /// generation.
    pub(crate) fn mark_child_started(&mut self) {
        match &mut self.op {
            Op::Clone { form, .. } => {
                form.stage = CloneStage::Authenticate;
                form.running = true;
                form.auth = CloneAuth::None;
            }
            Op::Delete { form } => {
                form.stage = DeleteStage::RemoteVerification;
                form.running = true;
            }
        }
        self.inspect = None;
        self.cancel = CancelState::Idle;
        self.grace_at = None;
        self.job_generation = self.generation;
        self.selected_stage = self.current_stage();
        self.hint.clear();
        self.focus_item(FocusItem::RequestCancel);
    }

    pub(crate) fn set_child(&mut self, session: Box<dyn PtySession>) {
        self.job_generation = self.generation;
        self.pty = Some(session);
    }

    pub(crate) fn tick(&mut self, now: Instant) {
        if self.cancel != CancelState::Grace {
            return;
        }
        let start = *self.grace_at.get_or_insert(now);
        if now.saturating_duration_since(start) >= GRACE_FOR {
            self.cancel = CancelState::ForceReady;
            self.focus_item(FocusItem::ForceStop);
        }
    }

    pub(crate) fn pump(&mut self) -> Option<DialogOutcome> {
        let mut saw_exit = false;
        let mut exit_code = None;
        let mut outputs = Vec::new();
        if let Some(pty) = &mut self.pty {
            while let Some(event) = pty.try_recv() {
                match event {
                    PtyEvent::Output(bytes) => outputs.push(bytes),
                    PtyEvent::Exit { code } => {
                        saw_exit = true;
                        exit_code = code;
                    }
                }
            }
        }
        for bytes in outputs {
            self.on_clone_output(&bytes);
        }
        if saw_exit {
            return self.on_child_exit(exit_code);
        }
        if let Some(worker) = &mut self.config_append
            && let Some(event) = worker.try_recv()
        {
            return self.on_config_event(event);
        }
        if let Some(worker) = &mut self.inspect_worker
            && let Some(event) = worker.try_recv()
        {
            return self.on_inspect_event(event);
        }
        if let Some(worker) = &mut self.mutate_worker
            && let Some(event) = worker.try_recv()
        {
            return self.on_mutate_event(event);
        }
        None
    }

    fn request_cancel(&mut self) {
        if !self.running() || self.cancel != CancelState::Idle {
            return;
        }
        if let Some(pty) = &mut self.pty {
            let _ = pty.interrupt();
        }
        self.cancel = CancelState::Grace;
        self.grace_at = None;
        self.hint = "cancelling… waiting for git to exit".into();
        self.focus = 0;
    }

    fn force_stop(&mut self) {
        if self.cancel != CancelState::ForceReady {
            return;
        }
        if let Some(pty) = &mut self.pty {
            let _ = pty.force_kill();
        }
    }

    fn on_child_exit(&mut self, code: Option<i32>) -> Option<DialogOutcome> {
        let job = self.job_generation;
        let cancelling =
            matches!(self.cancel, CancelState::Grace | CancelState::ForceReady);
        self.pty = None;
        self.set_running(false);
        self.cancel = CancelState::Idle;
        self.grace_at = None;
        if job != self.generation {
            return None;
        }
        if cancelling {
            self.clone_job = None;
            self.bump_generation();
            self.outcome = Some(DialogOutcome::Cancelled);
            return Some(DialogOutcome::Cancelled);
        }
        if self.clone_job.is_some() {
            return self.on_clone_git_exit(code);
        }
        if let Some(phase) = self.delete_job.as_ref().map(|j| j.phase) {
            return match phase {
                DeletePtyPhase::Fetch => self.on_fetch_exit(code),
                DeletePtyPhase::Worktree => self.on_worktree_exit(code),
                DeletePtyPhase::Idle => None,
            };
        }
        None
    }

    fn set_running(&mut self, running: bool) {
        match &mut self.op {
            Op::Clone { form, .. } => form.running = running,
            Op::Delete { form } => form.running = running,
        }
    }

    fn forwards_keys(&self) -> bool {
        self.running()
            && self.cancel == CancelState::Idle
            && self.clone_job.is_some()
            && matches!(
                &self.op,
                Op::Clone { form, .. } if form.auth == CloneAuth::None
            )
    }

    fn clone_auth_is_prompt(&self) -> bool {
        matches!(
            &self.op,
            Op::Clone { form, .. }
                if matches!(form.auth, CloneAuth::Passphrase | CloneAuth::Username)
        )
    }

    fn forward_key(&mut self, key: &KeyEvent) {
        match key.code {
            KeyCode::Enter => self.pty_write(b"\n"),
            KeyCode::Backspace => self.pty_write(b"\x7f"),
            KeyCode::Char(c) if !c.is_control() => {
                let mut buf = [0; 4];
                self.pty_write(c.encode_utf8(&mut buf).as_bytes());
            }
            _ => {}
        }
    }

    fn pty_write(&mut self, bytes: &[u8]) {
        if let Some(pty) = &mut self.pty {
            let _ = pty.write(bytes);
        }
        self.decoder.record_write(bytes);
    }

    fn clear_native_auth(&mut self) {
        if let Op::Clone { form, .. } = &mut self.op {
            form.auth = CloneAuth::None;
            form.stage = CloneStage::Clone;
            form.prompt = Field::new();
        }
        self.focus_item(FocusItem::RequestCancel);
    }

    fn send_native_prompt(&mut self) {
        let secret = matches!(
            &self.op,
            Op::Clone { form, .. } if form.auth == CloneAuth::Passphrase
        );
        let text = self.prompt().to_string();
        if secret {
            self.decoder.note_secret(&text);
        }
        let mut packet = text.into_bytes();
        packet.push(b'\n');
        self.pty_write(&packet);
        self.clear_native_auth();
    }

    fn begin_clone(&mut self) {
        let Op::Clone { form, .. } = &self.op else {
            return;
        };
        let source = form.source.text().trim().to_string();
        let add_parent = form.add_parent;
        let Ok(dest) = self.abs_dest() else {
            return;
        };
        self.mark_child_started();
        self.clone_job = Some(CloneJob {
            dest: dest.clone(),
            add_parent,
        });
        let argv = clone_flow::git_clone_argv(&source, &dest);
        let size = PtySize { cols: 80, rows: 24 };
        let Some(transport) = self.transport.as_mut() else {
            return;
        };
        match transport.spawn(&argv, size) {
            Ok(session) => self.set_child(session),
            Err(e) => {
                self.clone_job = None;
                self.set_running(false);
                self.present_error(e.to_string());
            }
        }
    }

    fn on_clone_output(&mut self, bytes: &[u8]) {
        if self.clone_job.is_none() {
            return;
        }
        self.decoder.feed(bytes);
        self.log = self.decoder.log().to_vec();
        let interaction = self.decoder.interaction().clone();
        self.apply_clone_interaction(&interaction);
    }

    fn apply_clone_interaction(&mut self, interaction: &Interaction) {
        let focus = {
            let Op::Clone { form, .. } = &mut self.op else {
                return;
            };
            match interaction {
                Interaction::Native {
                    kind: PromptKind::HostKey,
                    ..
                } => {
                    form.stage = CloneStage::Authenticate;
                    form.auth = CloneAuth::HostKey;
                    FocusItem::AcceptKey
                }
                Interaction::Native {
                    kind: PromptKind::Passphrase | PromptKind::Password,
                    ..
                } => {
                    form.stage = CloneStage::Authenticate;
                    form.auth = CloneAuth::Passphrase;
                    FocusItem::Prompt
                }
                Interaction::Native {
                    kind: PromptKind::Username,
                    ..
                } => {
                    form.stage = CloneStage::Authenticate;
                    form.auth = CloneAuth::Username;
                    FocusItem::Prompt
                }
                Interaction::Terminal => {
                    form.auth = CloneAuth::None;
                    form.stage = CloneStage::Clone;
                    FocusItem::RequestCancel
                }
            }
        };
        self.focus_item(focus);
    }

    fn on_clone_git_exit(
        &mut self,
        code: Option<i32>,
    ) -> Option<DialogOutcome> {
        let job = self.clone_job.clone()?;
        if code != Some(0) {
            let dest = job.dest.clone();
            self.clone_job = None;
            let err = if code.is_none() {
                clone::CloneError::GitInterrupted { dest }
            } else {
                clone::CloneError::GitFailed { dest, code }
            };
            self.present_error(err.to_string());
            return None;
        }
        self.refresh_requested = true;
        self.mutation_path = Some(job.dest.clone());
        if job.add_parent
            && let Some(worker) = &mut self.config_append
        {
            self.awaiting_config = true;
            if let Op::Clone { form, .. } = &mut self.op {
                form.stage = CloneStage::Result;
            }
            worker.begin(job.dest.clone(), self.generation);
            return None;
        }
        self.clone_job = None;
        self.present_completed(format!("cloned to `{}`", job.dest), None, None);
        if self.sticky() {
            None
        } else {
            self.outcome.clone()
        }
    }

    fn on_config_event(
        &mut self,
        event: ConfigAppendEvent,
    ) -> Option<DialogOutcome> {
        if event.generation != self.generation {
            return None;
        }
        self.awaiting_config = false;
        let dest = event.dest;
        self.clone_job = None;
        let config_error = event.result.err();
        self.present_completed(
            format!("cloned to `{dest}`"),
            config_error,
            None,
        );
        if self.sticky() {
            None
        } else {
            self.outcome.clone()
        }
    }

    fn begin_delete_preflight(&mut self) {
        let needs_fetch =
            self.delete_job.as_ref().is_some_and(|j| j.needs_fetch);
        let path = match &self.op {
            Op::Delete { form } => form.path.clone(),
            _ => return,
        };
        if let Some(job) = &mut self.delete_job {
            job.path = path.clone();
        }
        if needs_fetch {
            self.spawn_delete_fetch(&path);
        } else {
            if let Op::Delete { form } = &mut self.op {
                form.stage = DeleteStage::RemoteVerification;
            }
            self.start_inspect(None);
        }
    }

    fn spawn_delete_fetch(&mut self, path: &str) {
        self.mark_child_started();
        if let Some(job) = &mut self.delete_job {
            job.phase = DeletePtyPhase::Fetch;
        }
        let argv = delete_flow::git_fetch_argv(path);
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let size = PtySize { cols: 80, rows: 24 };
        if let Some(transport) = self.transport.as_mut() {
            match transport.spawn(&argv_ref, size) {
                Ok(session) => self.set_child(session),
                Err(e) => {
                    self.set_running(false);
                    self.present_error(e.to_string());
                }
            }
        } else {
            self.set_running(false);
            self.start_inspect(Some(FetchResult::Failed));
        }
    }

    fn start_inspect(&mut self, fetch: Option<FetchResult>) {
        let path = match &self.op {
            Op::Delete { form } => form.path.clone(),
            _ => return,
        };
        self.awaiting_inspect = true;
        self.job_generation = self.generation;
        if let Some(job) = &mut self.delete_job {
            job.phase = DeletePtyPhase::Idle;
        }
        if let Some(worker) = &mut self.inspect_worker {
            worker.begin(path, self.generation, fetch);
        } else {
            self.awaiting_inspect = false;
            self.present_findings(vec![], false);
        }
    }

    fn on_fetch_exit(&mut self, code: Option<i32>) -> Option<DialogOutcome> {
        let fetch = match code {
            Some(0) => FetchResult::Success,
            None => FetchResult::Cancelled,
            _ => FetchResult::Failed,
        };
        self.start_inspect(Some(fetch));
        None
    }

    fn on_inspect_event(
        &mut self,
        event: InspectEvent,
    ) -> Option<DialogOutcome> {
        if event.generation != self.generation {
            return None;
        }
        let path = match &self.op {
            Op::Delete { form } => form.path.clone(),
            _ => return None,
        };
        if event.path != path {
            return None;
        }
        self.awaiting_inspect = false;
        if event.fetch_required {
            self.spawn_delete_fetch(&path);
            return None;
        }
        if let Op::Delete { form } = &mut self.op {
            form.class = event.class.clone();
            form.strategy = event.strategy.clone();
            form.confirm = event.confirm;
        }
        self.present_findings(event.findings, event.blocked);
        None
    }

    fn goto_delete_confirm(&mut self) {
        let confirm = self
            .delete_job
            .as_ref()
            .map(|j| delete_flow::confirm_for(j.strategy))
            .or(match &self.op {
                Op::Delete { form } => Some(form.confirm),
                _ => None,
            });
        match confirm {
            Some(DeleteConfirm::Permanent) => self.show_permanent_confirm(),
            Some(DeleteConfirm::Worktree) => self.show_worktree_confirm(),
            Some(DeleteConfirm::Trash) | None => self.show_trash_confirm(),
        }
    }

    fn begin_delete_mutate(&mut self) {
        let (path, confirm, perm) = match &self.op {
            Op::Delete { form } => (
                form.path.clone(),
                form.confirm,
                form.perm.text().to_string(),
            ),
            _ => return,
        };
        match confirm {
            DeleteConfirm::Permanent if perm.trim() != path => (),
            DeleteConfirm::Worktree => {
                self.mark_child_started();
                if let Op::Delete { form } = &mut self.op {
                    form.stage = DeleteStage::Delete;
                }
                if let Some(job) = &mut self.delete_job {
                    job.phase = DeletePtyPhase::Worktree;
                }
                let argv = delete_flow::git_worktree_remove_argv(&path);
                let argv_ref: Vec<&str> =
                    argv.iter().map(String::as_str).collect();
                let size = PtySize { cols: 80, rows: 24 };
                if let Some(transport) = self.transport.as_mut() {
                    match transport.spawn(&argv_ref, size) {
                        Ok(session) => self.set_child(session),
                        Err(e) => {
                            self.set_running(false);
                            self.present_error(e.to_string());
                        }
                    }
                }
            }
            DeleteConfirm::Trash | DeleteConfirm::Permanent => {
                let kind = if confirm == DeleteConfirm::Permanent {
                    MutateKind::Permanent
                } else {
                    MutateKind::Trash
                };
                self.awaiting_mutate = true;
                self.job_generation = self.generation;
                if let Op::Delete { form } = &mut self.op {
                    form.stage = DeleteStage::Delete;
                }
                if let Some(worker) = &mut self.mutate_worker {
                    worker.begin(path, kind, self.generation);
                } else {
                    self.awaiting_mutate = false;
                    self.present_error("mutate worker missing".to_string());
                }
            }
        }
    }

    fn on_worktree_exit(&mut self, code: Option<i32>) -> Option<DialogOutcome> {
        let path = self
            .delete_job
            .as_ref()
            .map(|j| j.path.clone())
            .unwrap_or_default();
        self.delete_job = None;
        if code == Some(0) {
            self.refresh_requested = true;
            self.mutation_path = Some(path.clone());
            self.present_completed(format!("deleted `{path}`"), None, None);
            if self.sticky() {
                None
            } else {
                self.outcome.clone()
            }
        } else {
            self.present_error(format!(
                "git worktree deletion failed for `{path}`"
            ));
            None
        }
    }

    fn on_mutate_event(&mut self, event: MutateEvent) -> Option<DialogOutcome> {
        if event.generation != self.generation {
            return None;
        }
        self.awaiting_mutate = false;
        match event.result {
            Ok(_) => {
                self.delete_job = None;
                self.refresh_requested = true;
                self.mutation_path = Some(event.path.clone());
                self.present_completed(
                    format!("deleted `{}`", event.path),
                    None,
                    None,
                );
                if self.sticky() {
                    None
                } else {
                    self.outcome.clone()
                }
            }
            Err(MutateFail::Trash(cause)) => {
                self.hint = format!("trash failed: {cause}");
                self.show_permanent_confirm();
                None
            }
            Err(MutateFail::Permanent(cause)) => {
                self.present_error(format!(
                    "permanent deletion failed: {cause}"
                ));
                None
            }
            Err(MutateFail::IdentityChanged) => {
                self.present_error(format!(
                    "path identity or type changed: `{}`",
                    event.path
                ));
                None
            }
        }
    }

    pub(crate) fn show_native_prompt(&mut self) {
        if let Op::Clone { form, .. } = &mut self.op {
            form.stage = CloneStage::Authenticate;
            form.running = true;
            form.auth = CloneAuth::Passphrase;
            form.prompt = Field::new();
        }
        self.selected_stage = self.current_stage();
        self.focus_item(FocusItem::Prompt);
    }

    pub(crate) fn show_host_key(&mut self) {
        if let Op::Clone { form, .. } = &mut self.op {
            form.stage = CloneStage::Authenticate;
            form.running = true;
            form.auth = CloneAuth::HostKey;
        }
        self.selected_stage = self.current_stage();
        self.focus_item(FocusItem::AcceptKey);
    }

    pub(crate) fn set_log(&mut self, log: Vec<String>) {
        self.log = log;
    }

    pub(crate) fn present_findings(
        &mut self,
        findings: Vec<String>,
        blocked: bool,
    ) {
        if let Op::Delete { form } = &mut self.op {
            form.stage = DeleteStage::Findings;
            form.running = false;
            form.blocked = blocked;
            form.findings = findings;
        }
        self.outcome = if blocked {
            Some(DialogOutcome::Failed {
                message: "hard blocker".into(),
            })
        } else {
            None
        };
        self.selected_stage = self.current_stage();
        if blocked {
            self.focus_item(FocusItem::Ack);
        } else {
            self.focus_item(FocusItem::Action);
        }
    }

    pub(crate) fn show_trash_confirm(&mut self) {
        if let Op::Delete { form } = &mut self.op {
            form.stage = DeleteStage::Confirm;
            form.confirm = DeleteConfirm::Trash;
            form.running = false;
            form.strategy = "trash".into();
        }
        self.outcome = None;
        self.selected_stage = self.current_stage();
        self.focus_item(FocusItem::Action);
    }

    pub(crate) fn show_worktree_confirm(&mut self) {
        if let Op::Delete { form } = &mut self.op {
            form.stage = DeleteStage::Confirm;
            form.confirm = DeleteConfirm::Worktree;
            form.running = false;
            form.class = "linked worktree".into();
            form.strategy = "git worktree".into();
        }
        self.outcome = None;
        self.selected_stage = self.current_stage();
        self.focus_item(FocusItem::Action);
    }

    pub(crate) fn show_permanent_confirm(&mut self) {
        if let Op::Delete { form } = &mut self.op {
            form.stage = DeleteStage::Confirm;
            form.confirm = DeleteConfirm::Permanent;
            form.running = false;
            form.blocked = false;
            form.perm = Field::new();
        }
        self.outcome = None;
        self.selected_stage = self.current_stage();
        self.focus_item(FocusItem::PermPath);
    }

    pub(crate) fn present_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        match &mut self.op {
            Op::Clone { form, .. } => {
                form.stage = CloneStage::Result;
                form.running = false;
            }
            Op::Delete { form } => {
                form.stage = DeleteStage::Delete;
                form.running = false;
            }
        }
        self.outcome = Some(DialogOutcome::Failed { message });
        self.selected_stage = self.current_stage();
        self.focus_item(FocusItem::Ack);
    }

    pub(crate) fn present_completed(
        &mut self,
        summary: impl Into<String>,
        config_error: Option<String>,
        refresh_error: Option<String>,
    ) {
        let outcome = DialogOutcome::Completed {
            summary: summary.into(),
            config_error,
            refresh_error,
        };
        match &mut self.op {
            Op::Clone { form, .. } => {
                form.stage = CloneStage::Result;
                form.running = false;
            }
            Op::Delete { form } => {
                form.stage = DeleteStage::Delete;
                form.running = false;
            }
        }
        self.outcome = Some(outcome);
        self.selected_stage = self.current_stage();
        if self.sticky() {
            self.focus_item(FocusItem::Ack);
        }
    }

    pub(crate) fn present_blocker(&mut self) {
        if let Op::Delete { form } = &mut self.op {
            form.stage = DeleteStage::Findings;
            form.blocked = true;
            form.running = false;
        }
        self.outcome = Some(DialogOutcome::Failed {
            message: "hard blocker".into(),
        });
        self.selected_stage = self.current_stage();
        self.focus_item(FocusItem::Ack);
    }

    fn sticky(&self) -> bool {
        self.outcome.as_ref().is_some_and(DialogOutcome::needs_ack)
    }

    fn typing(&self) -> bool {
        matches!(
            self.item(),
            Some(
                FocusItem::Source
                    | FocusItem::Dest
                    | FocusItem::Prompt
                    | FocusItem::PermPath
            )
        )
    }

    fn show_add_parent(&self) -> bool {
        let Op::Clone { probe, .. } = &self.op else {
            return false;
        };
        match self.abs_dest() {
            Ok(abs) if !abs.is_empty() => !probe.covered(&abs),
            _ => false,
        }
    }

    fn abs_dest(&self) -> Result<String, clone::CloneError> {
        let Op::Clone { form, probe } = &self.op else {
            return Err(clone::CloneError::NotAbsolute(String::new()));
        };
        clone::resolve_destination(
            form.dest.text(),
            form.parent.as_deref().map(Path::new),
            &|name| probe.env(name),
        )
    }

    fn active_field(&self) -> Option<&Field> {
        match (&self.op, self.item()) {
            (Op::Clone { form, .. }, Some(FocusItem::Source)) => {
                Some(&form.source)
            }
            (Op::Clone { form, .. }, Some(FocusItem::Dest)) => Some(&form.dest),
            (Op::Clone { form, .. }, Some(FocusItem::Prompt)) => {
                Some(&form.prompt)
            }
            (Op::Delete { form }, Some(FocusItem::Prompt)) => {
                Some(&form.prompt)
            }
            (Op::Delete { form }, Some(FocusItem::PermPath)) => {
                Some(&form.perm)
            }
            _ => None,
        }
    }

    fn active_field_mut(&mut self) -> Option<&mut Field> {
        let item = self.item();
        match (&mut self.op, item) {
            (Op::Clone { form, .. }, Some(FocusItem::Source)) => {
                Some(&mut form.source)
            }
            (Op::Clone { form, .. }, Some(FocusItem::Dest)) => {
                Some(&mut form.dest)
            }
            (Op::Clone { form, .. }, Some(FocusItem::Prompt)) => {
                Some(&mut form.prompt)
            }
            (Op::Delete { form }, Some(FocusItem::Prompt)) => {
                Some(&mut form.prompt)
            }
            (Op::Delete { form }, Some(FocusItem::PermPath)) => {
                Some(&mut form.perm)
            }
            _ => None,
        }
    }

    fn focus_item(&mut self, want: FocusItem) {
        if let Some(i) = self.items().iter().position(|x| *x == want) {
            self.focus = i;
            if let Some(field) = self.active_field_mut() {
                field.end();
            }
        } else {
            self.clamp_focus();
        }
    }

    fn clamp_focus(&mut self) {
        let n = self.items().len();
        if n == 0 {
            self.focus = 0;
        } else if self.focus >= n {
            self.focus = n - 1;
        }
        if let Some(field) = self.active_field_mut() {
            field.set_cursor(field.cursor());
        }
    }

    fn tab(&mut self, back: bool) {
        let n = self.items().len();
        if n == 0 {
            return;
        }
        if back {
            self.focus = if self.focus == 0 {
                n - 1
            } else {
                self.focus - 1
            };
        } else {
            self.focus = (self.focus + 1) % n;
        }
        if let Some(field) = self.active_field_mut() {
            field.end();
        }
    }

    fn toggle_add_parent(&mut self) {
        if self.git_started() || !self.show_add_parent() {
            return;
        }
        if let Op::Clone { form, .. } = &mut self.op {
            form.add_parent = !form.add_parent;
        }
    }

    fn select_stage(&mut self, next: bool) {
        let n = self.stage_n();
        if n == 0 {
            return;
        }
        if next {
            self.selected_stage = (self.selected_stage + 1) % n;
        } else {
            self.selected_stage = (self.selected_stage + n - 1) % n;
        }
    }

    fn toggle_inspect(&mut self) {
        let i = self.selected_stage;
        if i >= self.current_stage() {
            return;
        }
        if self.inspect == Some(i) {
            self.inspect = None;
        } else {
            self.inspect = Some(i);
            self.view_scroll = 0;
        }
    }

    fn scroll(&mut self, down: bool) {
        if down {
            self.view_scroll = self.view_scroll.saturating_add(1);
        } else {
            self.view_scroll = self.view_scroll.saturating_sub(1);
        }
    }

    fn enter(&mut self) -> Option<DialogOutcome> {
        match self.item() {
            Some(FocusItem::AddParent) => {
                self.toggle_add_parent();
                None
            }
            Some(FocusItem::Cancel) => self.escape(),
            Some(FocusItem::Ack) => self.acknowledge(),
            Some(FocusItem::RequestCancel) => {
                self.request_cancel();
                None
            }
            Some(FocusItem::AcceptKey) => {
                self.pty_write(b"yes\n");
                self.clear_native_auth();
                None
            }
            Some(FocusItem::RejectKey) => {
                self.pty_write(b"no\n");
                self.clear_native_auth();
                None
            }
            Some(FocusItem::Action) if self.clone_auth_is_prompt() => {
                self.send_native_prompt();
                None
            }
            Some(FocusItem::Action)
            | Some(FocusItem::Source)
            | Some(FocusItem::Dest)
            | Some(FocusItem::Prompt)
            | Some(FocusItem::PermPath)
            | Some(FocusItem::Warnings)
            | None => self.advance(),
            Some(FocusItem::ForceStop) => {
                self.force_stop();
                None
            }
        }
    }

    fn advance(&mut self) -> Option<DialogOutcome> {
        match &self.op {
            Op::Clone { form, .. } => {
                if form.stage == CloneStage::SourceDest {
                    if self.clone_validation_error().is_some() {
                        return None;
                    }
                    self.begin_clone();
                }
            }
            Op::Delete { form } => {
                let stage = form.stage;
                let blocked = form.blocked;
                match stage {
                    DeleteStage::Target => self.begin_delete_preflight(),
                    DeleteStage::Findings if !blocked => {
                        self.goto_delete_confirm();
                    }
                    DeleteStage::Confirm => self.begin_delete_mutate(),
                    _ => {}
                }
            }
        }
        None
    }

    fn escape(&mut self) -> Option<DialogOutcome> {
        if self.awaiting_config {
            self.hint = "clone already succeeded; waiting for config".into();
            return None;
        }
        if self.awaiting_mutate {
            self.hint = "deletion in progress".into();
            return None;
        }
        if self.awaiting_inspect {
            self.awaiting_inspect = false;
            self.bump_generation();
            self.outcome = Some(DialogOutcome::Cancelled);
            return Some(DialogOutcome::Cancelled);
        }
        if self.running() {
            self.hint =
                "Esc does not stop git — Ctrl-G to request cancel".into();
            return None;
        }
        if self.sticky() {
            self.hint = "error/blocker stays until acknowledged".into();
            return None;
        }
        self.bump_generation();
        self.outcome = Some(DialogOutcome::Cancelled);
        Some(DialogOutcome::Cancelled)
    }

    fn acknowledge(&mut self) -> Option<DialogOutcome> {
        self.bump_generation();
        self.outcome.take()
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    fn action_label(&self) -> String {
        match &self.op {
            Op::Clone { form, .. } => match form.stage {
                CloneStage::Authenticate | CloneStage::Clone => "Send".into(),
                CloneStage::SourceDest
                    if self.clone_validation_error().is_some() =>
                {
                    "Clone (invalid)".into()
                }
                _ => "Clone".into(),
            },
            Op::Delete { form } => match form.stage {
                DeleteStage::Target => "Preflight".into(),
                DeleteStage::Findings => "Accept".into(),
                DeleteStage::Confirm => match form.confirm {
                    DeleteConfirm::Trash => "Move to trash".into(),
                    DeleteConfirm::Worktree => "Delete worktree".into(),
                    DeleteConfirm::Permanent => "Delete permanently".into(),
                },
                _ => "Continue".into(),
            },
        }
    }
}

fn is_pty_input(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete => true,
        KeyCode::Char(c) if !c.is_control() => {
            !matches!(c, '[' | ']' | 'i' | 'I' | ' ')
        }
        _ => false,
    }
}

fn trunc_summary(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

#[cfg(test)]
#[path = "action_tests.rs"]
mod tests;
