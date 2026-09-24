use super::{
    CandidateState, CommandRunner, GitOps, GitStates, Head, RawOutput,
    RootResolve, ScanResult, Upstream, WorkState, candidate_state, fmt_count,
    measurable_texts, resolve_root, resolve_root_with, scan_head,
    scan_upstream, scan_worktree, scan_worktree_with, start_poll_with,
};
use crate::utils::test_utils::TempDir;

#[test]
fn gh_pr_lookup_uses_supported_head_filter_and_exact_identity() {
    assert_eq!(
        super::pull_request_args("123"),
        [
            "pr",
            "list",
            "--head",
            "123",
            "--state",
            "all",
            "--json",
            "number,state,headRepositoryOwner,headRefName"
        ]
    );
    let json = r#"[{"number":1,"state":"OPEN","headRepositoryOwner":{"login":"other"},"headRefName":"topic"},{"number":42,"state":"MERGED","headRepositoryOwner":{"login":"example"},"headRefName":"topic"}]"#;
    assert_eq!(
        super::parse_pull_request(json, "example", "topic").unwrap(),
        Some(super::PullRequest {
            number: 42,
            state: super::PullRequestState::Merged
        })
    );
    assert_eq!(super::parse_pull_request(json, "example", "123"), Ok(None));
    for remote in [
        "git@github.com:example/project.git",
        "https://github.com/example/project.git",
        "ssh://git@github.com/example/project.git",
    ] {
        assert_eq!(super::github_remote_owner(remote), Some("example"));
    }
    assert_eq!(
        super::github_remote_owner("https://gitlab.com/example/project"),
        None
    );
    assert_eq!(
        super::github_remote_owner("https://github.com/example"),
        None
    );
}
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[test]
fn pr_parser_accepts_real_gh_open_json() {
    let json = r#"[{"number":12,"state":"OPEN","headRepositoryOwner":{"login":"alice"},"headRefName":"feat/demo"}]"#;
    assert_eq!(
        super::parse_pull_request(json, "alice", "feat/demo").unwrap(),
        Some(super::PullRequest {
            number: 12,
            state: super::PullRequestState::Open,
        })
    );
}

#[test]
fn pr_parser_accepts_real_gh_closed_json() {
    let json = r#"[{"number":13,"state":"CLOSED","headRepositoryOwner":{"login":"alice"},"headRefName":"topic"}]"#;
    assert_eq!(
        super::parse_pull_request(json, "alice", "topic").unwrap(),
        Some(super::PullRequest {
            number: 13,
            state: super::PullRequestState::Closed,
        })
    );
}

#[test]
fn pr_parser_accepts_real_gh_merged_json() {
    let json = r#"[{"number":14,"state":"MERGED","headRepositoryOwner":{"login":"alice"},"headRefName":"topic"}]"#;
    assert_eq!(
        super::parse_pull_request(json, "alice", "topic").unwrap(),
        Some(super::PullRequest {
            number: 14,
            state: super::PullRequestState::Merged,
        })
    );
}

#[test]
fn pr_parser_rejects_other_fork_owner_even_with_matching_branch() {
    let json = r#"[{"number":14,"state":"OPEN","headRepositoryOwner":{"login":"fork"},"headRefName":"topic"}]"#;
    assert_eq!(
        super::parse_pull_request(json, "upstream", "topic"),
        Ok(None)
    );
}

#[test]
fn pr_parser_matches_numeric_branch_as_text_not_pr_number() {
    let json = r#"[{"number":123,"state":"OPEN","headRepositoryOwner":{"login":"alice"},"headRefName":"topic"},{"number":47,"state":"MERGED","headRepositoryOwner":{"login":"alice"},"headRefName":"123"}]"#;
    assert_eq!(
        super::parse_pull_request(json, "alice", "123").unwrap(),
        Some(super::PullRequest {
            number: 47,
            state: super::PullRequestState::Merged,
        })
    );
}

#[test]
fn pr_parser_rejects_empty_missing_and_malformed_json() {
    assert_eq!(super::parse_pull_request("[]", "alice", "topic"), Ok(None));
    assert!(super::parse_pull_request("{", "alice", "topic").is_err());
    assert!(
        super::parse_pull_request(
            r#"[{"number":4,"state":"OPEN","headRefName":"topic"}]"#,
            "alice",
            "topic",
        )
        .is_err()
    );
}

#[test]
fn valid_nonmatching_pr_list_is_missing_not_transient() {
    let json = br#"[{"number":14,"state":"OPEN","headRepositoryOwner":{"login":"fork"},"headRefName":"topic"}]"#;
    assert!(matches!(
        super::classify_pull_request_output(json, "upstream", "topic"),
        super::PullRequestLookup::Missing
    ));
}

#[test]
fn malformed_nonempty_pr_list_is_transient_not_missing() {
    let json = br#"[{"number":14,"state":"OPEN","headRefName":"topic"}]"#;
    assert!(matches!(
        super::classify_pull_request_output(json, "upstream", "topic"),
        super::PullRequestLookup::Transient
    ));
}

#[test]
fn pr_errors_distinguish_missing_from_transient_failures() {
    assert!(matches!(
        super::classify_pull_request_error("no pull requests found"),
        super::PullRequestLookup::Missing
    ));
    assert!(matches!(
        super::classify_pull_request_error("authentication failed"),
        super::PullRequestLookup::Transient
    ));
}

#[test]
fn gh_nonzero_exit_is_a_transient_failure() {
    let tmp = TempDir::new();
    let result = super::run_gh_command(
        "sh",
        tmp.path().to_str().unwrap(),
        &["-c", "echo authentication failed >&2; exit 1"],
        Duration::from_secs(1),
    );
    assert!(matches!(
        super::classify_pull_request_error(&result.unwrap_err()),
        super::PullRequestLookup::Transient
    ));
}

#[test]
fn gh_timeout_kills_child_and_is_transient() {
    let tmp = TempDir::new();
    let result = super::run_gh_command(
        "sh",
        tmp.path().to_str().unwrap(),
        &["-c", "exec sleep 3"],
        Duration::from_millis(30),
    );
    assert!(result.is_err());
    assert!(matches!(
        super::classify_pull_request_error(&result.unwrap_err()),
        super::PullRequestLookup::Transient
    ));
}

