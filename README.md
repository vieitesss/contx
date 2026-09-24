# contx

`contx` is a terminal picker and noninteractive CLI for project directories. It lists **session candidates** from your config, lets you fuzzy-search them with Git context, and activates the matching **project target**: a tmux **session** or a Herdr **workspace**. It can also clone a Git source, create a linked Git worktree, create an empty directory in the picker, and delete an existing session candidate. For agent workflows, see [SKILL.md](SKILL.md).

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

[clone]
default-protocol = "ssh"  # ssh | https
ssh-prefix = "git@github.com:"
https-prefix = "https://github.com"
```

- `paths`: each entry expands `~` and environment variables and must become an absolute directory, or `dir/*`.
  - A directory contributes its immediate child directories, grouped under that parent.
  - `dir/*` contributes grandchildren, grouped under each child of `dir`.
- `git-from-home`: when `true`, also include Git repositories among the immediate children of `$HOME` (each child that contains a `.git` directory or file). Configured paths come first; a duplicate keeps the first spelling.
- `multiplexer`: omitted means `auto`.
- `permanent-delete`: omitted means `false`. When `true`, deleting a symlink, ordinary directory, or standalone repository uses irreversible filesystem deletion instead of trash. Linked worktrees always use Git worktree deletion. The picker has no `--permanent` flag; it follows this setting.
- `[clone]`: picker-only presets. Omitted values default to SSH, `git@github.com:`, and `https://github.com`. Prefix edits in the dialog last only for that clone; edit TOML to change future defaults. CLI `clone <source>` is unchanged.

After a successful interactive clone whose destination is not already covered by `paths` or `git-from-home`, `contx` may offer to append the destination’s parent to `paths` in the active config. A destination already covered by a directory entry or a `dir/*` wildcard is not added again. Noninteractive clones never change the config.

The repo `config.toml` is a local example. It is not installed for you.

## Usage

```
contx [options]
contx [options] list
contx [options] open [--workspace-id <id>] <path>
contx [options] clone [--add-parent] <source> [destination]
contx [options] worktree create [--new-branch] [--add-parent] <repo> <branch> <destination>
contx [options] delete [--dry-run] [--permanent] [--force] <path>

  -c, --config-file <path>         configuration file
  --multiplexer auto|tmux|herdr    multiplexer (default: auto)
  --json                           structured output for non-picker commands
  -h, --help                       show this help
```

CLI `--multiplexer` overrides the config file.

With no subcommand, `contx` opens the picker. Other commands do not open it. `--json` requires a subcommand and reserves stdout for one JSON object (Git progress goes to stderr). Errors write `{"error":"..."}` as the last stderr line and exit nonzero; blocked deletes additionally include `preflight`. Git may have written progress to stderr first. JSON clone, open, and delete never prompt via contx, even when run from a TTY. Git/SSH credentials must be set up for unattended use.

`auto` uses the multiplexer that owns this terminal. Nested tmux-inside-Herdr prefers tmux; Herdr-inside-tmux prefers Herdr. If that cannot be decided, `contx` refuses and asks you to pass `--multiplexer tmux` or `--multiplexer herdr`. It never starts or attaches a multiplexer server, and it never falls back to the other backend.

Explicit `--multiplexer tmux` still requires nonempty `TMUX`. Explicit `--multiplexer herdr` still requires `HERDR_ENV=1` and nonempty `HERDR_SOCKET_PATH`, so a default Herdr server is never targeted.

Delete flags:

- `--dry-run`: print class, strategy, warnings, and blockers, then stop. No fetch, prompts, or filesystem/config changes. Remote verification is marked not performed.
- `--permanent`: irreversible deletion for a symlink, ordinary directory, or standalone repository. Linked worktrees still use Git worktree deletion. Allowed with `--dry-run`.
- `--force`: skip confirmations and accept overridable warnings. Does not bypass hard blockers, does not select permanent deletion, and is not a trash-failure fallback.

`--force` and `--dry-run` together are invalid (either order).

## Noninteractive project workflow

```sh
contx --json list
contx --json clone --add-parent <source> /absolute/path/to/new-repo
contx --json worktree create --new-branch --add-parent /absolute/path/to/repo feature/my-task /absolute/path/to/new-worktree
contx --json --multiplexer herdr open /absolute/path/to/new-worktree
```

`list` returns `{"candidates":[{"path":...,"group":...,"from_home_discovery":...}]}` from current configuration. `open` requires an existing current candidate (relative paths resolve against the process cwd); it returns a tmux `session` or Herdr `workspace_id` along with the activated `path` and `multiplexer`. It focuses an existing matching project target or creates one at that path. Herdr resolution prefers a workspace whose label matches the canonical target label, then a label ending with the target directory name; a label that merely contains the directory name is not a match. If neither exists, it creates a workspace with the canonical target label. If several workspaces match by name, the CLI **fails without prompting or creating** and returns `workspace_ids` in the JSON error; retry with `--workspace-id ID` only after selecting a listed ID. An explicit `--workspace-id` is accepted when its label is an exact canonical-label match, or—if no exact match exists—a suffix match; other IDs are refused. The picker still offers its interactive choice. Activating a project does not change the calling shell's cwd or move existing agent processes.

`worktree create` takes an existing repository-root candidate, a **local** branch, and a new destination. Without `--new-branch` the branch must already exist; with it, the branch must not exist and is created from the selected repo's HEAD. Git creates the linked checkout using `git worktree add` (non-force); creation alone does not focus a workspace. Destinations, including clone destinations, must not already exist. Relative destinations resolve against the process cwd. `--add-parent` explicitly adds an uncovered destination parent to the config **after** successful creation so the new checkout is discoverable by `list`, `open`, and `delete`. Without it, noninteractive creation leaves config unchanged; JSON outcomes report `discoverable` and `config_updated`. A Git success followed by a config write failure leaves the created directory in place and reports its path. A failed/interrupted Git operation also leaves any surviving destination untouched.

For deletion, `contx --json delete --dry-run <path>` returns a `preflight` object with `target` (including class and strategy), `warnings`, `blockers`, and `remote_verification`. Inspect it before considering `contx --json delete --force <path>`: `--force` accepts warnings but never bypasses hard blockers or opts into permanent deletion. JSON deletion without `--force` returns a confirmation-required error instead of prompting. Only a standalone-repo live delete fetches remotes; dry-run never does. Do not run from inside a target being deleted; active project panes can block it.

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

Clone protocol (SSH or HTTPS), repository path, editable protocol prefixes, destination, and the optional config-path choice are native controls. Directory creation only asks for a destination. Deletion preflight, findings, and confirmations are native controls. Interactive Git clone, fetch, and worktree deletion run in an embedded PTY/terminal area. Recognized Git/SSH prompts get native controls; unknown prompts remain usable through the terminal fallback.

While a Git child is running in the dialog, Ctrl-G requests cancellation; after a grace period an explicit Force Stop appears. Escape does not kill a running child. The picker’s Ctrl-G last-row motion applies only when the dialog is closed.

The picker clone composer starts with SSH selected. The repository path label shows `owner/repo` as a hint; the input remains empty. Type the repository path to produce `git@github.com:owner/repo`, or choose HTTPS for `https://github.com/owner/repo`. The destination is derived from the rightmost path component and keeps tracking source edits until the destination is manually edited. The protocol choices appear as a horizontal segmented switch and form one tab stop: Shift-Tab from the repository field focuses the currently selected protocol, and Left/Right or h/l selects a segment. Tab moves from protocol to repository to destination, then the **Edit prefixes** control and add-parent option; Shift-Tab moves back up the form. Prefix fields are hidden and skipped in the focus order until you expand **Edit prefixes** with Space. Space toggles **Edit prefixes** or add-parent when focused; Enter from any control submits the clone. Escape cancels. Invalid input displays an inline error and never starts Git. A slash is added between a prefix without a trailing `:` or `/` and the repository path.

Clone destination defaults relative to the focused group (the focused candidate’s group, or the focused header). Destination pre-fills with the repository name derived from the clone source the way `git clone` derives it (`<repo>` relative to the focused group, `~/<repo>` with no group), keeps tracking clone source edits until the field is edited, and is never overwritten after that. With no group, the destination must be absolute or `~`; relatives are never silently resolved against `$HOME` or the process working directory (the visible `~/` is editable text, not silent resolution). Directory creation uses the same destination rules but derives nothing from a source.

After a successful clone or directory creation, the catalog is rediscovered and the current query is kept. The new path is focused only if it is a discovered session candidate that matches that query; otherwise success is reported without inventing a row. After a successful deletion, the catalog is rediscovered, the query is kept, and focus moves to the nearest remaining visible row. An action that succeeds but whose refresh fails is still kept. Success shows a transient three-second message; cancellation is brief; errors and hard blockers remain until acknowledged.

Enter on a folded header never activates a project. Each candidate can show Git branch (or short SHA), dirty/add/delete counts, and cached upstream ahead/behind. Linked worktrees nest under their main checkout.

## Clone

```
contx clone [--add-parent] <source> [destination]
```

`<source>` is anything `git clone` accepts, including SSH. `contx` runs `git clone <source> <destination>` with no extra flags (no branch, depth, or submodule options). When destination is omitted, `<destination>` is the repository name derived from the clone source (same rule `git clone` uses), resolved from the process working directory. An explicit destination behaves as before. Git uses the terminal for auth and output.

The destination must not already exist. Relatives on the CLI are resolved from the process working directory (`~` and environment variables are expanded). `contx` prints the absolute destination, then clones.

Failure or interruption leaves any surviving destination in place and does not change the config. If the clone succeeds but a requested config update fails, the destination is kept and the command still fails.

When stdin is a TTY (outside JSON mode) and the destination is not already covered, `contx` asks whether to add the destination’s parent to `paths`. The question is asked before `git clone`; the file is written only after a successful clone. Declining still clones. `--add-parent` requests the same update without prompting, including in JSON mode.

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

After Enter on a directory (or `contx open <path>`):

- **tmux:** switch to the session named from that path. If none exists, create one there and switch.
- **Herdr:** prefer an existing workspace whose label matches the canonical target label, then a label ending with the target directory name. A label that merely contains the directory name is not a match; if neither match exists, create a workspace at the path with the canonical label (`--cwd`, `--label`, `--focus`). Live pane working directories do not make an unrelated workspace a match.

If a target was seen and is gone before the switch/focus, `contx` reports it and does not recreate it. Existing layout and processes are left as-is.

Herdr extras:

- Binary: nonempty `HERDR_BIN_PATH`, otherwise `herdr` on `PATH`. A set but unusable `HERDR_BIN_PATH` does not fall back to `PATH`.
- Several name-matching workspaces: a numbered prompt on a TTY stdin. Empty input, `q`, or EOF cancels with no focus and no create. Non-TTY stdin lists the IDs and exits.

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

- No Herdr pane, tab, or agent management in the TUI, and no control of one multiplexer from the other. CLI worktree creation uses Git; it does not attach Herdr worktree metadata. Clone and delete act on session candidates (including Git linked worktrees), not on Herdr worktrees.
- Herdr target resolution uses workspace labels, not live pane working directories. A shell that `cd`s away does not change which named workspace is selected; an open tab in an unrelated workspace does not make that workspace a match.
- Workspace IDs are not persisted. Disappeared targets are not recreated.
- No silent fallback between tmux and Herdr; no server start or attach. If `auto` picks the wrong nested multiplexer, pass `--multiplexer` explicitly.
