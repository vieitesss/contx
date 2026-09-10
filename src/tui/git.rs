use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
    process::Command,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

/// Change summary for one session candidate, computed off the UI
/// thread. `root` is the resolved worktree; `None` means the
/// candidate sits in no Git worktree, or resolution failed.
/// `upstream` is the cached divergence from the configured
/// `@{upstream}`; it never changes `state`. `head` is the
/// checked-out identity; it changes neither.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CandidateState {
    pub(crate) root: Option<String>,
    pub(crate) linked: bool,
    /// Main worktree path when `linked`: the primary checkout a
    /// linked worktree nests under in the grouped list. `None`
    /// for ordinary checkouts, nonrepos, failures, and whenever
    /// the read-only query failed; the candidate then stays a
    /// flat group child.
    pub(crate) primary: Option<String>,
    pub(crate) state: WorkState,
    pub(crate) head: Head,
    pub(crate) upstream: Upstream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkState {
    Clean,
    Measurable { added: u64, deleted: u64 },
    Marker,
    Failed,
}

/// Cached divergence of HEAD from the configured `@{upstream}`,
/// from local refs only: no fetch, no pull, no network.
/// `Absent` means no upstream is configured, HEAD is detached,
/// the tracking ref is missing, or the query failed; it renders
/// as nothing, never as equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Upstream {
    Absent,
    Counts { ahead: u64, behind: u64 },
}

/// Checked-out identity of one worktree: the current branch
/// name, or the short SHA when HEAD is detached. `Absent`
/// means the query failed or the candidate has no scan; it
/// renders as nothing, never as `HEAD` or an invented name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Head {
    Named(String),
    Detached { short: String },
    Absent,
}

/// One worktree scan: the local working state, the
/// checked-out identity, plus the cached upstream divergence.
/// All ride the same Scan job, so a root still scans exactly
/// once per cycle. `Head` owns a `String`, so this is `Clone`
/// but not `Copy`; the poller clones on fan-out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanResult {
    pub(crate) state: WorkState,
    pub(crate) head: Head,
    pub(crate) upstream: Upstream,
}

/// Resolution outcome for one candidate: no enclosing `.git`
/// marker anywhere upward (nonrepo), a resolved worktree, or an
/// enclosing marker whose git inspection failed. A linked root
/// also carries its primary worktree path when the read-only
/// query succeeded; `None` means no nest later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RootResolve {
    Nonrepo,
    Root(String, bool, Option<String>),
    Failed,
}

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const SCAN_EVERY: Duration = Duration::from_secs(1);
/// Cadence for re-attempting a lagging cumulative publish (see
/// `Sent::Lagging`). A retry cadence, not a timing assumption:
/// any value converges, since every attempt carries the latest
/// cumulative states.
const PUBLISH_RETRY: Duration = Duration::from_millis(20);

/// Narrow internal seam at git command execution. Production
/// runs the real executable; tests script launch, status, and
/// stdout. Not a Git provider: parse and classification stay
/// in this module. Distinct from `GitOps`, which substitutes
/// whole resolve/scan operations for the scheduler.
trait CommandRunner {
    fn run(&mut self, root: &str, args: &[&str]) -> io::Result<RawOutput>;
}

/// One git process invocation, before classification.
struct RawOutput {
    success: bool,
    stdout: Vec<u8>,
}

struct ProductionRunner;

impl CommandRunner for ProductionRunner {
    fn run(&mut self, root: &str, args: &[&str]) -> io::Result<RawOutput> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()?;
        Ok(RawOutput {
            success: output.status.success(),
            stdout: output.stdout,
        })
    }
}