#[test]
fn pr_cache_reuses_hit_and_miss_until_ttl_then_refreshes() {
    let cache = Mutex::new(HashMap::new());
    let now = Instant::now();
    let found = super::PullRequestLookup::Found(super::PullRequest {
        number: 9,
        state: super::PullRequestState::Open,
    });
    let calls = AtomicUsize::new(0);
    let lookup = || {
        calls.fetch_add(1, Ordering::SeqCst);
        found
    };
    assert!(matches!(
        super::pull_request_cached_with(&cache, "/repo", "topic", now, lookup),
        super::PullRequestLookup::Found(_)
    ));
    let cached = super::pull_request_cached_with(
        &cache,
        "/repo",
        "topic",
        now + Duration::from_secs(59),
        || panic!("cache hit should not spawn gh"),
    );
    assert!(matches!(cached, super::PullRequestLookup::Found(_)));
    assert!(matches!(
        super::pull_request_cached_with(
            &cache,
            "/repo",
            "topic",
            now + Duration::from_secs(60),
            || super::PullRequestLookup::Missing,
        ),
        super::PullRequestLookup::Missing
    ));
    assert!(matches!(
        super::pull_request_cached_with(
            &cache,
            "/repo",
            "topic",
            now + Duration::from_secs(61),
            || panic!("missing PR should remain cached for 60 seconds"),
        ),
        super::PullRequestLookup::Missing
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn pr_cache_is_keyed_by_root_and_branch_and_transient_retries_early() {
    let cache = Mutex::new(HashMap::new());
    let now = Instant::now();
    let transient = super::PullRequestLookup::Transient;
    super::pull_request_cached_with(&cache, "/fork", "123", now, || transient);
    assert!(matches!(
        super::pull_request_cached_with(
            &cache,
            "/fork",
            "123",
            now + Duration::from_secs(4),
            || panic!("transient should remain cached for five seconds")
        ),
        super::PullRequestLookup::Transient
    ));
    assert!(matches!(
        super::pull_request_cached_with(
            &cache,
            "/fork",
            "123",
            now + Duration::from_secs(5),
            || super::PullRequestLookup::Missing
        ),
        super::PullRequestLookup::Missing
    ));
    assert!(matches!(
        super::pull_request_cached_with(&cache, "/other", "123", now, || {
            transient
        }),
        super::PullRequestLookup::Transient
    ));
    assert!(matches!(
        super::pull_request_cached_with(&cache, "/fork", "topic", now, || {
            transient
        }),
        super::PullRequestLookup::Transient
    ));
}

#[test]
fn fmt_count_units_and_thresholds() {
    assert_eq!(fmt_count(999, 1), None);
    assert_eq!(fmt_count(1_000, 1), Some("1k".to_string()));
    assert_eq!(fmt_count(12_345_678, 1), Some("12345k".to_string()));
    assert_eq!(fmt_count(999_999, 2), None);
    assert_eq!(fmt_count(12_345_678, 2), Some("12m".to_string()));
    assert_eq!(fmt_count(999, 3), None);
    assert_eq!(fmt_count(1_500_000_000, 3), Some("1b".to_string()));
    assert_eq!(
        fmt_count(u64::MAX, 3),
        Some(format!("{}b", u64::MAX / 1_000_000_000)),
    );
}

#[test]
fn measurable_texts_both_counts_or_neither() {
    // Exact pair wins when it fits. Minus is U+2212.
    assert_eq!(
        measurable_texts(3, 12, 10),
        Some(("+3".to_string(), "−12".to_string())),
    );
    // Only the added side needs compacting.
    assert_eq!(
        measurable_texts(12_345_678, 9, 10),
        Some(("+12345k".to_string(), "−9".to_string())),
    );
    // Added stays finer while deleted escalates: least total
    // coarseness wins, never one side alone.
    assert_eq!(
        measurable_texts(1_500_000_000, 2_000_000_000, 10),
        Some(("+1500m".to_string(), "−2b".to_string())),
    );
    // Mixed small + extreme cannot pair: glyph only.
    assert_eq!(measurable_texts(3, u64::MAX, 10), None);
    assert_eq!(measurable_texts(u64::MAX, u64::MAX, 10), None);
    // Too-tight budget never yields plus-only.
    assert_eq!(measurable_texts(3, 12, 1), None);
}

#[test]
fn failed_resolve_is_failed_not_nonrepo() {
    let s = candidate_state(&RootResolve::Failed, None);
    assert_eq!(s.root, None);
    assert!(!s.linked);
    // Failed resolution needs no primary: nothing nests later.
    assert_eq!(s.primary, None);
    assert_eq!(s.state, WorkState::Failed);
    assert_eq!(s.head, Head::Absent);

    // A scanned state must not override Failed resolution.
    let s =
        candidate_state(&RootResolve::Failed, Some(scanned(WorkState::Clean)));
    assert_eq!(s.state, WorkState::Failed);
    assert_eq!(s.root, None);
    assert_eq!(s.head, Head::Absent);
    assert_eq!(s.upstream, Upstream::Absent);
}

#[test]
fn nonrepo_is_clean_without_root() {
    let s = candidate_state(&RootResolve::Nonrepo, None);
    assert_eq!(s.root, None);
    assert!(!s.linked);
    assert_eq!(s.state, WorkState::Clean);
    assert_eq!(s.head, Head::Absent);

    let s = candidate_state(
        &RootResolve::Nonrepo,
        Some(scanned(WorkState::Measurable {
            added: 1,
            deleted: 1,
        })),
    );
    assert_eq!(s.root, None);
    assert_eq!(s.state, WorkState::Clean);
    assert_eq!(s.head, Head::Absent);
    assert_eq!(s.upstream, Upstream::Absent);
}

#[test]
fn resolved_root_uses_scan_or_failed() {
    // A linked root carries its primary beside root and link
    // into UI state, so the grouped list can nest under it.
    let s = candidate_state(
        &RootResolve::Root("/r".to_string(), true, Some("/main".to_string())),
        Some(scanned(WorkState::Clean)),
    );
    assert_eq!(s.root.as_deref(), Some("/r"));
    assert!(s.linked);
    assert_eq!(s.primary.as_deref(), Some("/main"));
    assert_eq!(s.state, WorkState::Clean);
    assert_eq!(s.head, Head::Absent);
    assert_eq!(s.upstream, Upstream::Absent);

    // A scan carrying divergence fans out beside the local
    // state, never inside it.
    let diverged = ScanResult {
        state: WorkState::Clean,
        head: Head::Absent,
        upstream: Upstream::Counts {
            ahead: 2,
            behind: 1,
        },
    };
    let s = candidate_state(
        &RootResolve::Root("/r".to_string(), false, None),
        Some(diverged),
    );
    assert_eq!(s.state, WorkState::Clean);
    assert_eq!(
        s.upstream,
        Upstream::Counts {
            ahead: 2,
            behind: 1,
        }
    );

    let s = candidate_state(
        &RootResolve::Root("/r".to_string(), false, None),
        None,
    );
    assert_eq!(s.root.as_deref(), Some("/r"));
    assert!(!s.linked);
    assert_eq!(s.primary, None);
    assert_eq!(s.state, WorkState::Failed);
    assert_eq!(s.head, Head::Absent);
    assert_eq!(s.upstream, Upstream::Absent);
}

/// Poll `cond` until true or `timeout` elapses. The timeout is a
/// deadlock guard, never a timing assumption: every success path
/// below is rendezvoused through gates or call logs first, so a
/// hit always means a scheduling defect, never a slow machine.
fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !cond() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
    true
}

/// Manual gate: fake Git ops block in `wait` until the test
/// calls `open`. Deterministic rendezvous without sleeps.
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn closed() -> Self {
        Gate {
            open: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }

    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

/// Releases a gate when the test ends — including on assertion
/// failure — so a blocked fake op can never leak a worker
/// thread. Declare before the receiver: unwind then drops the
/// receiver first, and the released worker exits on its send.
struct OpenOnDrop(Arc<Gate>);

impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}

/// Injected ops with instant fakes and no pause between cycles.
/// Every closure must capture only `'static` (`Arc`) state: the
/// worker thread may still be exiting when the test returns.
/// A scan result with no upstream knowledge: what the
/// scheduler fakes produce. Production `scan_worktree` fills
/// real counts; the pool treats both identically.
fn scanned(state: WorkState) -> ScanResult {
    ScanResult {
        state,
        head: Head::Absent,
        upstream: Upstream::Absent,
    }
}

fn instant_ops(
    resolve: Arc<dyn Fn(&str) -> RootResolve + Send + Sync>,
    scan: Arc<dyn Fn(&str) -> WorkState + Send + Sync>,
) -> GitOps {
    GitOps {
        resolve: Arc::new(move |c| (resolve(c), Head::Absent)),
        scan: Arc::new(move |r| scanned(scan(r))),
        pr: Arc::new(|_, _| super::PullRequestLookup::Missing),
        between_cycles: Arc::new(|| {}),
        cycle_delay: Duration::ZERO,
    }
}

fn root(name: &str, linked: bool) -> RootResolve {
    RootResolve::Root(name.to_string(), linked, None)
}

/// Ops like `instant_ops`, but the worker parks in `gate` after
/// every cycle: the first snapshot is then a frozen point where
/// per-cycle call counts are exact. Drop the receiver, then open
/// the gate: the worker's next send fails and it exits cleanly.
fn gated_ops(
    resolve: Arc<dyn Fn(&str) -> RootResolve + Send + Sync>,
    scan: Arc<dyn Fn(&str) -> WorkState + Send + Sync>,
    gate: &Arc<Gate>,
) -> GitOps {
    GitOps {
        resolve: Arc::new(move |c| (resolve(c), Head::Absent)),
        scan: Arc::new(move |r| scanned(scan(r))),
        pr: Arc::new(|_, _| super::PullRequestLookup::Missing),
        between_cycles: Arc::new({
            let gate = Arc::clone(gate);
            move || gate.wait()
        }),
        cycle_delay: Duration::ZERO,
    }
}

/// Drain snapshots until one covers every candidate. The serial
/// worker yields it first try; the node-4 progressive worker
/// yields partials first. Bounded: a missing full snapshot is a
/// scheduling defect, not a slow machine.
fn recv_full(
    rx: &mpsc::Receiver<Vec<(String, CandidateState)>>,
    candidates: usize,
) -> Vec<(String, CandidateState)> {
    for _ in 0..32 {
        let snap = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("poll worker stopped without a full snapshot");
        if snap.len() == candidates {
            return snap;
        }
    }
    panic!("poll worker never published all {candidates} candidates");
}

#[test]
fn git_ops_never_exceed_four_in_flight() {
    // Shared by resolve and scan: the cap covers every Git op,
    // not each phase separately.
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let ops = instant_ops(
        Arc::new({
            let (current, peak) = (Arc::clone(&current), Arc::clone(&peak));
            // Odd candidates stay Nonrepo so resolve-only and
            // resolve-plus-scan paths interleave.
            move |c: &str| {
                let n = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                let out = match c.chars().last() {
                    Some('0' | '2' | '4' | '6') => root(c, false),
                    _ => RootResolve::Nonrepo,
                };
                current.fetch_sub(1, Ordering::SeqCst);
                out
            }
        }),
        Arc::new({
            let (current, peak) = (Arc::clone(&current), Arc::clone(&peak));
            move |_: &str| {
                let n = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                current.fetch_sub(1, Ordering::SeqCst);
                WorkState::Clean
            }
        }),
    );
    let candidates: Vec<String> = (0..8).map(|i| format!("/repo{i}")).collect();
    let rx = start_poll_with(candidates, ops, 4);
    let snap = recv_full(&rx, 8);
    assert_eq!(snap.len(), 8);
    drop(rx);
    assert!(
        peak.load(Ordering::SeqCst) <= 4,
        "worker pool exceeds four concurrent git ops",
    );
}

#[test]
fn candidates_sharing_a_root_scan_once() {
    let scans = Arc::new(AtomicUsize::new(0));
    let cycle = Arc::new(Gate::closed());
    let ops = gated_ops(
        Arc::new(|_: &str| root("/shared", false)),
        Arc::new({
            let scans = Arc::clone(&scans);
            move |r: &str| {
                assert_eq!(r, "/shared");
                scans.fetch_add(1, Ordering::SeqCst);
                WorkState::Clean
            }
        }),
        &cycle,
    );
    let rx = start_poll_with(
        vec!["/shared/a".to_string(), "/shared/b".to_string()],
        ops,
        4,
    );
    let snap = recv_full(&rx, 2);
    // Gate still closed: every cycle-1 op precedes the snapshot
    // send, so the count is frozen even if the worker has not
    // parked in the gate yet.
    assert_eq!(
        scans.load(Ordering::SeqCst),
        1,
        "nested candidates must share one scan",
    );
    drop(rx);
    cycle.open();
    for (_, state) in &snap {
        assert_eq!(state.root.as_deref(), Some("/shared"));
        assert_eq!(state.state, WorkState::Clean);
    }
}

#[test]
fn linked_worktrees_scan_as_distinct_roots() {
    let scanned = Arc::new(Mutex::new(Vec::new()));
    let cycle = Arc::new(Gate::closed());
    let ops = gated_ops(
        Arc::new(|c: &str| match c {
            "/plain" => root("/plain", false),
            "/linked" => root("/linked", true),
            other => panic!("unexpected candidate {other}"),
        }),
        Arc::new({
            let scanned = Arc::clone(&scanned);
            move |r: &str| {
                scanned.lock().unwrap().push(r.to_string());
                WorkState::Clean
            }
        }),
        &cycle,
    );
    let rx = start_poll_with(
        vec!["/plain".to_string(), "/linked".to_string()],
        ops,
        4,
    );
    let snap = recv_full(&rx, 2);
    // Gate still closed: the per-cycle scan log cannot grow past
    // the snapshot send, so this read is exact.
    let mut scanned = scanned.lock().unwrap().clone();
    drop(rx);
    cycle.open();
    scanned.sort();
    assert_eq!(
        scanned,
        vec!["/linked".to_string(), "/plain".to_string()],
        "linked worktrees must not share a scan",
    );
    assert_eq!(snap[0].0, "/plain");
    assert!(!snap[0].1.linked);
    assert_eq!(snap[1].0, "/linked");
    assert!(snap[1].1.linked);
}

#[test]
fn failed_nonrepo_and_scan_failure_stay_distinct() {
    let ops = instant_ops(
        Arc::new(|c: &str| match c {
            "/broken" => RootResolve::Failed,
            "/plain" => RootResolve::Nonrepo,
            "/repo" => root("/repo", false),
            other => panic!("unexpected candidate {other}"),
        }),
        Arc::new(|_: &str| WorkState::Failed),
    );
    let rx = start_poll_with(
        vec![
            "/broken".to_string(),
            "/plain".to_string(),
            "/repo".to_string(),
        ],
        ops,
        4,
    );
    let snap = recv_full(&rx, 3);
    drop(rx);
    // Failed resolution keeps no root; Nonrepo renders Clean and
    // blank; a failed scan on a resolved root keeps the root.
    assert_eq!(
        snap[0].1,
        CandidateState {
            root: None,
            linked: false,
            primary: None,
            state: WorkState::Failed,
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        }
    );
    assert_eq!(
        snap[1].1,
        CandidateState {
            root: None,
            linked: false,
            primary: None,
            state: WorkState::Clean,
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        }
    );
    assert_eq!(
        snap[2].1,
        CandidateState {
            root: Some("/repo".to_string()),
            linked: false,
            primary: None,
            state: WorkState::Failed,
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        }
    );
}

#[test]
fn full_snapshot_matches_serial_candidate_state() {
    let ops = instant_ops(
        Arc::new(|c: &str| match c {
            "/a" => root("/ra", false),
            "/b" => root("/rb", true),
            "/c" => RootResolve::Nonrepo,
            "/d" => RootResolve::Failed,
            other => panic!("unexpected candidate {other}"),
        }),
        Arc::new(|r: &str| match r {
            "/ra" => WorkState::Measurable {
                added: 3,
                deleted: 4,
            },
            "/rb" => WorkState::Marker,
            other => panic!("unexpected root {other}"),
        }),
    );
    let rx = start_poll_with(
        vec![
            "/a".to_string(),
            "/b".to_string(),
            "/c".to_string(),
            "/d".to_string(),
        ],
        ops,
        4,
    );
    let snap = recv_full(&rx, 4);
    drop(rx);
    let expected: Vec<(String, CandidateState)> = vec![
        (
            "/a".to_string(),
            candidate_state(
                &root("/ra", false),
                Some(scanned(WorkState::Measurable {
                    added: 3,
                    deleted: 4,
                })),
            ),
        ),
        (
            "/b".to_string(),
            candidate_state(
                &root("/rb", true),
                Some(scanned(WorkState::Marker)),
            ),
        ),
        (
            "/c".to_string(),
            candidate_state(&RootResolve::Nonrepo, None),
        ),
        (
            "/d".to_string(),
            candidate_state(&RootResolve::Failed, None),
        ),
    ];
    assert_eq!(snap, expected);
}

#[test]
fn refresh_cycle_preserves_all_states() {
    let ops = instant_ops(
        Arc::new(|c: &str| match c {
            "/repo" => root("/repo", false),
            "/plain" => RootResolve::Nonrepo,
            other => panic!("unexpected candidate {other}"),
        }),
        Arc::new(|_: &str| WorkState::Clean),
    );
    let rx = start_poll_with(
        vec!["/repo".to_string(), "/plain".to_string()],
        ops,
        4,
    );
    let first = recv_full(&rx, 2);
    let second = recv_full(&rx, 2);
    drop(rx);
    assert_eq!(first, second, "refresh must not omit last-known states",);
}

#[test]
fn refresh_rescans_each_unique_root_once() {
    let resolves = Arc::new(AtomicUsize::new(0));
    let scans = Arc::new(AtomicUsize::new(0));
    // Per-root scan counts surface as the state, so snapshots
    // pin which cycle they come from: a cumulative partial can
    // cover every candidate with stale states, which a bare
    // length check cannot tell from a cycle-end snapshot.
    let scan_calls = Arc::new(Mutex::new(HashMap::<String, u64>::new()));
    // Each cycle-end parks here until the test releases the
    // next cycle, so per-cycle call counts are exact points.
    // The mutex shares the receiver with the `Send + Sync`
    // closure; only the coordinator thread ever locks it.
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let ops = GitOps {
        resolve: Arc::new({
            let resolves = Arc::clone(&resolves);
            move |c: &str| {
                resolves.fetch_add(1, Ordering::SeqCst);
                let resolved = match c {
                    "/a/x" | "/a/y" => root("/ra", false),
                    "/b" => root("/rb", false),
                    other => panic!("unexpected candidate {other}"),
                };
                (resolved, Head::Absent)
            }
        }),
        scan: Arc::new({
            let (scans, scan_calls) =
                (Arc::clone(&scans), Arc::clone(&scan_calls));
            move |r: &str| {
                scans.fetch_add(1, Ordering::SeqCst);
                let mut calls = scan_calls.lock().unwrap();
                let n = calls.entry(r.to_string()).or_insert(0);
                *n += 1;
                ScanResult {
                    state: WorkState::Measurable {
                        added: *n,
                        deleted: 0,
                    },
                    head: Head::Absent,
                    upstream: Upstream::Absent,
                }
            }
        }),
        between_cycles: Arc::new({
            let release_rx = Arc::clone(&release_rx);
            move || {
                release_rx.lock().unwrap().recv().unwrap();
            }
        }),
        pr: Arc::new(|_, _| super::PullRequestLookup::Missing),
        cycle_delay: Duration::ZERO,
    };
    let rx = start_poll_with(
        vec!["/a/x".to_string(), "/a/y".to_string(), "/b".to_string()],
        ops,
        4,
    );
    // Cycle 1: three resolves, one scan per unique root. The
    // first full snapshot already proves both scans finished: a
    // root publishes only through its scan.
    let first = recv_full(&rx, 3);
    assert_eq!(resolves.load(Ordering::SeqCst), 3);
    assert_eq!(scans.load(Ordering::SeqCst), 2);
    for (_, state) in &first {
        assert_eq!(
            state.state,
            WorkState::Measurable {
                added: 1,
                deleted: 0,
            }
        );
    }
    // Cycle 2: no re-resolves, one rescan per unique root.
    // Drain until every state carries its second scan; stale
    // cumulative partials still show the first.
    release_tx.send(()).unwrap();
    let mut second = None;
    for _ in 0..32 {
        let snap = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("refresh cycle never rescanned every root");
        let rescanned = snap.len() == 3
            && snap.iter().all(|(_, s)| {
                s.state
                    == WorkState::Measurable {
                        added: 2,
                        deleted: 0,
                    }
            });
        if rescanned {
            second = Some(snap);
            break;
        }
    }
    let second = second.expect("refresh cycle never rescanned every root");
    // Both second scans completed, so the counters are final:
    // the coordinator parks in the unreleased gate next.
    assert_eq!(resolves.load(Ordering::SeqCst), 3);
    assert_eq!(scans.load(Ordering::SeqCst), 4);
    assert_eq!(second.len(), 3);
    drop(rx);
    // Release the parked worker so its next send fails and it
    // exits; further cycle-3 scans land after the asserts.
    release_tx.send(()).unwrap();
}

#[test]
fn dropped_receiver_exits_worker_promptly() {
    let ops = instant_ops(
        Arc::new(|_: &str| RootResolve::Nonrepo),
        Arc::new(|_: &str| WorkState::Clean),
    );
    let probe = Arc::clone(&ops.resolve);
    let rx = start_poll_with(vec!["/a".to_string()], ops, 4);
    recv_full(&rx, 1);
    drop(rx);
    // The worker owns the only other GitOps clone: its exit drops
    // the shared resolve closure, leaving just `probe` behind.
    assert!(
        wait_until(Duration::from_secs(10), || Arc::strong_count(&probe) == 1),
        "poll worker survived receiver drop",
    );
}

/// Node-4 behavior: finished candidates publish while a slow
/// resolve is still gated. Fails on the serial worker, which
/// holds every result behind the initial resolution barrier.
#[test]
fn fast_root_published_before_slow_root() {
    let gate = Arc::new(Gate::closed());
    let _guard = OpenOnDrop(Arc::clone(&gate));
    let resolved_log = Arc::new(Mutex::new(Vec::new()));
    let ops = instant_ops(
        Arc::new({
            let (gate, resolved_log) =
                (Arc::clone(&gate), Arc::clone(&resolved_log));
            move |c: &str| {
                resolved_log.lock().unwrap().push(c.to_string());
                match c {
                    "/slow" => {
                        gate.wait();
                        root("/rs", false)
                    }
                    "/fast" => root("/rf", false),
                    "/plain" => RootResolve::Nonrepo,
                    other => panic!("unexpected candidate {other}"),
                }
            }
        }),
        Arc::new(|_: &str| WorkState::Clean),
    );
    let rx = start_poll_with(
        vec![
            "/slow".to_string(),
            "/fast".to_string(),
            "/plain".to_string(),
        ],
        ops,
        4,
    );
    // Slow candidate first: the serial worker blocks inside its
    // resolve and never reaches the rest.
    assert!(
        wait_until(Duration::from_secs(3), || resolved_log
            .lock()
            .unwrap()
            .len()
            >= 2),
        "serial worker holds fast candidates behind the slow resolve",
    );
    // The fast root must surface while the slow one is gated.
    let mut partial = None;
    for _ in 0..8 {
        let snap = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("poll worker stopped while slow resolve gated");
        if snap.iter().any(|(c, _)| c == "/fast") {
            partial = Some(snap);
            break;
        }
    }
    let partial = partial.expect("fast root never published while slow gated");
    assert!(
        partial.iter().all(|(c, _)| c != "/slow"),
        "slow root published before its resolve finished",
    );
    gate.open();
    let full = recv_full(&rx, 3);
    drop(rx);
    assert_eq!(full[0].1.root.as_deref(), Some("/rs"));
    assert_eq!(full[1].1.root.as_deref(), Some("/rf"));
    assert_eq!(full[2].1.root, None);
}

/// Node-4 behavior: the bounded pool runs up to four scans at
/// once. Fails on the serial worker, which runs one op at a time.
#[test]
fn four_scans_can_overlap() {
    let gate = Arc::new(Gate::closed());
    let _guard = OpenOnDrop(Arc::clone(&gate));
    let entered = Arc::new(Mutex::new(Vec::new()));
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let ops = instant_ops(
        Arc::new(|c: &str| root(&format!("/root{c}"), false)),
        Arc::new({
            let (entered, current, peak, gate) = (
                Arc::clone(&entered),
                Arc::clone(&current),
                Arc::clone(&peak),
                Arc::clone(&gate),
            );
            move |r: &str| {
                entered.lock().unwrap().push(r.to_string());
                let n = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                gate.wait();
                current.fetch_sub(1, Ordering::SeqCst);
                WorkState::Clean
            }
        }),
    );
    let candidates: Vec<String> = (0..6).map(|i| i.to_string()).collect();
    let rx = start_poll_with(candidates, ops, 4);
    let overlapped = wait_until(Duration::from_secs(3), || {
        entered.lock().unwrap().len() >= 4
    });
    let started = entered.lock().unwrap().len();
    assert!(
        overlapped,
        "scan overlap stuck at {started}/4; serial worker runs one op at a time",
    );
    gate.open();
    let full = recv_full(&rx, 6);
    drop(rx);
    assert_eq!(
        peak.load(Ordering::SeqCst),
        4,
        "bounded pool runs four scans at once",
    );
    for (i, (name, state)) in full.iter().enumerate() {
        assert_eq!(name, &i.to_string());
        assert_eq!(state.root.as_deref(), Some(format!("/root{i}").as_str()));
        assert_eq!(state.state, WorkState::Clean);
    }
}

/// Run `git` with (`Some`) or without (`None`) `-C <dir>`,
/// succeeding or panicking with stderr. Throwaway repos only:
/// local paths, never clone/fetch/pull over the network, never
/// user repos. Global/system config stays out (a developer
/// hook, signer, or template must not leak in); identity
/// travels per-commit via `-c`.
fn git_cmd(dir: Option<&Path>, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git subprocess failed to spawn");
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("git output is not UTF-8")
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    git_cmd(Some(dir), args)
}

fn commit(dir: &Path, name: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(name), content).unwrap();
    git_in(dir, &["add", name]);
    git_in(
        dir,
        &[
            "-c",
            "user.email=contx-test@example.com",
            "-c",
            "user.name=contx-test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            msg,
        ],
    );
}

/// Throwaway origin + local clone. The branch is `topic`, not
/// `main`, so every assertion below proves the query follows
/// the configured `@{upstream}` instead of a hardcoded name.
/// Local paths only: the clone and every ref update stay on
/// disk, no network.
struct ClonePair {
    _tmp: TempDir,
    origin: PathBuf,
    work: PathBuf,
}

fn clone_pair() -> ClonePair {
    let tmp = TempDir::new();
    let origin = tmp.child("origin");
    git_in(&origin, &["init", "-b", "topic"]);
    commit(&origin, "file.txt", "one\n", "first");
    let work = tmp.path().join("work");
    git_cmd(
        None,
        &[
            "clone",
            origin.to_str().expect("temp path is UTF-8"),
            work.to_str().expect("temp path is UTF-8"),
        ],
    );
    ClonePair {
        _tmp: tmp,
        origin,
        work,
    }
}

/// Pretend a fetch ran: advance the cached tracking ref from
/// the local origin. No network — the remote is a temp-dir
/// path. Production never fetches; it only reads the cache
/// this simulates.
fn fetch_origin(work: &Path) {
    git_in(work, &["fetch", "origin"]);
}

fn work_str(pair: &ClonePair) -> String {
    pair.work.to_str().expect("temp path is UTF-8").to_string()
}

#[test]
fn upstream_behind_counts_cached_tracking_ref() {
    let pair = clone_pair();
    commit(&pair.origin, "file.txt", "one\ntwo\n", "second");
    fetch_origin(&pair.work);
    // Work stays at the first commit; only the cached tracking
    // ref moved: behind 1, ahead 0.
    let behind = Upstream::Counts {
        ahead: 0,
        behind: 1,
    };
    assert_eq!(scan_upstream(&work_str(&pair)), behind);
    let full = scan_worktree(&work_str(&pair));
    assert_eq!(full.state, WorkState::Clean);
    assert_eq!(full.upstream, behind);
}

#[test]
fn upstream_ahead_counts_local_only_commits() {
    let pair = clone_pair();
    commit(&pair.work, "file.txt", "one\nlocal\n", "local");
    assert_eq!(
        scan_upstream(&work_str(&pair)),
        Upstream::Counts {
            ahead: 1,
            behind: 0,
        }
    );
}

#[test]
fn upstream_diverged_counts_both_sides() {
    let pair = clone_pair();
    commit(&pair.origin, "file.txt", "one\ntwo\n", "second");
    fetch_origin(&pair.work);
    commit(&pair.work, "file.txt", "one\nlocal\n", "local");
    assert_eq!(
        scan_upstream(&work_str(&pair)),
        Upstream::Counts {
            ahead: 1,
            behind: 1,
        }
    );
}

#[test]
fn upstream_equal_is_counts_not_absent() {
    let pair = clone_pair();
    let upstream = scan_upstream(&work_str(&pair));
    assert_eq!(
        upstream,
        Upstream::Counts {
            ahead: 0,
            behind: 0,
        }
    );
    assert_ne!(upstream, Upstream::Absent);
}

#[test]
fn upstream_absent_without_configured_tracking() {
    let tmp = TempDir::new();
    let solo = tmp.child("solo");
    git_in(&solo, &["init", "-b", "topic"]);
    commit(&solo, "file.txt", "one\n", "first");
    let root = solo.to_str().expect("temp path is UTF-8");
    // No upstream, no failure: the local state stays Clean.
    assert_eq!(scan_upstream(root), Upstream::Absent);
    let full = scan_worktree(root);
    assert_eq!(full.state, WorkState::Clean);
    assert_eq!(full.upstream, Upstream::Absent);
}

#[test]
fn upstream_absent_on_detached_head() {
    let pair = clone_pair();
    git_in(&pair.work, &["checkout", "--detach", "HEAD"]);
    let root = work_str(&pair);
    assert_eq!(scan_upstream(&root), Upstream::Absent);
    assert_eq!(scan_worktree(&root).state, WorkState::Clean);
}

#[test]
fn upstream_absent_when_tracking_ref_missing() {
    let pair = clone_pair();
    // Upstream configured, but the cached ref was never
    // fetched: Absent, not a false equal and not a local
    // failure.
    git_in(
        &pair.work,
        &["config", "branch.topic.merge", "refs/heads/nope"],
    );
    let root = work_str(&pair);
    assert_eq!(scan_upstream(&root), Upstream::Absent);
    let full = scan_worktree(&root);
    assert_eq!(full.state, WorkState::Clean);
    assert_eq!(full.upstream, Upstream::Absent);
}

#[test]
fn dirty_state_survives_missing_upstream() {
    let tmp = TempDir::new();
    let solo = tmp.child("solo");
    git_in(&solo, &["init", "-b", "topic"]);
    commit(&solo, "file.txt", "one\n", "first");
    let root = solo.to_str().expect("temp path is UTF-8").to_string();
    // Text edit without any upstream: measurable lines stay,
    // divergence stays Absent.
    std::fs::write(solo.join("file.txt"), "one\ntwo\nthree\n").unwrap();
    let full = scan_worktree(&root);
    assert_eq!(
        full.state,
        WorkState::Measurable {
            added: 2,
            deleted: 0,
        }
    );
    assert_eq!(full.upstream, Upstream::Absent);
    // Staged pure rename: dirty but unmeasurable stays Marker.
    git_in(&solo, &["checkout", "--", "file.txt"]);
    git_in(&solo, &["mv", "file.txt", "renamed.txt"]);
    let full = scan_worktree(&root);
    assert_eq!(full.state, WorkState::Marker);
    assert_eq!(full.upstream, Upstream::Absent);
}

#[test]
fn head_names_current_branch_exactly() {
    let tmp = TempDir::new();
    let solo = tmp.child("solo");
    git_in(&solo, &["init", "-b", "feat/foo"]);
    commit(&solo, "file.txt", "one\n", "first");
    let root = solo.to_str().expect("temp path is UTF-8");
    // Unusual name, not `main`: the exact git name travels.
    assert_eq!(scan_head(root), Head::Named("feat/foo".to_string()));
    let full = scan_worktree(root);
    assert_eq!(full.state, WorkState::Clean);
    assert_eq!(full.head, Head::Named("feat/foo".to_string()));
}

#[test]
fn head_detached_reports_short_sha() {
    let pair = clone_pair();
    git_in(&pair.work, &["checkout", "--detach", "HEAD"]);
    let root = work_str(&pair);
    let short = git_in(&pair.work, &["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string();
    // Detached reports git's own abbrev, never the literal
    // `HEAD` and never an invented branch.
    assert!(!short.is_empty() && short != "HEAD");
    assert_eq!(
        scan_head(&root),
        Head::Detached {
            short: short.clone()
        }
    );
    let full = scan_worktree(&root);
    assert_eq!(full.state, WorkState::Clean);
    assert_eq!(full.head, Head::Detached { short });
}

#[test]
fn head_linked_worktree_reports_its_own_branch() {
    let tmp = TempDir::new();
    let solo = tmp.child("solo");
    git_in(&solo, &["init", "-b", "topic"]);
    commit(&solo, "file.txt", "one\n", "first");
    git_in(&solo, &["branch", "side"]);
    let linked = tmp.path().join("linked");
    let linked_str = linked.to_str().expect("temp path is UTF-8");
    git_in(&solo, &["worktree", "add", linked_str, "side"]);
    let solo_root = solo.to_str().expect("temp path is UTF-8");
    // Each checkout reports its own branch, not the other's.
    assert_eq!(scan_head(solo_root), Head::Named("topic".to_string()));
    assert_eq!(scan_head(linked_str), Head::Named("side".to_string()));
    let full = scan_worktree(linked_str);
    assert_eq!(full.head, Head::Named("side".to_string()));
}

#[test]
fn resolve_root_reports_primary_for_linked_worktree() {
    let tmp = TempDir::new();
    let solo = tmp.child("solo");
    git_in(&solo, &["init", "-b", "topic"]);
    commit(&solo, "file.txt", "one\n", "first");
    git_in(&solo, &["branch", "side"]);
    let linked = tmp.path().join("linked");
    let linked_str = linked.to_str().expect("temp path is UTF-8");
    git_in(&solo, &["worktree", "add", linked_str, "side"]);
    // Expected tops straight from git, so a symlinked temp
    // dir cannot skew the comparison.
    let main_top = git_in(&solo, &["rev-parse", "--show-toplevel"])
        .trim()
        .to_string();
    let linked_top = git_in(&linked, &["rev-parse", "--show-toplevel"])
        .trim()
        .to_string();
    match resolve_root(linked_str) {
        RootResolve::Root(root, linked, primary) => {
            assert!(linked);
            assert_eq!(root, linked_top);
            assert_eq!(primary.as_deref(), Some(main_top.as_str()));
        }
        other => panic!("expected linked root, got {other:?}"),
    }
    // UI state carries the primary beside root and link.
    let s = candidate_state(
        &resolve_root(linked_str),
        Some(scanned(WorkState::Clean)),
    );
    assert_eq!(s.root.as_deref(), Some(linked_top.as_str()));
    assert!(s.linked);
    assert_eq!(s.primary.as_deref(), Some(main_top.as_str()));
}

#[test]
fn resolve_root_leaves_primary_absent_for_ordinary_checkout() {
    let tmp = TempDir::new();
    let solo = tmp.child("solo");
    git_in(&solo, &["init", "-b", "topic"]);
    commit(&solo, "file.txt", "one\n", "first");
    let solo_str = solo.to_str().expect("temp path is UTF-8");
    match resolve_root(solo_str) {
        RootResolve::Root(_, linked, primary) => {
            assert!(!linked);
            assert_eq!(primary, None);
        }
        other => panic!("expected ordinary root, got {other:?}"),
    }
    let s = candidate_state(
        &resolve_root(solo_str),
        Some(scanned(WorkState::Clean)),
    );
    assert!(!s.linked);
    assert_eq!(s.primary, None);
}

#[test]
fn head_survives_dirty_worktree() {
    let tmp = TempDir::new();
    let solo = tmp.child("solo");
    git_in(&solo, &["init", "-b", "feat/foo"]);
    commit(&solo, "file.txt", "one\n", "first");
    let root = solo.to_str().expect("temp path is UTF-8").to_string();
    std::fs::write(solo.join("file.txt"), "one\ntwo\nthree\n").unwrap();
    // Dirty lines keep their state; identity is untouched.
    let full = scan_worktree(&root);
    assert_eq!(
        full.state,
        WorkState::Measurable {
            added: 2,
            deleted: 0,
        }
    );
    assert_eq!(full.head, Head::Named("feat/foo".to_string()));
}

#[test]
fn failed_scan_is_failed_with_absent_head_and_upstream() {
    // No repo here: the local scan fails, so the extras are
    // skipped and the state is never flipped by their absence.
    let tmp = TempDir::new();
    let root = tmp.path().to_str().expect("temp path is UTF-8");
    assert_eq!(scan_head(root), Head::Absent);
    let full = scan_worktree(root);
    assert_eq!(full.state, WorkState::Failed);
    assert_eq!(full.head, Head::Absent);
    assert_eq!(full.upstream, Upstream::Absent);
}

/// Last-known state for tests below: `root` travels with the
/// local `state`, and divergence travels beside it, never
/// inside it.
fn known_with(
    root: Option<&str>,
    state: WorkState,
    upstream: Upstream,
) -> CandidateState {
    CandidateState {
        root: root.map(str::to_string),
        linked: false,
        primary: None,
        state,
        head: Head::Absent,
        upstream,
        pull_request: None,
        pull_request_checked: false,
    }
}

fn known(root: Option<&str>, state: WorkState) -> CandidateState {
    known_with(root, state, Upstream::Absent)
}

#[test]
fn last_known_partial_omits_known_sibling() {
    let mut states = GitStates::new();
    states.apply(vec![(
        "/a".to_string(),
        known(Some("/a"), WorkState::Clean),
    )]);
    // Partial omits `/a`: still resolving, not gone.
    states.apply(vec![(
        "/b".to_string(),
        known(Some("/b"), WorkState::Marker),
    )]);
    assert_eq!(states.get("/a"), Some(&known(Some("/a"), WorkState::Clean)));
    assert_eq!(
        states.get("/b"),
        Some(&known(Some("/b"), WorkState::Marker))
    );
    // Never observed: renders as loading, never blank.
    assert_eq!(states.get("/c"), None);
}

#[test]
fn last_known_later_snapshot_overwrites_same_candidate() {
    let mut states = GitStates::new();
    states.apply(vec![(
        "/a".to_string(),
        known(Some("/a"), WorkState::Clean),
    )]);
    // A refresh rescan lands: merge updates, never freezes.
    states.apply(vec![(
        "/a".to_string(),
        known(
            Some("/a"),
            WorkState::Measurable {
                added: 2,
                deleted: 1,
            },
        ),
    )]);
    assert_eq!(
        states.get("/a"),
        Some(&known(
            Some("/a"),
            WorkState::Measurable {
                added: 2,
                deleted: 1,
            }
        ))
    );
}

#[test]
fn last_known_failed_survives_unrelated_partials() {
    let mut states = GitStates::new();
    states.apply(vec![
        ("/f".to_string(), known(None, WorkState::Failed)),
        ("/a".to_string(), known(Some("/a"), WorkState::Clean)),
    ]);
    // Later partials cover other candidates only: the failure
    // stays instead of flashing to loading.
    states.apply(vec![(
        "/b".to_string(),
        known(Some("/b"), WorkState::Clean),
    )]);
    assert_eq!(states.get("/f"), Some(&known(None, WorkState::Failed)));
    assert_eq!(states.get("/a"), Some(&known(Some("/a"), WorkState::Clean)));
    assert_eq!(states.get("/b"), Some(&known(Some("/b"), WorkState::Clean)));
}

#[test]
fn last_known_empty_snapshot_changes_nothing() {
    let mut states = GitStates::new();
    states.apply(vec![(
        "/a".to_string(),
        known(Some("/a"), WorkState::Clean),
    )]);
    states.apply(vec![]);
    assert_eq!(states.get("/a"), Some(&known(Some("/a"), WorkState::Clean)));
    assert_eq!(states.get("/b"), None);
}

#[test]
fn last_known_retains_upstream_counts() {
    let diverged = Upstream::Counts {
        ahead: 2,
        behind: 1,
    };
    let mut states = GitStates::new();
    states.apply(vec![
        (
            "/a".to_string(),
            known_with(Some("/a"), WorkState::Clean, diverged),
        ),
        ("/b".to_string(), known(Some("/b"), WorkState::Clean)),
    ]);
    // A partial that omits `/a` keeps its divergence beside the
    // local state; `/b` updates locally while staying `Absent`.
    states.apply(vec![(
        "/b".to_string(),
        known(Some("/b"), WorkState::Marker),
    )]);
    assert_eq!(
        states.get("/a"),
        Some(&known_with(Some("/a"), WorkState::Clean, diverged))
    );
    assert_eq!(
        states.get("/b"),
        Some(&known(Some("/b"), WorkState::Marker))
    );
    assert_eq!(states.get("/b").map(|s| s.upstream), Some(Upstream::Absent));
}

#[test]
fn last_known_drains_queued_snapshots_in_order() {
    let mut states = GitStates::new();
    states.apply(vec![(
        "/a".to_string(),
        known(Some("/a"), WorkState::Clean),
    )]);
    // Several queued publishes drain oldest-first through the
    // same merge rule: later states win, empties change
    // nothing, omissions retain.
    let (tx, rx) = mpsc::sync_channel::<Vec<(String, CandidateState)>>(3);
    tx.send(vec![(
        "/b".to_string(),
        known(Some("/b"), WorkState::Marker),
    )])
    .unwrap();
    tx.send(vec![(
        "/a".to_string(),
        known(
            Some("/a"),
            WorkState::Measurable {
                added: 2,
                deleted: 1,
            },
        ),
    )])
    .unwrap();
    tx.send(vec![]).unwrap();
    drop(tx);
    while let Ok(snapshot) = rx.try_recv() {
        states.apply(snapshot);
    }
    assert_eq!(
        states.get("/a"),
        Some(&known(
            Some("/a"),
            WorkState::Measurable {
                added: 2,
                deleted: 1,
            }
        ))
    );
    assert_eq!(
        states.get("/b"),
        Some(&known(Some("/b"), WorkState::Marker))
    );
}

#[test]
fn progressive_poll_applies_through_last_known() {
    let gate = Arc::new(Gate::closed());
    let _guard = OpenOnDrop(Arc::clone(&gate));
    let ops = instant_ops(
        Arc::new({
            let gate = Arc::clone(&gate);
            move |c: &str| match c {
                "/slow" => {
                    gate.wait();
                    root("/rs", false)
                }
                "/fast" => root("/rf", false),
                "/plain" => RootResolve::Nonrepo,
                other => panic!("unexpected candidate {other}"),
            }
        }),
        Arc::new(|_: &str| WorkState::Clean),
    );
    let rx = start_poll_with(
        vec![
            "/slow".to_string(),
            "/fast".to_string(),
            "/plain".to_string(),
        ],
        ops,
        4,
    );
    let mut states = GitStates::new();
    // Drain until the fast root is known while the slow one is
    // still gated: the slow key stays unknown (loading), never
    // blank or invented.
    let mut saw_fast = false;
    for _ in 0..32 {
        let snapshot = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("poll worker stopped while the slow resolve stayed gated");
        states.apply(snapshot);
        if states.get("/fast").is_some() {
            saw_fast = true;
            break;
        }
    }
    assert!(saw_fast, "fast root never published while slow gated");
    assert_eq!(states.get("/slow"), None);
    gate.open();
    let full = recv_full(&rx, 3);
    states.apply(full);
    drop(rx);
    assert_eq!(
        states.get("/fast").map(|s| s.root.clone()),
        Some(Some("/rf".to_string()))
    );
    assert_eq!(
        states.get("/slow").map(|s| s.root.clone()),
        Some(Some("/rs".to_string()))
    );
    assert_eq!(states.get("/plain"), Some(&known(None, WorkState::Clean)));
    assert_eq!(states.get("/missing"), None);
}

struct ScriptRunner {
    script: VecDeque<io::Result<RawOutput>>,
    calls: Vec<(String, Vec<String>)>,
}

impl ScriptRunner {
    fn new(script: Vec<io::Result<RawOutput>>) -> Self {
        Self {
            script: script.into(),
            calls: vec![],
        }
    }
}

impl CommandRunner for ScriptRunner {
    fn run(&mut self, root: &str, args: &[&str]) -> io::Result<RawOutput> {
        self.calls.push((
            root.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));
        self.script.pop_front().expect("scripted git exhausted")
    }
}

fn git_ok(stdout: &str) -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: true,
        stdout: stdout.as_bytes().to_vec(),
    })
}

