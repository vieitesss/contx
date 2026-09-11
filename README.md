# contx

`contx` is a terminal picker for project directories. It lists **session candidates** from your config, lets you fuzzy-search them with Git context, and activates the matching **project target**: a tmux **session** or a Herdr **workspace**. It can also clone a Git source into a new directory, create an empty directory, and delete an existing session candidate.

Candidate discovery, Git inspection, and the picker do not depend on which multiplexer you use. Activation talks to one backend only. `contx` does not list or manage Herdr panes, tabs, or agents, and it does not control one multiplexer from the other.

## Prerequisites

- A recent Rust toolchain (Cargo) to build
- tmux and/or Herdr, depending on where you run
- `contx` on your `PATH` if you bind a launch key

Activation needs a live multiplexer context (nonempty `TMUX` for tmux; `HERDR_ENV=1` and a nonempty `HERDR_SOCKET_PATH` for Herdr). The picker can still open without that; activation will then fail.

## Build

```sh
cargo build --release
```

The binary is `target/release/contx`. To install it onto Cargo's bin directory (`~/.cargo/bin` by default):

```sh
cargo install --path .
```

From a checkout, `just build` and `just run` wrap `cargo build`. `just run` launches the debug binary with this repo’s `config.toml`.

## Configuration

Default file: `~/.config/contx/config.toml`. A missing default file means an empty candidate list and `multiplexer = "auto"`. `-c` / `--config-file` must point at a file that exists.

```toml
paths = [
    "~/.config",
    "$HOME/opt/",
    "$HOME/work/*",
]
git-from-home = true
multiplexer = "auto"   # auto | tmux | herdr
permanent-delete = false
```

- `paths`: each entry expands `~` and environment variables and must become an absolute directory, or `dir/*`.
  - A directory contributes its immediate child directories, grouped under that parent.
  - `dir/*` contributes grandchildren, grouped under each child of `dir`.
- `git-from-home`: when `true`, also include Git repositories among the immediate children of `$HOME` (each child that contains a `.git` directory or file). Configured paths come first; a duplicate keeps the first spelling.
- `multiplexer`: omitted means `auto`.
- `permanent-delete`: omitted means `false`. When `true`, deleting a symlink, ordinary directory, or standalone repository uses irreversible filesystem deletion instead of trash. Linked worktrees always use Git worktree deletion. The picker has no `--permanent` flag; it follows this setting.

After a successful interactive clone whose destination is not already covered by `paths` or `git-from-home`, `contx` may offer to append the destination’s parent to `paths` in the active config. A destination already covered by a directory entry or a `dir/*` wildcard is not added again. Noninteractive clones never change the config.

The repo `config.toml` is a local example. It is not installed for you.

## Usage

```
contx [options]
contx [options] clone <source> [destination]
contx [options] delete [--dry-run] [--permanent] [--force] <path>

  -c, --config-file <path>         configuration file
  --multiplexer auto|tmux|herdr    multiplexer (default: auto)
  -h, --help                       show this help
```

CLI `--multiplexer` overrides the config file.

With no subcommand, `contx` opens the picker. `clone` and `delete` run without opening it.

`auto` uses the multiplexer that owns this terminal. Nested tmux-inside-Herdr prefers tmux; Herdr-inside-tmux prefers Herdr. If that cannot be decided, `contx` refuses and asks you to pass `--multiplexer tmux` or `--multiplexer herdr`. It never starts or attaches a multiplexer server, and it never falls back to the other backend.

Explicit `--multiplexer tmux` still requires nonempty `TMUX`. Explicit `--multiplexer herdr` still requires `HERDR_ENV=1` and nonempty `HERDR_SOCKET_PATH`, so a default Herdr server is never targeted.

Delete flags:

- `--dry-run`: print class, strategy, warnings, and blockers, then stop. No fetch, prompts, or filesystem/config changes. Remote verification is marked not performed.
- `--permanent`: irreversible deletion for a symlink, ordinary directory, or standalone repository. Linked worktrees still use Git worktree deletion. Allowed with `--dry-run`.
- `--force`: skip confirmations and accept overridable warnings. Does not bypass hard blockers, does not select permanent deletion, and is not a trash-failure fallback.