/// One `git` invocation. Launch failure, nonzero status, and
/// invalid UTF-8 are `None`; the caller maps that to Failed.
/// Read-only inspection takes no index locks: concurrent workers
/// share worktrees, so an optional lock must never block a scan
/// or let one mutate state another is reading.
fn git(
    runner: &mut dyn CommandRunner,
    root: &str,
    args: &[&str],
) -> Option<String> {
    let out = runner.run(root, args).ok()?;
    if !out.success {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Nearest enclosing non-bare worktree root plus whether it is a
/// linked worktree (`.git` is a file). The marker search is a
/// filesystem walk, so a missing marker (nonrepo) never runs
/// git; any git failure under an existing marker is Failed.
/// Bare repositories stay in the nonrepo bucket.
pub(crate) fn resolve_root(candidate: &str) -> RootResolve {
    resolve_root_with(candidate, &mut ProductionRunner)
}

fn resolve_root_with(
    candidate: &str,
    runner: &mut dyn CommandRunner,
) -> RootResolve {
    let mut dir = Path::new(candidate);
    let marker = loop {
        if dir.join(".git").exists() {
            break Some(dir);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break None,
        }
    };
    let top = match marker {
        Some(d) => d,
        None => return RootResolve::Nonrepo,
    };
    let top = top.display().to_string();
    let root = match git(runner, &top, &["rev-parse", "--show-toplevel"]) {
        Some(t) => t.trim().to_string(),
        None => return RootResolve::Failed,
    };
    match git(runner, &top, &["rev-parse", "--is-bare-repository"]) {
        Some(b) if b.trim() == "true" => return RootResolve::Nonrepo,
        Some(_) => {}
        None => return RootResolve::Failed,
    }
    let git_path = Path::new(&root).join(".git");
    let linked = fs::symlink_metadata(&git_path)
        .map(|m| !m.file_type().is_dir())
        .unwrap_or(true);
    // Linked checkouts nest under their main worktree in the
    // grouped list. The extra query rides this same Resolve
    // job; anything but a linked root skips it, and any
    // failure is `None`, never a fetch or a Failed.
    let primary = if linked {
        primary_root(runner, &root)
    } else {
        None
    };
    RootResolve::Root(root, linked, primary)
}

/// Main worktree path for a linked checkout from
/// `rev-parse --git-common-dir`, which points at the main
/// worktree's `.git` directory. Relative output is resolved
/// against the linked root; stripping a trailing `/.git`
/// yields the main worktree. Read-only like every other query:
/// `None` on any failure, so the candidate stays flat.
fn primary_root(runner: &mut dyn CommandRunner, root: &str) -> Option<String> {
    let common = git(runner, root, &["rev-parse", "--git-common-dir"])?;
    let common = common.trim();
    if common.is_empty() {
        return None;
    }
    let abs = if Path::new(common).is_absolute() {
        common.to_string()
    } else {
        Path::new(root).join(common).display().to_string()
    };
    match abs.strip_suffix("/.git") {
        Some(main) if !main.is_empty() => Some(main.to_string()),
        _ => None,
    }
}

/// Line count for untracked text. `None` means binary: skip it
/// (the candidate still shows dirty via the status below).
fn count_lines(bytes: &[u8]) -> Option<u64> {
    if bytes.contains(&0) {
        return None;
    }
    let mut n = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        n += 1;
    }
    Some(n)
}

/// Working state vs HEAD: tracked staged+unstaged numstat plus
/// non-ignored untracked text lines as additions. Unborn (no
/// HEAD) compares against the empty tree. Dirty without
/// measurable lines (binary, pure rename, empty file,
/// submodule, conflict, other) is Marker.
fn scan_root_with(root: &str, runner: &mut dyn CommandRunner) -> WorkState {
    let status = match git(
        runner,
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    ) {
        Some(s) => s,
        None => return WorkState::Failed,
    };
    let mut dirty = false;
    let mut untracked = vec![];
    // NUL-delimited `XY path` entries: quoting never kicks in,
    // so quoted/tab/newline paths survive intact. Rename sources
    // arrive as bare trailing fields without the `XY ` prefix
    // and are skipped. Non-UTF8 output fails the String
    // conversion in `git`, surfacing as Failed. Non-empty junk
    // with no valid XY field is Failed, not Clean.
    let mut saw_field = false;
    let mut saw_valid = false;
    for field in status.split('\0') {
        if field.is_empty() {
            continue;
        }
        saw_field = true;
        let raw = field.as_bytes();
        if raw.len() < 4 || raw[2] != b' ' {
            continue;
        }
        saw_valid = true;
        dirty = true;
        if raw[0] == b'?' && raw[1] == b'?' {
            untracked.push(&field[3..]);
        }
    }
    if saw_field && !saw_valid {
        return WorkState::Failed;
    }
    let head = git(runner, root, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| EMPTY_TREE.to_string());
    let numstat = match git(runner, root, &["diff", "--numstat", "-M", &head]) {
        Some(n) => n,
        None => return WorkState::Failed,
    };
    let mut added = 0u64;
    let mut deleted = 0u64;
    for line in numstat.lines() {
        dirty = true;
        let mut cols = line.split('\t');
        if let (Some(a), Some(d)) = (cols.next(), cols.next())
            && let (Ok(x), Ok(y)) = (a.parse::<u64>(), d.parse::<u64>())
        {
            added += x;
            deleted += y;
        }
    }
    for path in untracked {
        let full = Path::new(root).join(path);
        let meta = match fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.file_type().is_file() {
            continue;
        }
        let bytes = match fs::read(&full) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Some(n) = count_lines(&bytes) {
            added += n;
        }
    }
    if !dirty {
        WorkState::Clean
    } else if added + deleted > 0 {
        WorkState::Measurable { added, deleted }
    } else {
        WorkState::Marker
    }
}

/// Parse `rev-list --left-right --count HEAD...@{upstream}`
/// output (`ahead` then `behind`, whitespace-separated).
/// Anything else — empty, one column, non-numeric, extra
/// columns — is `Absent`, never a false equal.
fn parse_upstream(out: &str) -> Upstream {
    let mut cols = out.split_whitespace();
    match (cols.next(), cols.next(), cols.next()) {
        (Some(a), Some(b), None) => match (a.parse(), b.parse()) {
            (Ok(ahead), Ok(behind)) => Upstream::Counts { ahead, behind },
            _ => Upstream::Absent,
        },
        _ => Upstream::Absent,
    }
}

/// Cached ahead/behind of HEAD vs the configured `@{upstream}`
/// from local refs only. No fetch, no pull: a stale tracking
/// ref just reports stale counts. No upstream, detached HEAD,
/// a missing tracking ref, or any failure is `Absent`; the
/// caller keeps the local `WorkState` either way.
#[cfg(test)]
pub(crate) fn scan_upstream(root: &str) -> Upstream {
    scan_upstream_with(root, &mut ProductionRunner)
}

fn scan_upstream_with(root: &str, runner: &mut dyn CommandRunner) -> Upstream {
    let out = match git(
        runner,
        root,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    ) {
        Some(o) => o,
        None => return Upstream::Absent,
    };
    parse_upstream(&out)
}

/// Checked-out identity of one worktree from local refs only.
/// A trimmed name other than `HEAD` is that exact branch;
/// `HEAD` means detached, so the short SHA follows instead.
/// Any failure is `Absent`; the caller keeps the local
/// `WorkState` either way.
#[cfg(test)]
pub(crate) fn scan_head(root: &str) -> Head {
    scan_head_with(root, &mut ProductionRunner)
}

fn scan_head_with(root: &str, runner: &mut dyn CommandRunner) -> Head {
    let name = match git(runner, root, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Some(n) => n.trim().to_string(),
        None => return Head::Absent,
    };
    if name.is_empty() {
        return Head::Absent;
    }
    if name != "HEAD" {
        return Head::Named(name);
    }
    match git(runner, root, &["rev-parse", "--short", "HEAD"]) {
        Some(s) if !s.trim().is_empty() => Head::Detached {
            short: s.trim().to_string(),
        },
        _ => Head::Absent,
    }
}

/// One Scan job: the local working state, then the
/// checked-out identity and the cached upstream divergence.
/// A failed local scan skips the extra commands (Failed
/// renders alone) and stays `Absent` on both.
pub(crate) fn scan_worktree(root: &str) -> ScanResult {
    scan_worktree_with(root, &mut ProductionRunner)
}

fn scan_worktree_with(
    root: &str,
    runner: &mut dyn CommandRunner,
) -> ScanResult {
    let state = scan_root_with(root, runner);
    let (head, upstream) = match state {
        WorkState::Failed => (Head::Absent, Upstream::Absent),
        _ => (
            scan_head_with(root, runner),
            scan_upstream_with(root, runner),
        ),
    };
    ScanResult {
        state,
        head,
        upstream,
    }
}

/// Map a resolved candidate plus optional scan onto UI state.
/// Failed resolution is never treated as nonrepo. A missing scan
/// on a resolved root is Failed. Nonrepo is Clean with no root
/// (the row stays blank). Every scan-less path reports
/// `Absent`, never equal.
pub(crate) fn candidate_state(
    resolved: &RootResolve,
    scanned: Option<ScanResult>,
) -> CandidateState {
    match resolved {
        RootResolve::Root(root, linked, primary) => match scanned {
            Some(s) => CandidateState {
                root: Some(root.clone()),
                linked: *linked,
                primary: primary.clone(),
                state: s.state,
                head: s.head,
                upstream: s.upstream,
            },
            None => CandidateState {
                root: Some(root.clone()),
                linked: *linked,
                primary: primary.clone(),
                state: WorkState::Failed,
                head: Head::Absent,
                upstream: Upstream::Absent,
            },
        },
        RootResolve::Failed => CandidateState {
            root: None,
            linked: false,
            primary: None,
            state: WorkState::Failed,
            head: Head::Absent,
            upstream: Upstream::Absent,
        },
        RootResolve::Nonrepo => CandidateState {
            root: None,
            linked: false,
            primary: None,
            state: WorkState::Clean,
            head: Head::Absent,
            upstream: Upstream::Absent,
        },
    }
}

/// One side of a measurable pair at unit `level`: 0 exact, 1
/// `k`, 2 `m`, 3 `b`. Integer division only (`u64` `/`, no
/// floats, no overflow); `None` below the unit's threshold, so
/// `0k`/`0m`/`0b` never render.
pub(crate) fn fmt_count(n: u64, level: u8) -> Option<String> {
    match level {
        0 => Some(format!("{n}")),
        1 if n >= 1_000 => Some(format!("{}k", n / 1_000)),
        2 if n >= 1_000_000 => Some(format!("{}m", n / 1_000_000)),
        3 if n >= 1_000_000_000 => Some(format!("{}b", n / 1_000_000_000)),
        _ => None,
    }
}

/// Paired `(+A, −D)` texts whose `+A −D` char width fits
/// `budget`, searching unit levels independently from
/// least-coarse (minimizing total coarseness). Both counts or
/// neither: `None` means glyph only, never plus-only.
pub(crate) fn measurable_texts(
    added: u64,
    deleted: u64,
    budget: usize,
) -> Option<(String, String)> {
    for total in 0..=6u8 {
        for added_level in 0..=3u8 {
            let deleted_level = match total.checked_sub(added_level) {
                Some(l) if l <= 3 => l,
                _ => continue,
            };
            let (Some(a), Some(d)) = (
                fmt_count(added, added_level),
                fmt_count(deleted, deleted_level),
            ) else {
                continue;
            };
            if format!("+{a} −{d}").chars().count() <= budget {
                return Some((format!("+{a}"), format!("−{d}")));
            }
        }
    }
    None
}

/// Last-known observed Git state per session candidate, owned
/// by the Git observation module. The poller publishes
/// cumulative snapshots that omit still-pending candidates, so
/// callers merge here instead of relearning that rule: an
/// omitted candidate keeps its state, a repeated candidate is
/// overwritten, and an empty snapshot changes nothing. `None`
/// from `get` means never observed, which renders as loading.
#[derive(Clone, Debug, Default)]
pub(crate) struct GitStates {
    known: HashMap<String, CandidateState>,
}

impl GitStates {
    pub(crate) fn new() -> Self {
        GitStates {
            known: HashMap::new(),
        }
    }

    /// Merge one published snapshot into last-known state.
    pub(crate) fn apply(&mut self, snapshot: Vec<(String, CandidateState)>) {
        for (candidate, state) in snapshot {
            self.known.insert(candidate, state);
        }
    }

    /// Last-known state for one candidate, if observed yet.
    pub(crate) fn get(&self, candidate: &str) -> Option<&CandidateState> {
        self.known.get(candidate)
    }

    /// Drop states for paths that are no longer in the catalog.
    pub(crate) fn retain_paths(&mut self, keep: &HashSet<String>) {
        self.known.retain(|k, _| keep.contains(k));
    }
}

/// Injectable Git operations behind the poll worker. Production
/// runs real `git` subprocesses; tests substitute deterministic
/// fakes to drive scheduling without touching the filesystem.
/// Shared across worker threads, so every op is `Send + Sync`.
#[derive(Clone)]
pub(crate) struct GitOps {
    pub(crate) resolve: Arc<dyn Fn(&str) -> RootResolve + Send + Sync>,
    pub(crate) scan: Arc<dyn Fn(&str) -> ScanResult + Send + Sync>,
    pub(crate) between_cycles: Arc<dyn Fn() + Send + Sync>,
}

impl GitOps {
    /// Real subprocess-backed operations with the production
    /// one-second pause between refresh cycles.
    fn real() -> Self {
        GitOps {
            resolve: Arc::new(resolve_root),
            scan: Arc::new(scan_worktree),
            between_cycles: Arc::new(|| thread::sleep(SCAN_EVERY)),
        }
    }
}

/// Spawn the Git poll thread. Capacity 1: a slow UI cannot
/// queue extra full snapshots. Dropping the receiver makes the
/// next `send` fail and the thread return.
pub(crate) fn start_poll(
    candidates: Vec<String>,
) -> mpsc::Receiver<Vec<(String, CandidateState)>> {
    start_poll_with(candidates, GitOps::real(), 4)
}

/// Same poller with injected operations for deterministic tests.
/// `workers` sizes the bounded Git pool (at least one thread).
pub(crate) fn start_poll_with(
    candidates: Vec<String>,
    ops: GitOps,
    workers: usize,
) -> mpsc::Receiver<Vec<(String, CandidateState)>> {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || poll_worker(candidates, ops, workers, tx));
    rx
}