fn git_fail() -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: false,
        stdout: vec![],
    })
}

fn git_nonzero(stdout: &str) -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: false,
        stdout: stdout.as_bytes().to_vec(),
    })
}

fn git_launch_err() -> io::Result<RawOutput> {
    Err(io::Error::new(ErrorKind::NotFound, "no git"))
}

fn git_utf8() -> io::Result<RawOutput> {
    Ok(RawOutput {
        success: true,
        stdout: vec![0xff, 0xfe],
    })
}

fn marked(tmp: &TempDir, name: &str, linked: bool) -> String {
    let dir = tmp.child(name);
    if linked {
        std::fs::write(dir.join(".git"), "gitdir: /tmp/main/.git\n").unwrap();
    } else {
        std::fs::create_dir(dir.join(".git")).unwrap();
    }
    dir.to_str().expect("temp path is UTF-8").to_string()
}

fn porcelain(entries: &[&str]) -> String {
    let mut s = String::new();
    for e in entries {
        s.push_str(e);
        s.push('\0');
    }
    s
}

#[test]
fn resolve_head_reads_root_bare_and_branch_with_one_git_process() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner =
        ScriptRunner::new(vec![git_ok(&format!("{root}\nfalse\ntopic\n"))]);
    assert_eq!(
        super::resolve_head_with(&root, &mut runner),
        (
            RootResolve::Root(root.clone(), false, None),
            Head::Named("topic".into())
        ),
    );
    assert_eq!(runner.calls.len(), 1);
    assert_eq!(
        runner.calls[0].1,
        [
            "rev-parse",
            "--show-toplevel",
            "--is-bare-repository",
            "--abbrev-ref",
            "HEAD"
        ],
    );
}

