# Context

`contx` is an early-stage Rust CLI for inspecting and managing project targets, coding agents, and Git worktrees.

## Terms

- **session**: The top-level tmux container. A session contains windows. Distinct from a Herdr session.
- **window**: A tmux container within a session. A window contains panes.
- **pane**: The smallest multiplexer container tracked by `contx`; Git worktrees and agents are located through panes.
- **workspace**: The top-level Herdr container that realizes a project target. A workspace contains tabs, which contain panes.
- **Herdr session**: A persistent Herdr server namespace. Not a project target and not a tmux session.
- **project target**: The multiplexer container activated for a selected session candidate: a tmux session or a Herdr workspace.
- **multiplexer**: tmux or Herdr as the runtime that owns a terminal context. Independent environments; `contx` does not control one multiplexer from the other.
- **inner multiplexer**: The multiplexer whose live pane or client owns `contx`'s controlling TTY when multiplexers are nested.
- **agent process**: A recognized agent program running within a pane, whether started manually or by `contx`.
- **agent conversation**: A provider-specific, resumable conversation that may be opened by another agent process. `contx` does not close an existing process when opening the conversation elsewhere.
- **branch extraction**: Relocating the current branch from its original worktree into a new worktree. The original worktree must be clean and switches to a chosen return branch.
- **return branch**: The branch left in the original worktree during branch extraction. `contx` suggests the previous named branch, then the repository default branch, while allowing the user to choose another eligible branch.
- **observed resource**: A project target, window, pane, Git repository or worktree, or agent process discovered by `contx`. Its disappearance is accepted as current state; `contx` does not recreate it automatically.
- **Git repository**: A directory that contains a `.git` directory or a `.git` file (linked worktree).
- **home repository discovery**: Inspecting only the immediate child directories of `$HOME` for Git repositories.
- **tmux command boundary**: The subprocess call that executes tmux and converts its output or failure into `TmuxError`.
- **session candidate**: Any directory made available through configured-path expansion or discovery. Independent of which multiplexer will activate it.
- **clone source**: A provider-independent Git locator or reference, including SSH. Not a session candidate, because it is not yet a local directory.
- **cloning**: Creating a local Git repository from a clone source. Does not activate, enter, or open a project target; the user separately chooses whether to enter it.
- **clone destination**: The local path created for a cloned repository. It must not already exist.
- **search result**: A session candidate matching the current query, together with match information.
- **checkout identity**: The Git worktree's currently checked-out named branch, or a short SHA when detached. Independent of Git change summary and of cached upstream divergence; omitted while loading, when Git inspection failed, and for non-repos; never invents a branch.
- **Git change summary**: The additions, deletions, or dirty marker comparing a Git worktree's working state with HEAD.
- **cached upstream divergence**: The ahead and behind counts of the current branch HEAD relative to its configured upstream, taken from locally cached refs. Nonzero counts are shown as ↓behind and ↑ahead; zero or unavailable counts are omitted, with no automatic fetch or pull.
- **group key**: The configured parent directory used to group session candidates (dir children group under the expanded parent; dir/* grandchildren under the intermediate parent; home-discovered repos under $HOME). Empty query keeps first-seen `group_order`. A non-empty query lays out remainder-hit children in fuzzy rank order and may repeat a group header for each consecutive run.
- **tree spine**: The muted comment-colored tree drawing (├ / └ / │) that indicates indentation hierarchy: config group headers, remainder-only children, and linked worktrees nested under their main checkout. The spine never selects and never enters fuzzy ranking.
- **primary checkout**: The main worktree of a linked worktree, identified by the enclosing .git marker's git-common-dir, used only for nesting the linked worktree under its main. Independent of checkout identity and cached upstream divergence.
- **deletion**: The umbrella operation on the exact selected session candidate or displayed linked worktree (the link object when that path is a symlink; the target is never followed). A linked worktree is deleted only as that worktree via non-force Git worktree deletion; a primary checkout cannot be deleted while linked worktrees exist.
_Avoid_: remove, forget
- **trashing**: The default recoverable deletion strategy for an ordinary directory or standalone repository: moving the exact path to operating-system trash. Failure has no automatic fallback to permanent deletion.
- **permanent deletion**: The optional irreversible filesystem deletion strategy for an ordinary directory or standalone repository. It requires its own explicit decision and is never an automatic fallback from failed trashing.
- **deletion preflight**: The check that classifies a proposed deletion into overridable warnings versus hard blockers before any strategy runs.
- **overridable warning**: A deletion-preflight finding the user may explicitly accept, such as a nonempty directory or local-only Git data. Acceptance never bypasses a hard blocker.
- **hard blocker**: A deletion-preflight finding that forbids the operation regardless of warning acceptance, such as a non-candidate, active project target, identity race, dependent worktrees, failed required remote verification, unavailable chosen operation, or Git refusal.
- **worktree remainder**: The visible path of a session candidate within its group, after the group prefix is collapsed. Matches still rank on the full path; only the remainder paints fuzzy highlights on child rows.
- **remainder hit**: A match range that sits in the worktree remainder rather than the collapsed group prefix.
- **prefix-only group**: A config group whose matching session candidates have no remainder hit. Not a search result and not a project target.
_Avoid_: folded path
- **folded group**: a config group shown as its header row only (`▸ name (N)`), hiding its children until expanded. The startup state for every group under an empty query, and the state of a prefix-only group under a non-empty query.
- **unfolded group**: a config group showing its header (`▾ name`) plus children with the tree spine. A non-empty query unfolds every group that has a remainder hit, lays those children out in fuzzy rank order (repeating the header for each consecutive run of the same group key), and hides groups with no matching session candidates; prefix-only groups stay folded until expanded. Clearing the query refolds all.
- **focused header**: a folded group's header as a keyboard stop. Up/Down can land on it; Enter/Right/Space expands it (never activates a project target); Left on a child collapses its parent. Headers never enter fuzzy ranking.
- **progressive action dialog**: The floating in-picker interaction surface for picker-initiated clone and deletion, with the picker remaining visible beneath it; `contx` never restores or hands off the real terminal during the action. All operation stages stay visible vertically—the current stage expanded, completed stages collapsed to reopenable summaries, future stages visible but inactive—and successful completion returns to the preserved picker.
