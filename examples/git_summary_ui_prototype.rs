// PROTOTYPE (throwaway, not for production).
//
// Question: "What should live Git change summaries look like on
// session candidates?"
//
// One Ledger-only executable for live Git change summaries:
// background Git polling plus the approved glyph/color treatment
// are in. Nothing here may be promoted to production as-is;
// delete this file once the design question is settled.

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyModifiers},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::{border, line},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    env, fs, io,
    path::Path,
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

const DEFAULT_CONFIG_FILE: &str = "~/.config/contx/config.toml";
const POLL_TICK: Duration = Duration::from_millis(100);
const SELECTED_BG: Color = Color::Rgb(0xCE, 0xC9, 0xC0);
/// Light-theme border titles: Gruber Lighter `@color_accent`
/// (niagara) from
/// `~/personal/gruber-darker-tmux-theme/gruber-lighter-tmux-theme.tmux`
/// (`#37526D`). Semantic accent role, more colorful than gray.
const TITLE_FG: Color = Color::Rgb(0x37, 0x52, 0x6D);
/// Mixed list frame: solid horizontals/corners from
/// `border::PLAIN`, dashed verticals from
/// `border::LIGHT_DOUBLE_DASHED`. Applied via `border_set`.
const LIST_BORDER: border::Set = border::Set {
    vertical_left: border::LIGHT_DOUBLE_DASHED.vertical_left,
    vertical_right: border::LIGHT_DOUBLE_DASHED.vertical_right,
    ..border::PLAIN
};
/// Interior divider for the mixed frame: library vertical
/// `╎` (`line::LIGHT_DOUBLE_DASH_VERTICAL` backing
/// `border::LIGHT_DOUBLE_DASHED`). Junctions stay library
/// `┬`/`┴` (`line::HORIZONTAL_DOWN`/`HORIZONTAL_UP`).
const DIVIDER: &str = line::LIGHT_DOUBLE_DASH_VERTICAL;

#[derive(Deserialize)]
struct RawConfig {
    paths: Option<Vec<String>>,
    #[serde(rename = "git-from-home")]
    git_from_home: Option<bool>,
}

// NOTE (future): an agent column would be fixed-width; its
// placement is unresolved, including far right. No placeholder
// or generic column abstraction lives here.

struct App {
    candidates: Vec<String>,
    display: Vec<String>,
    query: String,
    selected: usize,
    /// Latest worker snapshot per absolute candidate path.
    /// Missing entries are still loading. This map is the only
    /// thing Git ticks may change; candidates, query, and
    /// selection stay stable.
    states: HashMap<String, CandidateState>,
    rx: mpsc::Receiver<Vec<(String, CandidateState)>>,
    scroll: usize,
}

impl App {
    /// Indices into `candidates` matching the query, in original
    /// order. The candidate list itself never changes after load,
    /// so later Git ticks cannot reorder, refilter, or retarget.
    fn filtered(&self) -> Vec<usize> {
        (0..self.candidates.len())
            .filter(|&i| matches_query(&self.display[i], &self.query))
            .collect()
    }

    /// Approved-color style for one status fragment. Colors stay
    /// on the selected row too: glyphs are bold and bright
    /// against the production `bg_alt` selection background.
    fn spot(color: Color) -> Style {
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    }