`--force` and `--dry-run` together are invalid (either order).

## Picker

Type to fuzzy-search. Groups start folded.

| Key | Action |
| --- | --- |
| Up / Down | Move (also Ctrl-K / Ctrl-J) |
| Home / End | First / last (also Ctrl-T / Ctrl-G or Ctrl-B) |
| Enter | Activate the selected directory, or expand a folded group header |
| Right or Space | Expand a folded group header |
| Left | Collapse the current group (from a child row) |
| Ctrl-X | Open the action menu (sequential, not a chord) |
| Ctrl-C | Quit without activating |

Action menu: `c` clones, `n` creates a directory, `d` deletes. Up / Down (also `k` / `j`) and Enter select an item. Escape or Ctrl-X again cancels without changing the query. Clone and new directory are always available. Delete is available only on a candidate row, not on a group header or an empty catalog.

Clone and directory creation open a floating **progressive action dialog** over the preserved picker. The current stage is expanded; completed stages collapse to inspectable summaries. There is no terminal handoff. Neither action activates a project target. CLI `clone` and `delete` remain ordinary non-TUI commands with inherited terminal behavior.

Clone source, destination, and the optional config-path choice are native fields. Directory creation only asks for a destination. Deletion preflight, findings, and confirmations are native controls. Interactive Git clone, fetch, and worktree deletion run in an embedded PTY/terminal area. Recognized Git/SSH prompts get native controls; unknown prompts remain usable through the terminal fallback.

While a Git child is running in the dialog, Ctrl-G requests cancellation; after a grace period an explicit Force Stop appears. Escape does not kill a running child. The picker’s Ctrl-G last-row motion applies only when the dialog is closed.

The clone source pre-fills with `https://github.com/`, ready for a repository path: typing `owner/repo` yields `https://github.com/owner/repo`. Edit or clear the field to clone from anywhere else.

Clone destination defaults relative to the focused group (the focused candidate’s group, or the focused header). Destination pre-fills with the repository name derived from the clone source the way `git clone` derives it (`<repo>` relative to the focused group, `~/<repo>` with no group), keeps tracking clone source edits until the field is edited, and is never overwritten after that. With no group, the destination must be absolute or `~`; relatives are never silently resolved against `$HOME` or the process working directory (the visible `~/` is editable text, not silent resolution). Directory creation uses the same destination rules but derives nothing from a source.

After a successful clone or directory creation, the catalog is rediscovered and the current query is kept. The new path is focused only if it is a discovered session candidate that matches that query; otherwise success is reported without inventing a row. After a successful deletion, the catalog is rediscovered, the query is kept, and focus moves to the nearest remaining visible row. An action that succeeds but whose refresh fails is still kept. Success shows a transient three-second message; cancellation is brief; errors and hard blockers remain until acknowledged.

Enter on a folded header never activates a project. Each candidate can show Git branch (or short SHA), dirty/add/delete counts, and cached upstream ahead/behind. Linked worktrees nest under their main checkout.

## Clone

```
contx clone <source> [destination]
```

`<source>` is anything `git clone` accepts, including SSH. `contx` runs `git clone <source> <destination>` with no extra flags (no branch, depth, or submodule options). When destination is omitted, `<destination>` is the repository name derived from the clone source (same rule `git clone` uses), resolved from the process working directory. An explicit destination behaves as before. Git uses the terminal for auth and output.

The destination must not already exist. Relatives on the CLI are resolved from the process working directory (`~` and environment variables are expanded). `contx` prints the absolute destination, then clones.

Failure or interruption leaves any surviving destination in place and does not change the config. If the clone succeeds but a requested config update fails, the destination is kept and the command still fails.

When stdin is a TTY and the destination is not already covered, `contx` asks whether to add the destination’s parent to `paths`. The question is asked before `git clone`; the file is written only after a successful clone. Declining still clones.