#[test]
fn resolve_head_detached_fetches_short_sha_only_after_combined_query() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![
        git_ok(&format!("{root}\nfalse\nHEAD\n")),
        git_ok("a1b2c3d\n"),
    ]);
    assert_eq!(
        super::resolve_head_with(&root, &mut runner),
        (
            RootResolve::Root(root, false, None),
            Head::Detached {
                short: "a1b2c3d".into()
            }
        ),
    );
    assert_eq!(runner.calls[1].1, ["rev-parse", "--short", "HEAD"]);
}

#[test]
fn resolve_head_retains_valid_root_when_unborn_head_exits_nonzero() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner =
        ScriptRunner::new(vec![git_nonzero(&format!("{root}\nfalse\nHEAD\n"))]);
    assert_eq!(
        super::resolve_head_with(&root, &mut runner),
        (RootResolve::Root(root, false, None), Head::Absent),
    );
    assert_eq!(runner.calls.len(), 1);
}

#[test]
fn resolve_head_handles_newline_in_root_path() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "line\nbreak", false);
    let mut runner =
        ScriptRunner::new(vec![git_ok(&format!("{root}\nfalse\ntopic\n"))]);
    assert_eq!(
        super::resolve_head_with(&root, &mut runner),
        (
            RootResolve::Root(root, false, None),
            Head::Named("topic".into())
        ),
    );
}