/// One unit of Git work for the pool: resolve a candidate to its
/// worktree, or scan a known worktree root.
enum Job {
    Resolve(String),
    Scan(String),
}

/// A finished unit of Git work, sent back to the coordinator.
enum Outcome {
    Resolved {
        candidate: String,
        resolved: RootResolve,
    },
    Scanned {
        root: String,
        scanned: ScanResult,
    },
}

/// Worker side of the pool queues. Scans always beat resolves:
/// a finished resolve must not wait behind dozens of queued
/// resolves before its worktree scans. Every enqueue pairs with
/// a wake ping sent after the job, so a parked worker either
/// sees the job on recheck or consumes the ping and rechecks —
/// wakeups are never missed, and stale pings just re-park.
struct WorkerQueue {
    scans: Mutex<mpsc::Receiver<String>>,
    resolves: Mutex<mpsc::Receiver<String>>,
    wake: Mutex<mpsc::Receiver<()>>,
}

impl WorkerQueue {
    /// Next job, scans first. `None` once every sender is dropped
    /// and both queues drain: only the coordinator shutdown does
    /// that, so queued-but-unstarted jobs are abandoned exactly
    /// then, while in-flight commands always finish first.
    fn next(&self) -> Option<Job> {
        loop {
            if let Ok(root) = self.scans.lock().unwrap().try_recv() {
                return Some(Job::Scan(root));
            }
            match self.resolves.lock().unwrap().try_recv() {
                Ok(candidate) => return Some(Job::Resolve(candidate)),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Err(mpsc::TryRecvError::Disconnected) =
                        self.scans.lock().unwrap().try_recv()
                    {
                        return None;
                    }
                }
            }
            if self.wake.lock().unwrap().recv().is_err() {
                return None;
            }
        }
    }
}

