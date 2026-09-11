//! Off-thread catalog refresh and production clone/delete workers.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::config::{ResolvedConfig, SessionCandidate};
use crate::delete::{
    DeleteClass, DeleteRequest, DeleteStrategy, DeleteTarget, FetchResult,
    classify, has_configured_remotes, permanent_delete, preflight, revalidate,
};
use crate::mux::pane_cwds;
use crate::tui::action::{
    CloneDestProbe, ConfigAppend, ConfigAppendEvent, DeleteConfirm,
    InspectEvent, InspectWorker, MutateEvent, MutateFail, MutateKind,
    MutateWorker, RecordedFetch, inspect_event_from_preflight,
};

pub(crate) struct FsCloneProbe {
    pub config: Option<ResolvedConfig>,
}

impl CloneDestProbe for FsCloneProbe {
    fn exists(&self, abs: &str) -> bool {
        crate::clone::dest_exists(abs)
    }

    fn covered(&self, abs: &str) -> bool {
        let Some(cfg) = &self.config else {
            return true;
        };
        cfg.destination_covered(abs, &|n| env::var_os(n))
            .unwrap_or(true)
    }

    fn env(&self, name: &str) -> Option<OsString> {
        env::var_os(name)
    }
}

pub(crate) struct ThreadedConfigAppend {
    tx: Sender<ConfigAppendEvent>,
    rx: Receiver<ConfigAppendEvent>,
    config: ResolvedConfig,
}

impl ThreadedConfigAppend {
    pub(crate) fn new(config: ResolvedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, config }
    }
}

impl ConfigAppend for ThreadedConfigAppend {
    fn begin(&mut self, dest: String, generation: u64) {
        let tx = self.tx.clone();
        let config = self.config.clone();
        thread::spawn(move || {
            let result = config
                .append_parent_to_paths(&dest, &|n| env::var_os(n))
                .map_err(|e| e.to_string());
            let _ = tx.send(ConfigAppendEvent {
                generation,
                dest,
                result,
            });
        });
    }

    fn try_recv(&mut self) -> Option<ConfigAppendEvent> {
        self.rx.try_recv().ok()
    }
}

pub(crate) struct ThreadedInspect {
    tx: Sender<InspectEvent>,
    rx: Receiver<InspectEvent>,
    candidates: Vec<SessionCandidate>,
    process_cwd: PathBuf,
    multiplexer: crate::config::Multiplexer,
    permanent_delete: bool,
}

impl ThreadedInspect {
    pub(crate) fn new(config: &ResolvedConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            candidates: config.candidates.clone(),
            process_cwd: env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("/")),
            multiplexer: config.multiplexer,
            permanent_delete: config.permanent_delete,
        }
    }
}

impl InspectWorker for ThreadedInspect {
    fn begin(
        &mut self,
        path: String,
        generation: u64,
        fetch: Option<FetchResult>,
    ) {
        let tx = self.tx.clone();
        let candidates = self.candidates.clone();
        let process_cwd = self.process_cwd.clone();
        let multiplexer = self.multiplexer;
        let permanent_delete = self.permanent_delete;
        thread::spawn(move || {
            if fetch.is_none()
                && classify(&path).ok() == Some(DeleteClass::StandaloneRepo)
                && has_configured_remotes(&path)
            {
                let _ = tx.send(InspectEvent {
                    generation,
                    path,
                    findings: vec![],
                    blocked: false,
                    class: "standalone repository".into(),
                    strategy: "trash".into(),
                    confirm: DeleteConfirm::Trash,
                    fetch_required: true,
                });
                return;
            }
            let panes = pane_cwds(multiplexer);
            let request = DeleteRequest {
                path: path.clone(),
                dry_run: false,
                permanent: false,
                force: false,
            };
            let mut fetcher = RecordedFetch(fetch);
            let event = match preflight(
                &path,
                &candidates,
                &process_cwd,
                &process_cwd,
                &|n| env::var_os(n),
                &request,
                permanent_delete,
                panes,
                true,
                &mut fetcher,
            ) {
                Ok(pf) => inspect_event_from_preflight(generation, pf),
                Err(e) => InspectEvent {
                    generation,
                    path,
                    findings: vec![e.to_string()],
                    blocked: true,
                    class: "unknown".into(),
                    strategy: "trash".into(),
                    confirm: DeleteConfirm::Trash,
                    fetch_required: false,
                },
            };
            let _ = tx.send(event);
        });
    }

    fn try_recv(&mut self) -> Option<InspectEvent> {
        self.rx.try_recv().ok()
    }
}

pub(crate) struct ThreadedMutate {
    tx: Sender<MutateEvent>,
    rx: Receiver<MutateEvent>,
    candidates: Vec<SessionCandidate>,
}

impl ThreadedMutate {
    pub(crate) fn new(candidates: Vec<SessionCandidate>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, candidates }
    }
}

impl MutateWorker for ThreadedMutate {
    fn begin(&mut self, path: String, kind: MutateKind, generation: u64) {
        let tx = self.tx.clone();
        let candidates = self.candidates.clone();
        thread::spawn(move || {
            let class = match classify(&path) {
                Ok(c) => c,
                Err(_) => {
                    let _ = tx.send(MutateEvent {
                        generation,
                        path,
                        result: Err(MutateFail::IdentityChanged),
                    });
                    return;
                }
            };
            let strategy = match kind {
                MutateKind::Trash => DeleteStrategy::Trash,
                MutateKind::Permanent => DeleteStrategy::Permanent,
            };
            let target = DeleteTarget {
                path: path.clone(),
                class,
                strategy,
            };
            if revalidate(&target, &candidates).is_err() {
                let _ = tx.send(MutateEvent {
                    generation,
                    path,
                    result: Err(MutateFail::IdentityChanged),
                });
                return;
            }
            let result = match kind {
                MutateKind::Trash => match trash::delete(&path) {
                    Ok(()) => Ok(DeleteStrategy::Trash),
                    Err(e) => Err(MutateFail::Trash(e.to_string())),
                },
                MutateKind::Permanent => match permanent_delete(&path, class) {
                    Ok(()) => Ok(DeleteStrategy::Permanent),
                    Err(e) => Err(MutateFail::Permanent(e.to_string())),
                },
            };
            let _ = tx.send(MutateEvent {
                generation,
                path,
                result,
            });
        });
    }

    fn try_recv(&mut self) -> Option<MutateEvent> {
        self.rx.try_recv().ok()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RefreshKind {
    Clone {
        dest: String,
        config_error: Option<String>,
    },
    Create {
        dest: String,
        config_error: Option<String>,
    },
    Delete {
        path: String,
    },
}

pub(crate) struct RefreshEvent {
    pub generation: u64,
    pub kind: RefreshKind,
    pub result: Result<Vec<SessionCandidate>, String>,
}

pub(crate) fn spawn_refresh(
    tx: Sender<RefreshEvent>,
    generation: u64,
    kind: RefreshKind,
    config_path: String,
) {
    thread::spawn(move || {
        let result =
            crate::config::reread_candidates(&config_path, &|n| env::var_os(n))
                .map_err(|e| e.to_string());
        let _ = tx.send(RefreshEvent {
            generation,
            kind,
            result,
        });
    });
}