#[test]
fn resolve_head_keeps_unborn_real_worktree_instead_of_failed() {
    let tmp = TempDir::new();
    let root = tmp.child("unborn");
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let (resolved, head) = super::resolve_head(root.to_str().unwrap());
    assert!(matches!(resolved, RootResolve::Root(..)));
    assert_eq!(head, Head::Absent);
}

#[test]
fn scripted_nonrepo_runs_no_git() {
    let tmp = TempDir::new();
    let path = tmp.child("plain");
    let mut runner = ScriptRunner::new(vec![]);
    let resolved = resolve_root_with(path.to_str().unwrap(), &mut runner);
    assert_eq!(resolved, RootResolve::Nonrepo);
    assert!(runner.calls.is_empty());
}

#[test]
fn scripted_launch_failure_is_failed_inspection() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![git_launch_err()]);
    assert_eq!(resolve_root_with(&root, &mut runner), RootResolve::Failed);
    let mut runner = ScriptRunner::new(vec![git_launch_err()]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(scan.state, WorkState::Failed);
    assert_eq!(scan.head, Head::Absent);
    assert_eq!(scan.upstream, Upstream::Absent);
}

#[test]
fn scripted_nonzero_and_invalid_utf8_are_failed() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![git_fail()]);
    assert_eq!(resolve_root_with(&root, &mut runner), RootResolve::Failed);
    let mut runner = ScriptRunner::new(vec![git_utf8()]);
    assert_eq!(resolve_root_with(&root, &mut runner), RootResolve::Failed);
    let mut runner = ScriptRunner::new(vec![git_fail()]);
    assert_eq!(
        scan_worktree_with(&root, &mut runner).state,
        WorkState::Failed
    );
    let mut runner = ScriptRunner::new(vec![git_utf8()]);
    assert_eq!(
        scan_worktree_with(&root, &mut runner).state,
        WorkState::Failed
    );
}

