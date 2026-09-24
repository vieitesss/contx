# Worktree branch and PR lookup latency

## Summary

The in-progress collector already uses the right broad shape for a UI: local Git work runs in the background, GitHub lookup has its own worker pool, and local state is published before the PR badge. On this machine, a read-only end-to-end reproduction took **897 ms median** (five runs; 816–1,321 ms); `gh pr list` alone took **568 ms median** (six runs; 550–740 ms). This shows <1 s is plausible for one authenticated checkout on a warm network, not a general guarantee. The implementation's 3 s PR timeout allows >2 s result latency; GitHub/network latency is outside local control.

Recommendation: keep the single `gh pr list` request and async/stale-cache design. First profile cold launch and many-worktree cases. If needed, make branch identity independent of the expensive worktree status/numstat scan, and consider a persisted stale-while-revalidate PR cache only if cold-start latency warrants its freshness/complexity tradeoff. Define the SLA as p95 under healthy network, not an absolute bound; always publish local branch information promptly and represent PR timeout as unknown/transient, never as “no PR.”

## Collection path

At `src/tui/mod.rs::Tui::build`, `SessionCandidate::paths` feeds `git::start_poll`. The UI polls snapshots every 100 ms (`Tui::run`/`apply_git_snapshots`). `src/tui/git.rs` resolves each candidate by walking upward for `.git`, then runs `rev-parse --show-toplevel` and `--is-bare-repository`; unique roots are scanned by a four-worker local pool. An ordinary scan invokes status, HEAD verification, `diff --numstat`, branch identity, and upstream-count Git subprocesses. The current poll cadence is one second; roots and states persist across cycles.

Important workspace caveat: this checkout was already dirty before research. `HEAD` is `cc47f86`; the uncommitted `src/tui/git.rs` changes add PR lookup, caching, timeout, and PR worker threads (plus status/UI/test edits). The committed `HEAD` version does not include this PR lookup. Findings below describe the observed dirty working tree, not released `main`; coordinate before changing these files.

In the dirty tree, `Outcome::Scanned` queues the branch's PR lookup only after the entire `scan_worktree` finishes, then publishes the local result. Four separate PR workers run `gh`; `pull_request_cached_with` memoizes found/missing results for 60 s and transient errors for 5 s, keyed by worktree root + branch. The `gh` process has a 3 s timeout. A later PR outcome publishes a second cumulative snapshot. Thus rendering is not the wait; branch identity is gated behind local status/diff work, while PR is correctly off the UI/local-Git worker path.

## Benchmark

Safe read-only commands only; no files or Git refs changed. Environment: `gh 2.101.0`, authenticated session, one ordinary checkout (`main`, with no matching PR). A Python `perf_counter` sequence timed ordinary-root resolve (`rev-parse --show-toplevel`, `--is-bare-repository`), worktree scan (`status --porcelain=v1 -z --untracked-files=all`, `rev-parse --verify --quiet HEAD`, `diff --numstat -M HEAD`, `rev-parse --abbrev-ref HEAD`, `rev-list --left-right --count HEAD...@{upstream}`), local `remote get-url origin`, and `gh pr list --head main --state all --json number,state,headRepositoryOwner,headRefName`.

| Measurement | Samples | Median | Range |
|---|---:|---:|---:|
| Complete nine-process sequence | 5 | 897 ms | 816–1,321 ms |
| `gh pr list` only | 6 | 568 ms | 550–740 ms |
| Individual local Git processes | 6 each | about 26–34 ms | about 26–39 ms |

Limitations: one worktree, warm/authenticated GitHub CLI, no matching PR, small sample, one machine/network. Not a cold-start, many-candidate, PR-bearing, or p95 benchmark. It does establish that local process overhead is material but GitHub round-trip dominates this example.

## Recommendation and tradeoffs

1. **Keep `gh pr list --head <branch> --state all --json ...` as the default.** The official CLI supports these filters and fields. The installed CLI's first-party v2.101.0 implementation (`pkg/cmd/pr/shared/lister.go`) builds a GraphQL `pullRequests` query filtered by `headRefName` and states, requesting a page of up to the configured limit (30 by default). For the usual one matching PR this is one GraphQL request. Replacing it with hand-built `gh api`/HTTP is not shown to reduce the dominant network latency and adds auth/host/fork/error handling. Current local owner matching protects against same-name branches in different head repos.
2. **Make the branch/PR lane independent from detailed dirty-state scanning if profiles show delay.** After root resolution, obtain the branch identity immediately and enqueue PR lookup without waiting for status + numstat and untracked-line counting. A lightweight `rev-parse --abbrev-ref HEAD` is enough for the common named branch (keep detached-HEAD fallback). Alternatively, `git status --porcelain=v2 --branch --ahead-behind` exposes `branch.head`, `branch.upstream`, and `branch.ab` in one machine-readable stream, potentially replacing the separate branch/upstream subprocesses. Preserve the separate numstat pass if exact added/deleted line counts remain a product requirement; status can still be expensive on a huge/untracked-heavy tree.
3. **Cache policy follows the SLA.** Existing in-process 60 s success/miss and 5 s transient TTLs cut repeated API calls while the process lives; they do not help a new process. A persistent last-known `(remote identity, head owner, branch) -> PR` cache could make cold display subsecond, but is stale by design and needs invalidation/versioning. Use stale-while-revalidate and visibly distinguish stale/unknown if adopted.
4. **Do not promise a hard two-second remote-data guarantee.** The current 3 s `gh` timeout alone exceeds it; reducing timeout can bound waiting only if timed-out results stay “unknown” and old cached values remain visible. Network/API tails, auth refresh, rate limits, and slow local filesystems cannot be guaranteed under 1–2 s. Keep branch information available independently.
5. **Avoid premature bulk-fetching all PRs.** Per-branch CLI lookups parallelize and are precise. If real configs contain many branches from the same repo, profile first; a repo-wide batch can be explored but requires pagination/closed/merged completeness and fork-owner matching.