/// Persistent pool worker: run jobs until the queue closes, then
/// return. Never touches the UI; outcomes travel back to the
/// coordinator. A failed outcome send means the coordinator is
/// gone, so the worker returns too.
fn git_worker(
    ops: GitOps,
    queue: Arc<WorkerQueue>,
    out: mpsc::Sender<Outcome>,
) {
    while let Some(job) = queue.next() {
        let outcome = match job {
            Job::Resolve(candidate) => {
                let resolved = (ops.resolve)(&candidate);
                Outcome::Resolved {
                    candidate,
                    resolved,
                }
            }
            Job::Scan(root) => {
                let scanned = (ops.scan)(&root);
                Outcome::Scanned { root, scanned }
            }
        };
        if out.send(outcome).is_err() {
            return;
        }
    }
}

/// Enqueue one job plus its wake ping. Fails only when the pool
/// is gone, which the outcome loop surfaces as shutdown.
fn assign(
    tx: &mpsc::Sender<String>,
    wake: &mpsc::Sender<()>,
    pending: &mut usize,
    job: String,
) -> bool {
    if tx.send(job).is_err() {
        return false;
    }
    let _ = wake.send(());
    *pending += 1;
    true
}

/// Cumulative known states in candidate order. Still-resolving
/// or still-scanning candidates are omitted: the UI renders
/// absent keys as loading, and never clears them (node 5).
fn snapshot(
    candidates: &[String],
    published: &HashMap<String, CandidateState>,
) -> Vec<(String, CandidateState)> {
    candidates
        .iter()
        .filter_map(|c| published.get(c).map(|s| (c.clone(), s.clone())))
        .collect()
}