    /// Combined GIT cell contents for one row, left-aligned
    /// and right-padded with unstyled spaces to exactly
    /// `git_w`. Nonrepo is blank; `git_w == 0` hides the cell
    /// entirely. Measurable rows show both counts or neither
    /// (never plus-only), compacted with integer `k`/`m`/`b`
    /// units; glyph only when no pair fits. Every Git-backed
    /// row reports its state: blue bold icon when clean, icon
    /// plus green `+N` and red `−N` (U+2212) when measurable,
    /// icon plus yellow `•` when dirty without lines, red ``
    /// on inspect failure, dim gray `…` while loading. PUA
    /// icons count as one cell each.
    fn git_cell_spans(&self, i: usize, git_w: usize) -> Vec<Span<'static>> {
        if git_w == 0 {
            return vec![];
        }
        let icon_of = |linked: bool| {
            Span::styled(work_icon(linked), Self::spot(Color::Blue))
        };
        let pad = |spans: Vec<Span<'static>>| {
            let used = status_width(&spans).min(git_w);
            let mut out = spans;
            out.push(Span::from(" ".repeat(git_w - used)));
            out
        };
        match self.states.get(&self.candidates[i]) {
            None => pad(vec![Span::styled("…", Style::new().fg(Color::Gray))]),
            // Inspect failure first: red regardless of whether
            // a root resolved, so a failed resolution under an
            // enclosing `.git` never reads as nonrepo.
            Some(s) if s.state == WorkState::Failed => {
                pad(vec![Span::styled("\u{f467}", Self::spot(Color::Red))])
            }
            Some(s) if s.root.is_none() => {
                vec![Span::from(" ".repeat(git_w))]
            }
            Some(s) => match s.state {
                WorkState::Clean => pad(vec![icon_of(s.linked)]),
                WorkState::Measurable { added, deleted } => {
                    let budget = git_w.saturating_sub(2);
                    match measurable_texts(added, deleted, budget) {
                        Some((a, d)) => pad(vec![
                            icon_of(s.linked),
                            Span::from(" "),
                            Span::styled(a, Self::spot(Color::Green)),
                            Span::from(" "),
                            Span::styled(d, Self::spot(Color::Red)),
                        ]),
                        // No pair fits: glyph only, never
                        // plus-only or minus-only.
                        None => pad(vec![icon_of(s.linked)]),
                    }
                }
                WorkState::Marker => {
                    let full = vec![
                        icon_of(s.linked),
                        Span::from(" "),
                        Span::styled("•", Self::spot(Color::Yellow)),
                    ];
                    if status_width(&full) <= git_w {
                        pad(full)
                    } else {
                        // Narrow: keep the glyph, drop `•`.
                        pad(vec![icon_of(s.linked)])
                    }
                }
                // Failed is caught by the outer arm above.
                WorkState::Failed => pad(vec![icon_of(s.linked)]),
            },
        }
    }

    /// One shared row spanning both columns: `[GIT cell][╎][PATH]`.
    /// A single `Line` keeps scrolling and selection highlight
    /// synchronized across columns; no independently navigable
    /// panes. The divider is the library dashed vertical `╎`
    /// matching the mixed frame's dashed vertical sides.
    /// GIT width comes from `git_col_width` only, so the
    /// query never moves the columns. PATH fills the remaining
    /// width with middle truncation; as terminals shrink both
    /// counts hide together (`git_w == 2`) then the whole GIT
    /// cell (`git_w == 0`), never only `+N` or only `−N`.
    fn render_entry(
        &self,
        i: usize,
        selected: bool,
        inner_w: usize,
    ) -> Vec<Line<'static>> {
        let git_w = git_col_width(inner_w);
        let gap = usize::from(git_w > 0);
        let path_w = inner_w.saturating_sub(git_w + gap).max(1);
        let mut spans = self.git_cell_spans(i, git_w);
        if git_w > 0 {
            // Dashed vertical divider matching the mixed
            // frame's dashed vertical sides; patching
            // bg below keeps the selection highlight
            // synchronized across both columns in one shared
            // `Line`.
            spans.push(Span::from(DIVIDER));
        }
        spans.push(Span::from(truncate_path(&self.display[i], path_w)));
        if selected {
            for span in &mut spans {
                span.style = span.style.bg(SELECTED_BG);
            }
            vec![Line::from(spans).style(Style::new().bg(SELECTED_BG))]
        } else {
            vec![Line::from(spans)]
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.filtered().len();
        if len == 0 {
            return;
        }
        self.selected = self.selected.min(len - 1);
    }

    /// Handle one key press. Returns true when the UI should exit.
    /// Digits (including `1`/`2`/`3`) append to the query;
    /// Left/Right are ignored; Enter deliberately does nothing
    /// (no tmux in a prototype).
    fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        use event::KeyEventKind::Press;
        if key.kind != Press {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('c') {
                return true;
            }
            return false;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        match key.code {
            KeyCode::Char(c) => {
                self.query.push(c);
                self.clamp_selection();
            }
            KeyCode::Backspace => {
                let _ = self.query.pop();
                self.clamp_selection();
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                let len = self.filtered().len();
                if self.selected + 1 < len {
                    self.selected += 1;
                }
            }
            _ => {}
        }
        false
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            while let Ok(msg) = self.rx.try_recv() {
                self.states = msg.into_iter().collect();
            }
            terminal.draw(|frame| self.render(frame))?;
            if event::poll(POLL_TICK)? {
                if let Event::Key(key) = event::read()? {
                    if self.handle_key(key) {
                        return Ok(());
                    }
                }
            }
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(frame.area());

        let filtered = self.filtered();
        let ready = filtered
            .iter()
            .filter(|&&i| self.states.contains_key(&self.candidates[i]))
            .count();
        let sel_path = filtered
            .get(self.selected)
            .map(|&i| self.candidates[i].as_str())
            .unwrap_or("—");
        frame.render_widget(
            Line::from(format!(
                "PROTOTYPE git-summary · {}/{} shown · sel {} · \
                 {}/{} ready · query {}",
                filtered.len(),
                self.candidates.len(),
                sel_path,
                ready,
                filtered.len(),
                self.query,
            )),
            areas[0],
        );

        frame.render_widget(
            Line::from(vec![
                "> ".into(),
                self.query.as_str().into(),
                "█".into(),
            ]),
            areas[1],
        );

        let inner_w = (areas[2].width as usize).saturating_sub(2);
        let inner_h = (areas[2].height as usize).saturating_sub(2);
        let git_w = git_col_width(inner_w);
        let gap = usize::from(git_w > 0);
        let path_w = inner_w.saturating_sub(git_w + gap);
        // No header content row: `git`/`path` titles live in the
        // top border. One shared list scrolls as a whole; each
        // combined `Line` carries selection bg across both
        // columns so Git and path stay synchronized.
        let view_h = inner_h;
        let mut lines: Vec<Line> = vec![];
        let mut sel_start = 0;
        for (pos, &i) in filtered.iter().enumerate() {
            let entry_selected = pos == self.selected;
            if entry_selected {
                sel_start = lines.len();
            }
            lines.extend(self.render_entry(i, entry_selected, inner_w));
        }
        if view_h == 0 {
            self.scroll = self.scroll.min(lines.len());
        } else {
            if sel_start < self.scroll {
                self.scroll = sel_start;
            } else if sel_start + 1 > self.scroll + view_h {
                self.scroll = sel_start + 1 - view_h;
            }
            let max = lines.len().saturating_sub(view_h);
            self.scroll = self.scroll.min(max);
        }
        let visible: Vec<Line> =
            lines.into_iter().skip(self.scroll).take(view_h).collect();
        let filled = visible.len();
        frame.render_widget(
            Paragraph::new(visible)
                .block(Block::bordered().border_set(LIST_BORDER)),
            areas[2],
        );
        draw_list_chrome(frame, areas[2], git_w, path_w, filled);
    }
}

