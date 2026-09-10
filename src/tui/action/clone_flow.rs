//! Picker clone orchestration: PTY `git clone`, native prompts, and
//! off-thread config append. Does not call `clone::run`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub(crate) fn git_clone_argv<'a>(
    source: &'a str,
    dest: &'a str,
) -> [&'a str; 4] {
    ["git", "clone", source, dest]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloneJob {
    pub dest: String,
    pub add_parent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigAppendEvent {
    pub generation: u64,
    pub dest: String,
    pub result: Result<(), String>,
}

pub(crate) trait ConfigAppend {
    fn begin(&mut self, dest: String, generation: u64);
    fn try_recv(&mut self) -> Option<ConfigAppendEvent>;
}

/// Scripted config-append worker for tests. `begin` only records;
/// the test injects the result later (never a blocking write).
#[derive(Clone)]
pub(crate) struct FakeConfigAppend {
    inner: Rc<RefCell<FakeConfigInner>>,
}

struct FakeConfigInner {
    begins: Vec<(String, u64)>,
    events: VecDeque<ConfigAppendEvent>,
}

impl FakeConfigAppend {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(FakeConfigInner {
                begins: vec![],
                events: VecDeque::new(),
            })),
        }
    }

    pub(crate) fn begins(&self) -> Vec<(String, u64)> {
        self.inner.borrow().begins.clone()
    }

    pub(crate) fn inject(&self, event: ConfigAppendEvent) {
        self.inner.borrow_mut().events.push_back(event);
    }
}

impl ConfigAppend for FakeConfigAppend {
    fn begin(&mut self, dest: String, generation: u64) {
        self.inner.borrow_mut().begins.push((dest, generation));
    }

    fn try_recv(&mut self) -> Option<ConfigAppendEvent> {
        self.inner.borrow_mut().events.pop_front()
    }
}

#[cfg(test)]
#[path = "clone_flow_tests.rs"]
mod tests;