/// Outcome of one cumulative publish attempt.
enum Sent {
    /// The latest cumulative states reached the UI.
    Done,
    /// The UI lags (Full): the latest states are still only in
    /// `published` and must be re-attempted, or a gated cycle
    /// would strand them behind the stale buffered snapshot.
    Lagging,
    /// The receiver is gone: shut the pool down.
    Gone,
}

/// Best-effort cumulative snapshot per completion; skipped while
/// the UI lags (Full), reported when the receiver is gone. The
/// cycle-end blocking send stays the guaranteed snapshot, but
/// callers also converge a `Lagging` publish via re-attempts so
/// a quiet gated cycle cannot strand fresh states.
fn publish(
    tx: &mpsc::SyncSender<Vec<(String, CandidateState)>>,
    candidates: &[String],
    published: &HashMap<String, CandidateState>,
) -> Sent {
    match tx.try_send(snapshot(candidates, published)) {
        Ok(()) => Sent::Done,
        Err(mpsc::TrySendError::Full(_)) => Sent::Lagging,
        Err(mpsc::TrySendError::Disconnected(_)) => Sent::Gone,
    }
}

/// Unique worktree roots from the persisted resolutions.
fn unique_roots(mapping: &HashMap<String, RootResolve>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for resolved in mapping.values() {
        if let RootResolve::Root(r, ..) = resolved
            && seen.insert(r.clone())
        {
            roots.push(r.clone());
        }
    }
    roots
}

