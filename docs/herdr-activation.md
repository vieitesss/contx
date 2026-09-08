# Herdr activation

`contx` can activate a **project target** in tmux or Herdr. Candidates, Git, and the TUI stay backend-agnostic. Activation only: `contx` does not list or manage Herdr panes, tabs, agents, or worktrees, and it does not control one multiplexer from the other.

See `CONTEXT.md` for terms. A **project target** is a tmux **session** or a Herdr **workspace**. A **Herdr session** is the server namespace, not a project target.

## Outcomes

Select a session candidate in the TUI, then `contx` activates the matching project target in the current multiplexer.

| Outcome | tmux | Herdr |
|---|---|---|
| Unique existing target | `switch-client` to the derived session | `workspace focus` on the discovered workspace ID |
| No match | `new-session -ds` at the candidate path, then switch | `workspace create --cwd --label --focus` |
| Observed target gone before focus | Report; never recreate | Report; never recreate |
| Layout and processes | Leave the existing session as-is | Leave the existing workspace as-is |

Convenience differences, not bugs:

- tmux names are derived and looked up by name; Herdr workspace IDs are opaque, server-scoped, and discovered live, never stored.
- tmux `switch-client` moves the current client; Herdr `workspace focus` focuses all server clients.
- Herdr create also creates the first tab and root shell.
- On create, `--label` uses the same rules as tmux session names. Labels are never used to reuse a workspace.

tmux naming and lookup are unchanged.

## Choosing a multiplexer

```
contx --multiplexer auto|tmux|herdr
```

Config (`~/.config/contx/config.toml`, or `-c` / `--config-file`):

```toml
multiplexer = "auto"   # auto | tmux | herdr
```

CLI `--multiplexer` wins over the file. Missing key or missing implicit config file means `auto`. Unknown values are errors. `--help` / `-h` prints usage.

`auto` picks the **inner multiplexer** when nested:

- tmux inside Herdr (this terminal is a live tmux pane or client) → tmux
- Herdr inside tmux (inherited `TMUX`, but this terminal is not a live tmux pane or client) → Herdr

Markers inherit both ways, so they are not enough. When `TMUX` is set, detection probes `tmux list-panes -a` (`#{pane_tty}`) and `tmux list-clients` (`#{client_tty}`) and compares them to the controlling TTY. That probe is for detection only. After a backend is chosen, activation talks only to that backend. Errors never fall back to the other, and `contx` never launches or attaches a server on its own.

Herdr is recognized only with `HERDR_ENV=1` and a nonempty `HERDR_SOCKET_PATH`. Explicit `--multiplexer herdr` uses the same check and refuses with `not running inside herdr` if either is missing, so a default Herdr server is never targeted. Explicit `--multiplexer tmux` skips the inner-vs-outer TTY probe; tmux still requires nonempty `TMUX`.

Detection fails closed and asks for an override instead of guessing:

| Situation | What happens |
|---|---|
| Neither multiplexer | `not running inside a multiplexer; pass --multiplexer tmux or --multiplexer herdr` |
| Both markers, TTY unknown or tmux probe failed | `ambiguous multiplexer; pass --multiplexer tmux or --multiplexer herdr` |
| Only `TMUX`, TTY unknown or probe failed | `could not confirm tmux owns this terminal; …` |
| Only `TMUX`, TTY known but not a live pane or client | `TMUX is set but this terminal is not a live tmux pane or client; …` |
| `HERDR_ENV=1` without `HERDR_SOCKET_PATH` | `HERDR_ENV is set but HERDR_SOCKET_PATH is missing; …` |

This environment has had no live nested-Herdr validation. The controlling-TTY check is implemented on Unix (`/dev/tty` + `ttyname`) and covered by fixtures; it is not proven against a live Herdr-inside-tmux or tmux-inside-Herdr session here.

## Herdr matching

The Herdr binary is `$HERDR_BIN_PATH` when that variable is nonempty, otherwise `herdr` on `PATH`. A set but unlaunchable `HERDR_BIN_PATH` does not fall back to `PATH`.

`contx` calls:

- `herdr pane list`
- `herdr workspace focus <workspace_id>`
- `herdr workspace create --cwd <path> --label <name> --focus`

A workspace has no public immutable path. Matching:

1. Canonicalize the selected candidate (failure refuses; nothing is created).
2. Exact match on a pane’s canonical `cwd` or `foreground_cwd`.
3. Dedupe by workspace ID.
4. One workspace → focus it. If it is gone (`workspace_not_found`), report disappearance and do not recreate.
5. Several workspaces → numbered choice on stderr when stdin is a TTY:

   ```
   Multiple Herdr workspaces match `/path/to/project`:
     1) 3
     2) 8
   Enter number to activate, or empty to cancel:
   ```

   A number focuses that workspace. Empty input, `q`, or EOF cancels: success, no focus, no create. Invalid input is an error and does not create. If stdin is not a TTY, `contx` lists the IDs and exits without asking or creating.
6. None → create at the canonical path.
7. Never reuse by label.

Accepted live-cwd limitation: a shell that `cd`s away can make the original workspace unmatchable, so a later activation creates a duplicate; a pane that `cd`s into another project can cause that other workspace to be reused. `contx` does not keep an external index, metadata token, private session file, or persisted ID.

## Invocation

`contx` does not close picker panes. Example only; it does not edit dotfiles.

Herdr custom command syntax (`[[keys.command]]`, `type = "pane"` opens a temporary pane and closes it when the command exits). Closing that pane is Herdr’s, not `contx`’s:

```toml
[[keys.command]]
key = "prefix+o"
type = "pane"
command = "contx"
```

## What it does not do

- List or manage Herdr panes, tabs, agents, or worktrees in the TUI
- Control tmux from Herdr or Herdr from tmux
- Replace `herdr-spaces` in dotfiles
- Persist workspace IDs or last-workspace state
- Recreate a disappeared project target
- Fall back silently from one multiplexer to the other
- Start or attach a multiplexer server
