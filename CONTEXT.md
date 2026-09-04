# Context

`contx` is an early-stage Rust CLI for inspecting and managing tmux sessions, coding agents, and Git worktrees.

## Current scope

- Run tmux commands through `src/tmux`.
- Report tmux session context from the CLI.
- Prefer Rust's standard library until another dependency is justified.

## Terms

- **session**: The top-level tmux container shown by `contx`. A session contains windows.
- **window**: A tmux container within a session. A window contains panes.
- **pane**: The smallest tmux container tracked by `contx`; Git worktrees and agents are located through panes.
- **agent process**: A recognized agent program running within a pane, whether started manually or by `contx`.
- **agent conversation**: A provider-specific, resumable conversation that may be opened by another agent process. `contx` does not close an existing process when opening the conversation elsewhere.
- **branch extraction**: Relocating the current branch from its original worktree into a new worktree. The original worktree must be clean and switches to a chosen return branch.
- **return branch**: The branch left in the original worktree during branch extraction. `contx` suggests the previous named branch, then the repository default branch, while allowing the user to choose another eligible branch.
- **observed resource**: A session, window, pane, Git repository or worktree, or agent process discovered by `contx`. Its disappearance is accepted as current state; `contx` does not recreate it automatically.
- **Git repository**: A directory that contains a `.git` directory or a `.git` file (linked worktree).
- **home repository discovery**: Inspecting only the immediate child directories of `$HOME` for Git repositories.
- **tmux command boundary**: The subprocess call that executes tmux and converts its output or failure into `TmuxError`.
- **session candidate**: Any directory made available through configured-path expansion or discovery.
- **search result**: A session candidate matching the current query, together with match information.