/// Worktree icon: ordinary checkout vs linked worktree.
/// Bold, bright, and padded at each use site.
fn work_icon(linked: bool) -> &'static str {
    if linked { "\u{ec7d}" } else { "\u{ec6f}" }
}

/// Fixed GIT column width from the inner list width only:
/// never the query, filter set, or poll snapshot. A full cell
/// needs 12; below 29 only the glyph survives (2); below 19
/// the cell hides so a useful path remains.
fn git_col_width(inner_w: usize) -> usize {
    if inner_w >= 29 {
        12
    } else if inner_w >= 19 {
        2
    } else {
        0
    }
}

/// Lowercase `git` title clipped to the Git width. Glyph-only
/// `git_w == 2` shows `gi`; `0` hides the title entirely.
fn git_border_title(git_w: usize) -> String {
    "git".chars().take(git_w).collect()
}

/// Lowercase `path` title clipped to the path width.
fn path_border_title(path_w: usize) -> String {
    "path".chars().take(path_w).collect()
}

/// Connected mixed list chrome: lowercase accent titles embedded
/// in the top border plus a library dashed vertical divider (`╎`
/// with `┬`/`┴` junctions from `line::NORMAL`). Outer frame is
/// the mixed `border::Set` (`─`/`╎`), never a hand-coded dash
/// alphabet. Hidden Git (`git_w == 0`) drops the
/// section and divider, leaving a path-only frame.
/// `inner_w = width - 2` keeps the established path-width reserve.
/// Empty filler rows get `╎`; content rows already carry `╎` with
/// selection bg, which is left alone. Titles use the Gruber
/// Lighter `@color_accent` niagara `#37526D`. All writes use
/// `cell_mut`, so tiny/zero areas never panic. Light theme only.
fn draw_list_chrome(
    frame: &mut Frame,
    area: Rect,
    git_w: usize,
    path_w: usize,
    filled_rows: usize,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let style = Style::new().fg(TITLE_FG);
    let right = area.x.saturating_add(area.width.saturating_sub(1));
    let top_y = area.y;
    for (k, ch) in git_border_title(git_w).chars().enumerate() {
        let x = area.x.saturating_add(1).saturating_add(k as u16);
        if x >= right {
            break;
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((x, top_y)) {
            cell.set_symbol(&ch.to_string());
            cell.set_style(style);
        }
    }
    if git_w == 0 {
        for (k, ch) in path_border_title(path_w).chars().enumerate() {
            let x = area.x.saturating_add(1).saturating_add(k as u16);
            if x >= right {
                break;
            }
            if let Some(cell) = frame.buffer_mut().cell_mut((x, top_y)) {
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
        }
        return;
    }
    let div_x = area.x.saturating_add(1).saturating_add(git_w as u16);
    if div_x >= right {
        return;
    }
    if let Some(cell) = frame.buffer_mut().cell_mut((div_x, top_y)) {
        cell.set_symbol(line::HORIZONTAL_DOWN);
    }
    if area.height >= 2 {
        let bot_y = area.y.saturating_add(area.height.saturating_sub(1));
        if let Some(cell) = frame.buffer_mut().cell_mut((div_x, bot_y)) {
            cell.set_symbol(line::HORIZONTAL_UP);
        }
    }
    let inner_h = (area.height as usize).saturating_sub(2);
    for row in filled_rows..inner_h {
        let y = area.y.saturating_add(1).saturating_add(row as u16);
        if let Some(cell) = frame.buffer_mut().cell_mut((div_x, y)) {
            cell.set_symbol(DIVIDER);
        }
    }
    for (k, ch) in path_border_title(path_w).chars().enumerate() {
        let x = div_x.saturating_add(1).saturating_add(k as u16);
        if x >= right {
            break;
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((x, top_y)) {
            cell.set_symbol(&ch.to_string());
            cell.set_style(style);
        }
    }
}

/// One side of a measurable pair at unit `level`: 0 exact, 1
/// `k`, 2 `m`, 3 `b`. Integer division only (`u64` `/`, no
/// floats, no overflow); `None` below the unit's threshold, so
/// `0k`/`0m`/`0b` never render.
fn fmt_count(n: u64, level: u8) -> Option<String> {
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
fn measurable_texts(
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

/// Visible width of styled fragments, counting characters.
fn status_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// Path-aware middle truncation to at most `max` chars: the
/// full string when it fits; else a leading prefix + `…`
/// (U+2026) + full basename when that fits; else `…` + tail.
/// `~/` survives whenever the prefix budget reaches 2.
fn truncate_path(display: &str, max: usize) -> String {
    let len = display.chars().count();
    if len <= max {
        return display.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let basename = display.rsplit('/').next().unwrap_or(display);
    let base_len = basename.chars().count();
    if basename != display && base_len + 2 <= max {
        let prefix: String = display.chars().take(max - base_len - 1).collect();
        return format!("{prefix}…{basename}");
    }
    let tail: String = display
        .chars()
        .skip(len.saturating_sub(max.saturating_sub(1)))
        .collect();
    format!("…{tail}")
}

/// Case-sensitive subsequence match on the display path. An empty
/// query matches everything, preserving original order.
fn matches_query(display: &str, query: &str) -> bool {
    let mut rest = display.chars();
    query.chars().all(|q| rest.any(|c| c == q))
}

/// Display `$HOME` itself as `~` and descendants as `~/...`,
/// only at a path-component boundary.
fn display_path(path: &str, home: &str) -> String {
    if home.is_empty() {
        return path.to_string();
    }
    if path == home {
        return "~".to_string();
    }
    match path.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// Change summary for one session candidate, computed off the UI
/// thread. `root` is the resolved worktree; `None` means the
/// candidate sits in no Git worktree.
#[derive(Clone)]
struct CandidateState {
    root: Option<String>,
    linked: bool,
    state: WorkState,
}

#[derive(Clone, Copy, PartialEq)]
enum WorkState {
    Clean,
    Measurable { added: u64, deleted: u64 },
    Marker,
    Failed,
}

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const SCAN_EVERY: Duration = Duration::from_secs(1);

/// One `git` invocation. Anything failing (missing git, vanished
/// repo, bad output) is `None`; the caller maps that to Failed.
fn git(root: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Resolution outcome for one candidate: no enclosing `.git`
/// marker anywhere upward (nonrepo), a resolved worktree, or an
/// enclosing marker whose git inspection failed.
enum RootResolve {
    Nonrepo,
    Root(String, bool),
    Failed,
}

/// Nearest enclosing non-bare worktree root plus whether it is a
/// linked worktree (`.git` is a file). The marker search is a
/// filesystem walk, so a missing marker (nonrepo) never runs
/// git; any git failure under an existing marker is Failed.
/// Bare repositories stay in the nonrepo bucket. Nested
/// candidates inherit the root.
fn resolve_root(candidate: &str) -> RootResolve {
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
    let root = match git(&top, &["rev-parse", "--show-toplevel"]) {
        Some(t) => t.trim().to_string(),
        None => return RootResolve::Failed,
    };
    match git(&top, &["rev-parse", "--is-bare-repository"]) {
        Some(b) if b.trim() == "true" => return RootResolve::Nonrepo,
        Some(_) => {}
        None => return RootResolve::Failed,
    }
    let git_path = Path::new(&root).join(".git");
    let linked = fs::symlink_metadata(&git_path)
        .map(|m| !m.file_type().is_dir())
        .unwrap_or(true);
    RootResolve::Root(root, linked)
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
fn scan_root(root: &str) -> WorkState {
    let status = match git(
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
    // conversion in `git`, surfacing as Failed.
    for field in status.split('\0') {
        if field.is_empty() {
            continue;
        }
        let raw = field.as_bytes();
        if raw.len() < 4 || raw[2] != b' ' {
            continue;
        }
        dirty = true;
        if raw[0] == b'?' && raw[1] == b'?' {
            untracked.push(&field[3..]);
        }
    }
    let head = git(root, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| EMPTY_TREE.to_string());
    let numstat = match git(root, &["diff", "--numstat", "-M", &head]) {
        Some(n) => n,
        None => return WorkState::Failed,
    };
    let mut added = 0u64;
    let mut deleted = 0u64;
    for line in numstat.lines() {
        dirty = true;
        let mut cols = line.split('\t');
        if let (Some(a), Some(d)) = (cols.next(), cols.next()) {
            if let (Ok(x), Ok(y)) = (a.parse::<u64>(), d.parse::<u64>()) {
                added += x;
                deleted += y;
            }
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
        // No size cap: every non-ignored untracked text file
        // contributes its lines as additions.
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

/// Background worker: resolve each candidate once, then rescan
/// each unique worktree about once per second. Never touches
/// the UI; snapshots travel over the channel.
fn poll_worker(
    candidates: Vec<String>,
    tx: mpsc::Sender<Vec<(String, CandidateState)>>,
) {
    let mut linked: HashMap<String, bool> = HashMap::new();
    let mut mapping: HashMap<String, RootResolve> = HashMap::new();
    for c in &candidates {
        let resolved = resolve_root(c);
        if let RootResolve::Root(r, l) = &resolved {
            linked.insert(r.clone(), *l);
        }
        mapping.insert(c.clone(), resolved);
    }
    let unique: Vec<String> = linked.keys().cloned().collect();
    loop {
        let states: HashMap<String, WorkState> =
            unique.iter().map(|r| (r.clone(), scan_root(r))).collect();
        let msg: Vec<(String, CandidateState)> = candidates
            .iter()
            .map(|c| {
                let (root, l, s) = match mapping.get(c) {
                    Some(RootResolve::Root(r, _)) => (
                        Some(r.clone()),
                        linked.get(r).copied().unwrap_or(false),
                        states.get(r).copied().unwrap_or(WorkState::Failed),
                    ),
                    // Enclosing marker but git failed: Failed,
                    // never the nonrepo path-only row.
                    Some(RootResolve::Failed) => {
                        (None, false, WorkState::Failed)
                    }
                    // Nonrepo placeholder; the row ignores it.
                    _ => (None, false, WorkState::Clean),
                };
                (
                    c.clone(),
                    CandidateState {
                        root,
                        linked: l,
                        state: s,
                    },
                )
            })
            .collect();
        if tx.send(msg).is_err() {
            return;
        }
        thread::sleep(SCAN_EVERY);
    }
}

fn fail(message: &str) -> ! {
    eprintln!("git_summary_ui_prototype: {message}");
    std::process::exit(1);
}

fn expand(path: &str) -> String {
    shellexpand::full_with_context(
        path,
        || env::var("HOME").ok(),
        |name| env::var(name).map(Some),
    )
    .map(|cow| cow.into_owned())
    .unwrap_or_else(|_| fail(&format!("bad path: {path}")))
}

fn inner_dirs(dir: &str) -> Vec<String> {
    let entries = Path::new(dir)
        .read_dir()
        .unwrap_or_else(|_| fail(&format!("unreadable dir: {dir}")));
    entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.path().display().to_string())
        .collect()
}

fn normalize_path(path: &str) -> Vec<String> {
    let expanded = expand(path);
    if !expanded.starts_with('/') {
        fail(&format!("not absolute: {path}"));
    }
    if Path::new(&expanded).is_dir() {
        return inner_dirs(&expanded);
    }
    if let Some(dir) = expanded.strip_suffix("/*") {
        if !Path::new(dir).is_dir() {
            fail(&format!("not a directory: {path}"));
        }
        let mut all = vec![];
        for child in inner_dirs(dir) {
            all.append(&mut inner_dirs(&child));
        }
        return all;
    }
    fail(&format!("not a directory: {path}"));
}

fn git_repos_from_home(home: &str) -> Vec<String> {
    let entries = Path::new(home)
        .read_dir()
        .unwrap_or_else(|_| fail("cannot read $HOME"));
    let mut repos: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| p.join(".git").exists())
        .map(|p| p.display().to_string())
        .collect();
    repos.sort();
    repos
}

fn load_candidates(config_file: &str, explicit: bool) -> Vec<String> {
    let content = match fs::read_to_string(config_file) {
        Ok(content) => content,
        Err(_) if !explicit => return vec![],
        Err(_) => fail(&format!("cannot read: {config_file}")),
    };
    let raw: RawConfig =
        toml::from_str(&content).unwrap_or_else(|_| fail("malformed config"));
    let mut configured = vec![];
    for path in raw.paths.unwrap_or_default() {
        configured.append(&mut normalize_path(&path));
    }
    let discovered = if raw.git_from_home.unwrap_or(false) {
        match env::var("HOME") {
            Ok(home) if !home.is_empty() => git_repos_from_home(&home),
            _ => fail("$HOME is not set"),
        }
    } else {
        vec![]
    };
    let mut seen = HashSet::new();
    configured
        .into_iter()
        .chain(discovered)
        .filter(|p| {
            let id = Path::new(p)
                .canonicalize()
                .map(|c| c.display().to_string())
                .unwrap_or_else(|_| p.clone());
            seen.insert(id)
        })
        .collect()
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut config_file = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config-file" | "-c" => {
                let value =
                    args.next().unwrap_or_else(|| fail("missing value"));
                let expanded = expand(value);
                if !Path::new(&expanded).exists() {
                    fail(&format!("not found: {value}"));
                }
                config_file = Some(expanded);
            }
            _ => fail(&format!("unknown arg: {arg}")),
        }
    }
    let (config_file, explicit) = match config_file {
        Some(f) => (f, true),
        None => (expand(DEFAULT_CONFIG_FILE), false),
    };
    let home = env::var("HOME").unwrap_or_default();
    let candidates = load_candidates(&config_file, explicit);
    let display = candidates.iter().map(|p| display_path(p, &home)).collect();
    let (tx, rx) = mpsc::channel();
    let worker_candidates = candidates.clone();
    thread::spawn(move || poll_worker(worker_candidates, tx));
    let mut app = App {
        candidates,
        display,
        query: String::new(),
        selected: 0,
        states: HashMap::new(),
        rx,
        scroll: 0,
    };
    if let Err(e) =
        ratatui::run(|terminal: &mut DefaultTerminal| app.run(terminal))
    {
        fail(&format!("terminal error: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        root: Option<&str>,
        linked: bool,
        state: WorkState,
    ) -> CandidateState {
        CandidateState {
            root: root.map(str::to_string),
            linked,
            state,
        }
    }

    fn app_with(state: Option<CandidateState>) -> App {
        let (_, rx) = mpsc::channel();
        let mut states = HashMap::new();
        if let Some(s) = state {
            states.insert("/repo".to_string(), s);
        }
        App {
            candidates: vec!["/repo".to_string()],
            display: vec!["/repo".to_string()],
            query: String::new(),
            selected: 0,
            states,
            rx,
            scroll: 0,
        }
    }

    fn cell_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn git_col_width_tiers() {
        assert_eq!(git_col_width(0), 0);
        assert_eq!(git_col_width(18), 0);
        assert_eq!(git_col_width(19), 2);
        assert_eq!(git_col_width(28), 2);
        assert_eq!(git_col_width(29), 12);
        assert_eq!(git_col_width(200), 12);
    }

    #[test]
    fn query_does_not_size_git_cell() {
        let plain =
            app_with(Some(candidate(Some("/r"), false, WorkState::Clean)));
        let mut queried =
            app_with(Some(candidate(Some("/r"), false, WorkState::Clean)));
        queried.query = "zzz".to_string();
        assert_eq!(plain.filtered().len(), 1);
        assert!(queried.filtered().is_empty());
        for inner_w in [10, 18, 19, 28, 29, 80] {
            let git_w = git_col_width(inner_w);
            assert_eq!(
                cell_text(&plain.git_cell_spans(0, git_w)),
                cell_text(&queried.git_cell_spans(0, git_w)),
            );
        }
    }

    #[test]
    fn loading_clean_measurable_share_width() {
        let cases = [
            None,
            Some(candidate(Some("/r"), false, WorkState::Clean)),
            Some(candidate(
                Some("/r"),
                false,
                WorkState::Measurable {
                    added: 12_345_678,
                    deleted: 9,
                },
            )),
        ];
        for state in cases {
            let app = app_with(state);
            let t = cell_text(&app.git_cell_spans(0, 12));
            assert_eq!(t.chars().count(), 12);
        }
    }

    #[test]
    fn cell_fragments_and_padding() {
        let clean =
            app_with(Some(candidate(Some("/r"), false, WorkState::Clean)));
        assert_eq!(
            cell_text(&clean.git_cell_spans(0, 12)),
            format!("\u{ec6f}{}", " ".repeat(11)),
        );
        let linked =
            app_with(Some(candidate(Some("/r"), true, WorkState::Clean)));
        assert_eq!(
            cell_text(&linked.git_cell_spans(0, 12)),
            format!("\u{ec7d}{}", " ".repeat(11)),
        );
        let nonrepo = app_with(Some(candidate(None, false, WorkState::Clean)));
        assert_eq!(cell_text(&nonrepo.git_cell_spans(0, 12)), " ".repeat(12),);
        let loading = app_with(None);
        assert_eq!(
            cell_text(&loading.git_cell_spans(0, 12)),
            format!("…{}", " ".repeat(11)),
        );
        let failed =
            app_with(Some(candidate(Some("/r"), false, WorkState::Failed)));
        assert_eq!(
            cell_text(&failed.git_cell_spans(0, 12)),
            format!("\u{f467}{}", " ".repeat(11)),
        );
        let marker =
            app_with(Some(candidate(Some("/r"), false, WorkState::Marker)));
        assert_eq!(
            cell_text(&marker.git_cell_spans(0, 12)),
            format!("\u{ec6f} •{}", " ".repeat(9)),
        );
        let small = app_with(Some(candidate(
            Some("/r"),
            false,
            WorkState::Measurable {
                added: 3,
                deleted: 12,
            },
        )));
        let t = cell_text(&small.git_cell_spans(0, 12));
        assert!(t.contains("\u{2212}"));
        assert!(!t.contains('-'));
        assert_eq!(t, format!("\u{ec6f} +3 −12{}", " ".repeat(4)),);
        // Padding is unstyled spaces, never colored.
        let cell = small.git_cell_spans(0, 12);
        let pad = cell.last().unwrap();
        assert!(pad.content.chars().all(|c| c == ' '));
        assert_eq!(pad.style, Style::default());
    }

    #[test]
    fn oversized_counts_stay_paired_inside_width() {
        let big = app_with(Some(candidate(
            Some("/r"),
            false,
            WorkState::Measurable {
                added: 12_345_678,
                deleted: 9,
            },
        )));
        let t = cell_text(&big.git_cell_spans(0, 12));
        assert_eq!(t.chars().count(), 12);
        assert!(t.contains('+') && t.contains('−'));
        // Extremes fall back to glyph only, never one-sided.
        let extreme = app_with(Some(candidate(
            Some("/r"),
            false,
            WorkState::Measurable {
                added: u64::MAX,
                deleted: u64::MAX,
            },
        )));
        let t = cell_text(&extreme.git_cell_spans(0, 12));
        assert_eq!(t.chars().count(), 12);
        assert!(!t.contains('+') && !t.contains('−'));
    }

    #[test]
    fn count_units_and_extremes() {
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
        // Exact pair wins when it fits.
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
    }

    #[test]
    fn truncate_path_cases() {
        assert_eq!(truncate_path("~/a/bb", 99), "~/a/bb");
        assert_eq!(truncate_path("~/a/bb", 6), "~/a/bb");
        assert_eq!(
            truncate_path("~/projects/foobar/longname", 12),
            "~/p…longname",
        );
        assert_eq!(truncate_path("~/projects/foobar/longname", 8), "…ongname",);
        assert_eq!(truncate_path("abcdefgh", 4), "…fgh");
    }

    #[test]
    fn narrow_widths_hide_counts_then_git() {
        let dirty = app_with(Some(candidate(
            Some("/r"),
            false,
            WorkState::Measurable {
                added: 3,
                deleted: 4,
            },
        )));
        // Both counts hide together; the glyph stays.
        let t = cell_text(&dirty.git_cell_spans(0, 2));
        assert_eq!(t.chars().count(), 2);
        assert!(!t.contains('+') && !t.contains('−'));
        let marker =
            app_with(Some(candidate(Some("/r"), false, WorkState::Marker)));
        assert!(!cell_text(&marker.git_cell_spans(0, 2)).contains('•'));
        // The whole cell hides below 19; the row is path only.
        assert!(dirty.git_cell_spans(0, 0).is_empty());
        let line = &dirty.render_entry(0, false, 18)[0];
        assert_eq!(line_text(line), "/repo");
    }

    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    fn app_with_many(n: usize, selected: usize) -> App {
        let (_, rx) = mpsc::channel();
        let candidates: Vec<String> =
            (0..n).map(|k| format!("/repo{k}")).collect();
        let display = candidates.clone();
        App {
            candidates,
            display,
            query: String::new(),
            selected,
            states: HashMap::new(),
            rx,
            scroll: 0,
        }
    }

    fn render_to_buffer(app: &mut App, w: u16, h: u16) -> Buffer {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()
    }

    #[test]
    fn border_titles_are_lowercase_and_connected() {
        assert_eq!(git_border_title(12), "git");
        assert_eq!(git_border_title(2), "gi");
        assert_eq!(git_border_title(0), "");
        assert_eq!(path_border_title(25), "path");
        assert_eq!(path_border_title(2), "pa");
        let mut app = app_with_many(2, 0);
        let buf = render_to_buffer(&mut app, 40, 10);
        // List frame starts below the two chrome lines.
        let top = row_text(&buf, 2, 40);
        assert!(top.contains("git"), "top border shows git: {top}");
        assert!(top.contains("path"), "top border shows path: {top}");
        assert!(!top.contains("GIT") && !top.contains("PATH"));
        // Mixed frame, not a hand-coded dash set: solid `─`
        // top from `border::PLAIN` and dashed `╎` sides from
        // `border::LIGHT_DOUBLE_DASHED`.
        assert!(top.contains("─"), "solid top: {top}");
        assert!(!top.contains("╌"), "no dashed run: {top}");
        assert_eq!(buf[(0, 3)].symbol(), "╎");
        // Titles use the Gruber Lighter accent, not gray.
        assert_eq!(buf[(1, 2)].fg, TITLE_FG);
        assert_eq!(buf[(14, 2)].fg, TITLE_FG);
        // Git section is 12 wide, so the divider junction sits
        // one cell inside plus the left border. Junctions stay
        // the library `┬`/`┴` the dashed line sets reuse.
        assert_eq!(buf[(13, 2)].symbol(), "┬");
        assert_eq!(buf[(13, 9)].symbol(), "┴");
        assert_eq!(buf[(1, 2)].symbol(), "g");
        assert_eq!(buf[(14, 2)].symbol(), "p");
        // Content rows carry the same dashed divider column.
        assert_eq!(buf[(13, 3)].symbol(), "╎");
        assert_eq!(buf[(13, 4)].symbol(), "╎");
    }

    #[test]
    fn no_standalone_header_row_frees_a_row() {
        let mut app = app_with_many(5, 0);
        // Height 7 leaves list inner height 3; all three rows
        // must be candidates when no header row exists.
        let buf = render_to_buffer(&mut app, 40, 7);
        assert!(row_text(&buf, 3, 40).contains("/repo0"));
        assert!(row_text(&buf, 4, 40).contains("/repo1"));
        assert!(row_text(&buf, 5, 40).contains("/repo2"));
        for y in [3, 4, 5] {
            let row = row_text(&buf, y, 40);
            assert!(!row.contains("GIT"));
        }
    }

    #[test]
    fn shared_scroll_and_selection_across_columns() {
        let mut app = app_with_many(10, 5);
        // Same geometry as above: inner height 3, so index 5
        // scrolls to the bottom visible row with one scroll
        // offset shared by both columns.
        let buf = render_to_buffer(&mut app, 40, 7);
        assert!(row_text(&buf, 3, 40).contains("/repo3"));
        assert!(row_text(&buf, 4, 40).contains("/repo4"));
        assert!(row_text(&buf, 5, 40).contains("/repo5"));
        // Selection bg spans Git, divider, and path cells (path
        // start at the divider + 1, not trailing filler).
        assert_eq!(buf[(1, 5)].bg, SELECTED_BG);
        assert_eq!(buf[(13, 5)].bg, SELECTED_BG);
        assert_eq!(buf[(14, 5)].bg, SELECTED_BG);
        assert_ne!(buf[(1, 3)].bg, SELECTED_BG);
        assert_ne!(buf[(14, 3)].bg, SELECTED_BG);
        // Divider stays in the same column on every row.
        assert_eq!(buf[(13, 3)].symbol(), "╎");
        assert_eq!(buf[(13, 4)].symbol(), "╎");
        assert_eq!(buf[(13, 5)].symbol(), "╎");
    }

    #[test]
    fn narrow_glyph_only_clips_git_title() {
        let mut app = app_with(Some(candidate(
            Some("/r"),
            false,
            WorkState::Measurable {
                added: 3,
                deleted: 4,
            },
        )));
        // Width 24 leaves inner 22: glyph-only Git.
        let buf = render_to_buffer(&mut app, 24, 8);
        let top = row_text(&buf, 2, 24);
        assert!(top.contains("gi"), "clipped git title: {top}");
        assert!(!top.contains("git"));
        assert!(top.contains("path"));
        assert_eq!(buf[(3, 2)].symbol(), "┬");
        assert_eq!(buf[(3, 3)].symbol(), "╎");
        let row = row_text(&buf, 3, 24);
        assert!(!row.contains('+') && !row.contains('−'));
    }

    #[test]
    fn hidden_git_removes_section_and_divider() {
        let mut app = app_with_many(1, 0);
        // Width 15 leaves inner 13: Git hidden.
        let buf = render_to_buffer(&mut app, 15, 8);
        let top = row_text(&buf, 2, 15);
        assert!(top.contains("path"));
        assert!(!top.contains("git") && !top.contains("gi"));
        assert!(!top.contains("┬"));
        assert!(!row_text(&buf, 7, 15).contains("┴"));
        // No interior divider; only the outer dashed borders
        // use `╎`.
        for x in 1..14 {
            assert_ne!(buf[(x, 3)].symbol(), "╎");
            assert_ne!(buf[(x, 3)].symbol(), "│");
            assert_ne!(buf[(x, 3)].symbol(), "┬");
        }
        assert!(row_text(&buf, 3, 15).contains("/repo0"));
        let line = &app.render_entry(0, false, 13)[0];
        assert!(!line_text(line).contains("╎"));
        assert!(!line_text(line).contains("│"));
    }

    #[test]
    fn tiny_sizes_are_safe() {
        for (w, h) in [(1, 1), (2, 2), (3, 3), (10, 2), (2, 10), (5, 5)] {
            let mut app = app_with_many(3, 1);
            let _ = render_to_buffer(&mut app, w, h);
        }
        // Zero inner widths clip titles instead of panicking.
        assert_eq!(git_border_title(0), "");
        assert_eq!(path_border_title(0), "");
        assert_eq!(git_col_width(0), 0);
        // Border keeps the established reserve: inner 29 still
        // shows full Git, inner 28 drops to glyph-only.
        assert_eq!(git_col_width(29), 12);
        assert_eq!(git_col_width(28), 2);
    }
}