#[test]
fn scripted_malformed_status_is_failed_not_clean() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![git_ok("not porcelain junk")]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(scan.state, WorkState::Failed);
    assert_eq!(scan.head, Head::Absent);
}

#[test]
fn scripted_malformed_numstat_does_not_invent_counts() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&[" M file.txt"])),
        git_ok("abc\n"),
        git_ok("not-tab-separated\n"),
        git_ok("topic\n"),
        git_fail(),
    ]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(scan.state, WorkState::Marker);
    assert_eq!(scan.head, Head::Named("topic".to_string()));
    assert_eq!(scan.upstream, Upstream::Absent);
}

#[test]
fn scripted_early_stop_after_status_is_failed() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&[" M file.txt"])),
        git_ok("abc\n"),
        git_fail(),
    ]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(scan.state, WorkState::Failed);
    assert_eq!(scan.head, Head::Absent);
    assert_eq!(scan.upstream, Upstream::Absent);
    assert_eq!(runner.calls.len(), 3);
}

#[test]
fn scripted_untracked_text_and_binary() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    std::fs::write(Path::new(&root).join("notes.txt"), "a\nb\n").unwrap();
    std::fs::write(Path::new(&root).join("blob.bin"), [b'a', 0, b'b']).unwrap();
    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&["?? notes.txt", "?? blob.bin"])),
        git_ok("abc\n"),
        git_ok(""),
        git_ok("topic\n"),
        git_fail(),
    ]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(
        scan.state,
        WorkState::Measurable {
            added: 2,
            deleted: 0,
        }
    );
    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&["?? blob.bin"])),
        git_ok("abc\n"),
        git_ok(""),
        git_ok("topic\n"),
        git_fail(),
    ]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(scan.state, WorkState::Marker);
}