## New directory

Action menu `n` creates an empty directory instead of cloning. It has two stages, destination and result, and needs no Git child. The destination resolves exactly like a clone destination; nested paths are allowed, so `a/b` creates both directories. `contx` refuses a destination that already exists, creates the directory, and offers to add its parent to `paths` under the same covered rule as clone. Nothing is activated; the catalog is rediscovered and the current query is kept.

## Delete

```
contx delete [--dry-run] [--permanent] [--force] <path>
```

`<path>` must resolve to a **current session candidate** (canonical identity). Nested candidates are not widened to an enclosing Git root. A path that is not a candidate is refused.

Default strategy is the OS trash (`permanent-delete = false`). `--permanent` or `permanent-delete = true` selects irreversible filesystem deletion for a symlink, ordinary directory, or standalone repository. A symlink is deleted as the link object only; the target is never followed. A linked worktree is deleted only as that worktree, through non-force Git worktree deletion. A primary checkout that still has linked worktrees is blocked.

The picker has no `--dry-run`, `--permanent`, or `--force`. It uses the config strategy and interactive confirmations, including a separate permanent ask if trash fails.

Preflight splits **overridable warnings** (nonempty ordinary directory, local Git dirtiness or unique data, nested repositories, and similar) from **hard blockers** (not a candidate; disappeared or changed path; process or pane sitting in the target; failed pane list while the multiplexer is live; primary checkout with linked worktrees; failed or cancelled fetch; unavailable trash; Git refusing worktree deletion). Accepting warnings never bypasses a blocker. `--force` does not either.

Standalone repositories run `git fetch --all --prune` (inherited stdio) before deletion unless there are no remotes (warn and skip) or this is a dry-run.

Confirmations run after preflight unless `--force`: trash and linked-worktree deletion ask `y/N`; permanent deletion requires typing the exact path.

If trash fails, an interactive session asks separately for permanent deletion. A noninteractive session fails unless permanent deletion was already the selected strategy. `--force` alone does not fall back to permanent deletion.

Active-path checks use the process working directory and pane working directories from **this invocation’s selected multiplexer only**. Outside a multiplexer, panes are skipped and the process working directory is still checked.

## Activation

After Enter on a directory:

- **tmux:** switch to the session named from that path. If none exists, create one there and switch.
- **Herdr:** focus a workspace whose live pane working directory matches that path. If none exists, create one at the path (`--cwd`, `--label`, `--focus`). The label uses the same rules as tmux session names and is never used to find an existing workspace.

If a target was seen and is gone before the switch/focus, `contx` reports it and does not recreate it. Existing layout and processes are left as-is.

Herdr extras:

- Binary: nonempty `HERDR_BIN_PATH`, otherwise `herdr` on `PATH`. A set but unusable `HERDR_BIN_PATH` does not fall back to `PATH`.
- Several matching workspaces: a numbered prompt on a TTY stdin. Empty input, `q`, or EOF cancels with no focus and no create. Non-TTY stdin lists the IDs and exits.

## Launch keys

User config examples only — not installed automatically. Put `contx` on your `PATH` first. `contx` does not close the popup or pane.

tmux:

```
bind-key o display-popup -E contx
```

Herdr:

```toml
[[keys.command]]
key = "prefix+o"
type = "pane"
command = "contx"
```

## Limitations

- No Herdr pane, tab, or agent management in the TUI, and no control of one multiplexer from the other. Clone and delete act on session candidates (including Git linked worktrees), not on Herdr worktrees.
- Herdr matching uses live pane working directories, not labels or stored IDs. A shell that `cd`s away can make the original workspace unmatchable (a later pick may create a duplicate); a pane that `cd`s into another project can reuse that other workspace.
- Workspace IDs are not persisted. Disappeared targets are not recreated.
- No silent fallback between tmux and Herdr; no server start or attach. If `auto` picks the wrong nested multiplexer, pass `--multiplexer` explicitly.
