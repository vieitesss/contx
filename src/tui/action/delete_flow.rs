//! Picker delete orchestration: PTY fetch/worktree, off-thread inspect
//! and trash/permanent mutate. Does not call `delete::run`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::delete::{
    DeleteClass, DeleteStrategy, FetchResult, Fetcher, Preflight,
};

use super::state::DeleteConfirm;

pub(crate) fn git_fetch_argv(path: &str) -> Vec<String> {
    vec![
        "git".into(),
        "-C".into(),
        path.into(),
        "fetch".into(),
        "--all".into(),
        "--prune".into(),
    ]
}

pub(crate) fn git_worktree_remove_argv(path: &str) -> Vec<String> {
    vec![
        "git".into(),
        "-C".into(),
        path.into(),
        "worktree".into(),
        "remove".into(),
        path.into(),
    ]
}

pub(crate) fn confirm_for(strategy: DeleteStrategy) -> DeleteConfirm {
    match strategy {
        DeleteStrategy::Trash => DeleteConfirm::Trash,
        DeleteStrategy::Permanent => DeleteConfirm::Permanent,
        DeleteStrategy::GitWorktree => DeleteConfirm::Worktree,
    }
}

pub(crate) fn class_label(class: DeleteClass) -> &'static str {
    match class {
        DeleteClass::Symlink => "symlink",
        DeleteClass::LinkedWorktree => "linked worktree",
        DeleteClass::StandaloneRepo => "standalone repository",
        DeleteClass::OrdinaryDirectory => "ordinary directory",
    }
}

pub(crate) fn strategy_label(strategy: DeleteStrategy) -> &'static str {
    match strategy {
        DeleteStrategy::Trash => "trash",
        DeleteStrategy::Permanent => "permanent",
        DeleteStrategy::GitWorktree => "git worktree",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeletePtyPhase {
    Idle,
    Fetch,
    Worktree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteJob {
    pub path: String,
    pub class: DeleteClass,
    pub strategy: DeleteStrategy,
    pub needs_fetch: bool,
    pub phase: DeletePtyPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectEvent {
    pub generation: u64,
    pub path: String,
    pub findings: Vec<String>,
    pub blocked: bool,
    pub class: String,
    pub strategy: String,
    pub confirm: DeleteConfirm,
    pub fetch_required: bool,
}

pub(crate) trait InspectWorker {
    fn begin(
        &mut self,
        path: String,
        generation: u64,
        fetch: Option<FetchResult>,
    );
    fn try_recv(&mut self) -> Option<InspectEvent>;
}

#[derive(Clone)]
pub(crate) struct FakeInspectWorker {
    inner: Rc<RefCell<FakeInspectInner>>,
}

struct FakeInspectInner {
    begins: Vec<(String, u64, Option<FetchResult>)>,
    events: VecDeque<InspectEvent>,
}

impl FakeInspectWorker {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(FakeInspectInner {
                begins: vec![],
                events: VecDeque::new(),
            })),
        }
    }

    pub(crate) fn begins(&self) -> Vec<(String, u64, Option<FetchResult>)> {
        self.inner.borrow().begins.clone()
    }

    pub(crate) fn inject(&self, event: InspectEvent) {
        self.inner.borrow_mut().events.push_back(event);
    }
}

impl InspectWorker for FakeInspectWorker {
    fn begin(
        &mut self,
        path: String,
        generation: u64,
        fetch: Option<FetchResult>,
    ) {
        self.inner
            .borrow_mut()
            .begins
            .push((path, generation, fetch));
    }

    fn try_recv(&mut self) -> Option<InspectEvent> {
        self.inner.borrow_mut().events.pop_front()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MutateKind {
    Trash,
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutateEvent {
    pub generation: u64,
    pub path: String,
    pub result: Result<DeleteStrategy, MutateFail>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MutateFail {
    Trash(String),
    Permanent(String),
    IdentityChanged,
}

pub(crate) trait MutateWorker {
    fn begin(&mut self, path: String, kind: MutateKind, generation: u64);
    fn try_recv(&mut self) -> Option<MutateEvent>;
}

#[derive(Clone)]
pub(crate) struct FakeMutateWorker {
    inner: Rc<RefCell<FakeMutateInner>>,
}

struct FakeMutateInner {
    begins: Vec<(String, MutateKind, u64)>,
    events: VecDeque<MutateEvent>,
}

impl FakeMutateWorker {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(FakeMutateInner {
                begins: vec![],
                events: VecDeque::new(),
            })),
        }
    }

    pub(crate) fn begins(&self) -> Vec<(String, MutateKind, u64)> {
        self.inner.borrow().begins.clone()
    }

    pub(crate) fn inject(&self, event: MutateEvent) {
        self.inner.borrow_mut().events.push_back(event);
    }
}

impl MutateWorker for FakeMutateWorker {
    fn begin(&mut self, path: String, kind: MutateKind, generation: u64) {
        self.inner
            .borrow_mut()
            .begins
            .push((path, kind, generation));
    }

    fn try_recv(&mut self) -> Option<MutateEvent> {
        self.inner.borrow_mut().events.pop_front()
    }
}

/// Fetcher that returns a previously obtained PTY fetch result.
pub(crate) struct RecordedFetch(pub Option<FetchResult>);

impl Fetcher for RecordedFetch {
    fn fetch_all_prune(&mut self, _root: &str) -> FetchResult {
        self.0.unwrap_or(FetchResult::Success)
    }
}

pub(crate) fn inspect_event_from_preflight(
    generation: u64,
    pf: Preflight,
) -> InspectEvent {
    let blocked = pf.is_blocked();
    let mut findings = Vec::new();
    if blocked {
        findings.push("HARD BLOCKER — cannot continue".into());
        for b in &pf.blockers {
            findings.push(b.to_string());
        }
    } else {
        findings.push("Overridable warnings — Enter accepts".into());
        for w in &pf.warnings {
            findings.push(format!("• {w}"));
        }
    }
    let (class, strategy, confirm) = match &pf.target {
        Some(t) => (
            class_label(t.class).into(),
            strategy_label(t.strategy).into(),
            confirm_for(t.strategy),
        ),
        None => ("unknown".into(), "trash".into(), DeleteConfirm::Trash),
    };
    InspectEvent {
        generation,
        path: pf.path,
        findings,
        blocked,
        class,
        strategy,
        confirm,
        fetch_required: false,
    }
}

#[cfg(test)]
#[path = "delete_flow_tests.rs"]
mod tests;