#[test]
fn scripted_odd_nul_paths_rename_submodule_conflict() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let tab = Path::new(&root).join("a\tb.txt");
    std::fs::write(&tab, "x\n").unwrap();
    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&["?? a\tb.txt"])),
        git_ok("abc\n"),
        git_ok(""),
        git_ok("topic\n"),
        git_fail(),
    ]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(
        scan.state,
        WorkState::Measurable {
            added: 1,
            deleted: 0,
        }
    );

    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&["R  new.txt", "old.txt"])),
        git_ok("abc\n"),
        git_ok(""),
        git_ok("topic\n"),
        git_fail(),
    ]);
    assert_eq!(
        scan_worktree_with(&root, &mut runner).state,
        WorkState::Marker
    );

    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&[" M sub"])),
        git_ok("abc\n"),
        git_ok("-\t-\tsub\n"),
        git_ok("topic\n"),
        git_fail(),
    ]);
    assert_eq!(
        scan_worktree_with(&root, &mut runner).state,
        WorkState::Marker
    );

    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&["UU f.txt"])),
        git_ok("abc\n"),
        git_ok(""),
        git_ok("topic\n"),
        git_fail(),
    ]);
    assert_eq!(
        scan_worktree_with(&root, &mut runner).state,
        WorkState::Marker
    );
}