/// Stop the pool after in-flight commands: dropping every sender
/// releases parked workers, and joining waits out the running
/// ops. Panics when a worker died first, so a defective op
/// cannot hang the coordinator silently.
fn shutdown(
    handles: Vec<thread::JoinHandle<()>>,
    scan_tx: mpsc::Sender<String>,
    resolve_tx: mpsc::Sender<String>,
    wake_tx: mpsc::Sender<()>,
) {
    drop((scan_tx, resolve_tx, wake_tx));
    for handle in handles {
        if handle.join().is_err() {
            panic!("git worker thread panicked");
        }
    }
}

/// Background coordinator plus a bounded pool of Git workers.
/// Resolves every candidate once, then rescans each unique
/// worktree about once per second. Scans jump ahead of queued
/// resolves, so finished worktrees publish while slow resolves
/// are still gated: snapshots are cumulative, never atomic.
/// Resolutions and last-known states persist across refresh
/// cycles, which rescan unique roots only. Never touches the
/// UI; snapshots travel over the channel.
fn poll_worker(
    candidates: Vec<String>,
    ops: GitOps,
    workers: usize,
    tx: mpsc::SyncSender<Vec<(String, CandidateState)>>,
) {
    let (scan_tx, scan_rx) = mpsc::channel::<String>();
    let (resolve_tx, resolve_rx) = mpsc::channel::<String>();
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    let (out_tx, out_rx) = mpsc::channel::<Outcome>();
    let queue = Arc::new(WorkerQueue {
        scans: Mutex::new(scan_rx),
        resolves: Mutex::new(resolve_rx),
        wake: Mutex::new(wake_rx),
    });
    let mut handles = Vec::new();
    for _ in 0..workers.max(1) {
        let (ops, queue, out_tx) =
            (ops.clone(), Arc::clone(&queue), out_tx.clone());
        handles.push(thread::spawn(move || git_worker(ops, queue, out_tx)));
    }
    // The coordinator never sends outcomes: dropping its copy
    // lets a dead pool surface as a closed channel below.
    drop(out_tx);
    let mut mapping: HashMap<String, RootResolve> = HashMap::new();
    let mut published: HashMap<String, CandidateState> = HashMap::new();
    let mut states: HashMap<String, ScanResult> = HashMap::new();
    let mut scanning: HashSet<String> = HashSet::new();
    let mut scanned: HashSet<String> = HashSet::new();
    let mut pending = 0usize;
    // Set while the latest cumulative states still await
    // delivery after a Full skip; cleared on any delivery.
    let mut dirty = false;
    // Open the first cycle: resolve every candidate once.
    for c in &candidates {
        if !assign(&resolve_tx, &wake_tx, &mut pending, c.clone()) {
            shutdown(handles, scan_tx, resolve_tx, wake_tx);
            return;
        }
    }
    loop {
        while pending > 0 {
            // While lagging, wait briefly instead of blocking:
            // a quiet gated cycle must still re-attempt the
            // latest states, or they strand behind the stale
            // buffered snapshot. Flowing outcomes take
            // precedence over the retry whenever ready.
            let outcome = if dirty {
                match out_rx.recv_timeout(PUBLISH_RETRY) {
                    Ok(outcome) => outcome,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        match publish(&tx, &candidates, &published) {
                            Sent::Done => dirty = false,
                            Sent::Lagging => {}
                            Sent::Gone => {
                                shutdown(handles, scan_tx, resolve_tx, wake_tx);
                                return;
                            }
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        shutdown(handles, scan_tx, resolve_tx, wake_tx);
                        return;
                    }
                }
            } else {
                match out_rx.recv() {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        shutdown(handles, scan_tx, resolve_tx, wake_tx);
                        return;
                    }
                }
            };
            pending -= 1;
            match outcome {
                Outcome::Resolved {
                    candidate,
                    resolved,
                } => {
                    // Queue the root scan unless one is already
                    // queued, in flight, or done this cycle, so
                    // nested candidates share a single scan.
                    let mut scan_now = None;
                    if let RootResolve::Root(r, ..) = &resolved
                        && !scanned.contains(r)
                        && scanning.insert(r.clone())
                    {
                        scan_now = Some(r.clone());
                    }
                    // Publish whatever is knowable now: Nonrepo
                    // and Failed stand alone, while a root whose
                    // scan already finished (a nested candidate
                    // resolving late) publishes from that state.
                    // Roots with an outstanding scan publish on
                    // completion instead.
                    let publish_now = match &resolved {
                        RootResolve::Nonrepo | RootResolve::Failed => true,
                        RootResolve::Root(r, ..) => scanned.contains(r),
                    };
                    if publish_now {
                        let scanned_state = match &resolved {
                            RootResolve::Root(r, ..) => states.get(r).cloned(),
                            _ => None,
                        };
                        published.insert(
                            candidate.clone(),
                            candidate_state(&resolved, scanned_state),
                        );
                        match publish(&tx, &candidates, &published) {
                            Sent::Done => dirty = false,
                            Sent::Lagging => dirty = true,
                            Sent::Gone => {
                                shutdown(handles, scan_tx, resolve_tx, wake_tx);
                                return;
                            }
                        }
                    }
                    mapping.insert(candidate, resolved);
                    if let Some(r) = scan_now
                        && !assign(&scan_tx, &wake_tx, &mut pending, r)
                    {
                        shutdown(handles, scan_tx, resolve_tx, wake_tx);
                        return;
                    }
                }
                Outcome::Scanned {
                    root,
                    scanned: result,
                } => {
                    scanning.remove(&root);
                    scanned.insert(root.clone());
                    states.insert(root.clone(), result.clone());
                    // Fan out to every candidate mapped to this
                    // root, including nested late resolvers.
                    let mut members = Vec::new();
                    for (c, r) in mapping.iter() {
                        if let RootResolve::Root(rr, ..) = r
                            && *rr == root
                        {
                            members.push(c.clone());
                        }
                    }
                    for c in members {
                        if let Some(r) = mapping.get(&c) {
                            published.insert(
                                c,
                                candidate_state(r, Some(result.clone())),
                            );
                        }
                    }
                    match publish(&tx, &candidates, &published) {
                        Sent::Done => dirty = false,
                        Sent::Lagging => dirty = true,
                        Sent::Gone => {
                            shutdown(handles, scan_tx, resolve_tx, wake_tx);
                            return;
                        }
                    }
                }
            }
        }
        // Cycle complete: every resolve and scan finished. The
        // blocking send is the guaranteed snapshot per cycle,
        // which also delivers anything still lagging.
        if tx.send(snapshot(&candidates, &published)).is_err() {
            shutdown(handles, scan_tx, resolve_tx, wake_tx);
            return;
        }
        dirty = false;
        (ops.between_cycles)();
        // Refresh rescans unique roots only: resolutions and
        // last-known states persist, while per-cycle scan marks
        // reset so each root scans exactly once per cycle.
        scanning.clear();
        scanned.clear();
        states.clear();
        for r in unique_roots(&mapping) {
            if !assign(&scan_tx, &wake_tx, &mut pending, r) {
                shutdown(handles, scan_tx, resolve_tx, wake_tx);
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