## Validation plan

- Define the endpoint precisely: elapsed time from candidate path input to a `Head::Named` plus checked PR result (`pull_request_checked`), excluding paint; separately measure time-to-local-branch.
- Benchmark cold process vs warm process/cache for 1, 5, and 20 candidates; ordinary + linked worktrees; PR open/closed/merged/missing; fork heads; huge/untracked-heavy repositories; missing `gh`, auth/rate-limit errors, and slow API. Report p50/p95/max and #processes/API calls; do not infer an SLA from one repo.
- Use a fake `gh` on `PATH` to deterministically test fast result, delayed result, timeout, transient failure, and miss; verify timeout is not cached/displayed as a miss and worker delay never blocks local Git snapshots. Add real read-only benchmark opt-in for network timings.
- Compare before/after on representative configs. Suggested acceptance: local branch p95 <250 ms on local SSD; PR p95 <2 s under healthy network with stale cache where available; never block UI; transient remote failure leaves local branch intact.

## Follow-up: 70-root cold launch (2026-09-24)

The single-checkout result above does **not** extrapolate to 70 worktrees. The branch-first partial publish still spent two Git process launches on root/bare resolution and a third on HEAD identity for each ordinary named root. One combined `git rev-parse --show-toplevel --is-bare-repository --abbrev-ref HEAD` now returns all three in **one** read-only process. Detached HEAD uses one extra short-SHA query; linked worktrees still resolve their main worktree separately. A Git failure after printing a valid root/bare pair (unborn HEAD) keeps the root and reports absent identity, rather than marking the worktree failed. Tests cover these cases and newline-containing paths.

App-level PTY timing with `/tmp/contx-diag/timeloop.py` (fresh process, release binary, **no PATH shims**, 2 s budget). Input begins at 1.1 s after launch and sends `rd` (expand/walk) at 1 ms per key, so the 70 rows are actually visible before the deadline; the earlier 100 backspace burn-in plus 8 ms per key imposed an approximately 3 s interaction floor. For the real config, some names are emitted in fragments by ratatui's diff renderer: the harness's contiguous-stream `t_branch` erroneously says 28 s (forced repaint). The supplementary grid replay in `/tmp/contx-diag/paintgrid.py` reconstructs complete visible names from those fragments; PR badges were contiguous and need no correction.

| Cold config | Before combined query | After combined query | Interpretation |
|---|---:|---:|---|
| Synthetic 70 distinct worktrees/remotes, branch tail | 4.15 s | **1.98 s** | One quiet no-shim run crossed the 2 s budget; a later run under system load average 43 was **4.25 s**, so this is not a guaranteed SLA. Synthetic `gh` does not return valid PR badges. |
| Representative user config (~68–69 Git roots), grid-replayed branch tail | 5.42 s | **3.09 s** | Material improvement, but **still over 2 s** under observed load. |
| Representative config, 7 observed PR badges | 15.47–23.90 s (one absent in 30 s) | 13.02–18.53 s (all 7) | Remote latency varies between runs; do **not** attribute this apparent improvement to the local Git change. An interleaved run with the original 8 ms input went from 15.28 s to 17.28 s instead. |

`gh pr list` takes 1.1–1.45 s median in shimmed measurements, and only four dedicated PR workers run concurrently. The synthetic 70 roots have 70 distinct `origin` URLs; the real config has at least 59 distinct observed origin URLs among ~69 roots. Even perfect repo-scoped batching still needs at least ~59 separate remote requests in the latter case. At 4 workers and ~1.2 s/request, **59/4 × 1.2 ≈ 18 s** is a throughput floor, before startup, retries, or rate limiting. Making all 70 fresh PR results appear in <2 s would require roughly 40 concurrent network requests at that observed service time (or an unproven multi-repository API), exposing GitHub secondary limits and local process pressure. A persisted badge cache could render old information quickly, but **cannot satisfy freshness** and must not be counted as a fast remote result. No aggressive fan-out or stale-as-fresh caching was introduced.

## Sources

- Repo: `src/tui/mod.rs` (`Tui::build`, `apply_git_snapshots`, `run`); `src/config/mod.rs` (`SessionCandidate::paths`); `src/tui/git.rs` (`resolve_root`, `scan_worktree_with`, `poll_worker`, PR cache/worker/outcome). Working-tree status was dirty at research time.
- GitHub CLI manual, [`gh pr list`](https://cli.github.com/manual/gh_pr_list): documents `--head`, state filters (`open|closed|merged|all`), JSON output fields including `number`, `state`, `headRepositoryOwner`, `headRefName`.
- GitHub CLI first-party source at the measured CLI version, [`pkg/cmd/pr/shared/lister.go` (v2.101.0)](https://github.com/cli/cli/blob/v2.101.0/pkg/cmd/pr/shared/lister.go): query filters `headRefName`/states and paginates GraphQL results.
- GitHub REST API docs, [List pull requests](https://docs.github.com/en/rest/pulls/pulls#list-pull-requests): alternative endpoint supports `state=all`, head `owner:ref`, and `per_page` up to 100; this is a viable fallback, not evidence it is faster.
- Git documentation, [`git status`](https://git-scm.com/docs/git-status): porcelain v2 branch headers include `branch.head`, `branch.upstream`, and `branch.ab +ahead -behind` with `--branch`; porcelain formats are intended for scripts.
- Git documentation, [`git rev-parse`](https://git-scm.com/docs/git-rev-parse): machine-readable top-level/bare/branch identity options.