#[test]
fn scripted_unborn_head_uses_empty_tree() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    std::fs::write(Path::new(&root).join("a.txt"), "one\n").unwrap();
    let mut runner = ScriptRunner::new(vec![
        git_ok(&porcelain(&["?? a.txt"])),
        git_fail(),
        git_ok(""),
        git_ok("topic\n"),
        git_fail(),
    ]);
    let scan = scan_worktree_with(&root, &mut runner);
    assert_eq!(
        scan.state,
        WorkState::Measurable {
            added: 1,
            deleted: 0,
        }
    );
    assert_eq!(runner.calls[2].1[3], super::EMPTY_TREE);
}

#[test]
fn scripted_vanished_worktree_is_failed() {
    let tmp = TempDir::new();
    let root = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![git_fail()]);
    assert_eq!(resolve_root_with(&root, &mut runner), RootResolve::Failed);
    let mut runner = ScriptRunner::new(vec![git_fail()]);
    assert_eq!(
        scan_worktree_with(&root, &mut runner).state,
        WorkState::Failed
    );
}

#[test]
fn scripted_common_dir_relative_absolute_and_failed_linked() {
    let tmp = TempDir::new();
    let linked = marked(&tmp, "wt", true);
    let mut runner = ScriptRunner::new(vec![
        git_ok(&format!("{linked}\nfalse\nside\n")),
        git_ok("../main/.git\n"),
    ]);
    match resolve_root_with(&linked, &mut runner) {
        RootResolve::Root(root, is_linked, primary) => {
            assert!(is_linked);
            assert_eq!(root, linked);
            let expected =
                Path::new(&linked).join("../main").display().to_string();
            assert_eq!(primary.as_deref(), Some(expected.as_str()));
        }
        other => panic!("{other:?}"),
    }

    let mut runner = ScriptRunner::new(vec![
        git_ok(&format!("{linked}\nfalse\nside\n")),
        git_ok("/abs/main/.git\n"),
    ]);
    match resolve_root_with(&linked, &mut runner) {
        RootResolve::Root(_, true, primary) => {
            assert_eq!(primary.as_deref(), Some("/abs/main"));
        }
        other => panic!("{other:?}"),
    }

    let mut runner = ScriptRunner::new(vec![
        git_ok(&format!("{linked}\nfalse\nside\n")),
        git_fail(),
    ]);
    match resolve_root_with(&linked, &mut runner) {
        RootResolve::Root(_, true, primary) => assert_eq!(primary, None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn scripted_failed_bare_and_linked_resolve() {
    let tmp = TempDir::new();
    let ordinary = marked(&tmp, "repo", false);
    let mut runner = ScriptRunner::new(vec![git_ok(&format!(
        "{ordinary}\nmaybe\nbranch\n"
    ))]);
    assert_eq!(
        resolve_root_with(&ordinary, &mut runner),
        RootResolve::Failed
    );
    let mut runner =
        ScriptRunner::new(vec![git_ok(&format!("{ordinary}\ntrue\nbranch\n"))]);
    assert_eq!(
        resolve_root_with(&ordinary, &mut runner),
        RootResolve::Nonrepo
    );
    let linked = marked(&tmp, "wt", true);
    let mut runner = ScriptRunner::new(vec![git_fail()]);
    assert_eq!(resolve_root_with(&linked, &mut runner), RootResolve::Failed);
}

#[test]
fn last_known_partial_introduces_late_linked_nesting() {
    let mut states = GitStates::new();
    states.apply(vec![(
        "/main".to_string(),
        known(Some("/main"), WorkState::Clean),
    )]);
    states.apply(vec![(
        { "/linked".to_string() },
        CandidateState {
            root: Some("/linked".to_string()),
            linked: true,
            primary: Some("/main".to_string()),
            state: WorkState::Clean,
            head: Head::Named("side".to_string()),
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )]);
    assert_eq!(
        states.get("/main"),
        Some(&known(Some("/main"), WorkState::Clean))
    );
    let linked = states.get("/linked").expect("late linked published");
    assert!(linked.linked);
    assert_eq!(linked.primary.as_deref(), Some("/main"));
    assert_eq!(linked.head, Head::Named("side".to_string()));
}

#[test]
fn lagging_receiver_converges_on_newest_cumulative() {
    let ops = instant_ops(
        Arc::new(|c: &str| match c {
            "/a" => root("/ra", false),
            "/b" => root("/rb", false),
            other => panic!("{other}"),
        }),
        Arc::new(|r: &str| match r {
            "/ra" => WorkState::Clean,
            "/rb" => WorkState::Marker,
            other => panic!("{other}"),
        }),
    );
    let rx = start_poll_with(vec!["/a".to_string(), "/b".to_string()], ops, 4);
    // Leave the capacity-1 channel unread so later publishes lag,
    // then drain: last-known must be the newest full snapshot.
    std::thread::sleep(Duration::from_millis(50));
    let mut states = GitStates::new();
    let full = recv_full(&rx, 2);
    states.apply(full);
    while let Ok(snapshot) = rx.try_recv() {
        states.apply(snapshot);
    }
    drop(rx);
    assert_eq!(
        states.get("/a").map(|s| (s.root.clone(), s.state)),
        Some((Some("/ra".to_string()), WorkState::Clean))
    );
    assert_eq!(
        states.get("/b").map(|s| (s.root.clone(), s.state)),
        Some((Some("/rb".to_string()), WorkState::Marker))
    );
}

#[test]
fn git_states_retain_paths_drops_survivors_only() {
    let mut states = GitStates::new();
    states.apply(vec![(
        "/a".into(),
        CandidateState {
            root: Some("/a".into()),
            linked: false,
            primary: None,
            state: WorkState::Clean,
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )]);
    states.apply(vec![(
        "/b".into(),
        CandidateState {
            root: Some("/b".into()),
            linked: false,
            primary: None,
            state: WorkState::Clean,
            head: Head::Absent,
            upstream: Upstream::Absent,
            pull_request: None,
            pull_request_checked: false,
        },
    )]);
    let keep = HashSet::from(["/a".to_string()]);
    states.retain_paths(&keep);
    assert!(states.get("/a").is_some());
    assert!(states.get("/b").is_none());
}
            pull_request: None,
            pull_request_checked: false,
            pull_request: None,
            pull_request_checked: false,
            pull_request: None,
            pull_request_checked: false,
