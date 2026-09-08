// PROTOTYPE (throwaway, not for production).
//
// Question: "What should session candidates look like as cards?"
//
// Three close variants (Tab): outlined, outlined tinted
// selection, light separators. Identical data/geometry/behavior.
// Nothing here may be promoted to production as-is; delete this
// file once the design question is settled.

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyModifiers},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
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
/// Gruber Lighter from
/// `~/personal/gruber-darker-tmux-theme/gruber-lighter-tmux-theme.tmux`
const FG: Color = Color::Rgb(0x28, 0x28, 0x28);
const BG: Color = Color::Rgb(0xF5, 0xF5, 0xF5);
const BG_ALT: Color = Color::Rgb(0xCE, 0xC9, 0xC0);
const ACCENT: Color = Color::Rgb(0x37, 0x52, 0x6D);
const COMMENT: Color = Color::Rgb(0x45, 0x55, 0x4D);
const OPERATOR: Color = Color::Rgb(0xAA, 0xA4, 0x9C);
const RED: Color = Color::Rgb(0x98, 0x22, 0x2A);
const GREEN: Color = Color::Rgb(0x82, 0xA7, 0x62);
const YELLOW: Color = Color::Rgb(0xCC, 0x96, 0x00);
/// Target inner content width; outer card adds left+right borders.
const CONTENT_TARGET: usize = 30;
/// 2 content lines plus top/bottom borders.
const CARD_H: usize = 4;

#[derive(Deserialize)]
struct RawConfig {
    paths: Option<Vec<String>>,
    #[serde(rename = "git-from-home")]
    git_from_home: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Outlined,
    OutlinedTint,
    Separators,
}

impl Variant {
    fn next(self) -> Self {
        match self {
            Self::Outlined => Self::OutlinedTint,
            Self::OutlinedTint => Self::Separators,
            Self::Separators => Self::Outlined,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Outlined => "outlined",
            Self::OutlinedTint => "outlined+tint",
            Self::Separators => "separators",
        }
    }

    fn border_set(self) -> border::Set<'static> {
        match self {
            Self::Separators => border::LIGHT_DOUBLE_DASHED,
            _ => border::PLAIN,
        }
    }
}

struct App {
    candidates: Vec<String>,
    display: Vec<String>,
    query: String,
    /// Absolute candidate path; kept across filter/layout when
    /// the path is still in the filtered set.
    selected: Option<String>,
    variant: Variant,
    /// Latest worker snapshot per absolute candidate path.
    /// Missing entries are still loading. This map is the only
    /// thing Git ticks may change; candidates, query, and
    /// selection stay stable.
    states: HashMap<String, CandidateState>,
    rx: mpsc::Receiver<Vec<(String, CandidateState)>>,
    /// Scroll offset in whole card-rows.
    scroll: usize,
    /// Column count from the last render; used for spatial nav.
    grid_cols: usize,
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

    /// Visible index among filtered results. Identity wins when
    /// the selected path still matches; otherwise clamp.
    fn visual_pos(&self) -> Option<usize> {
        let filtered = self.filtered();
        if filtered.is_empty() {
            return None;
        }
        if let Some(path) = &self.selected {
            if let Some(pos) =
                filtered.iter().position(|&i| self.candidates[i] == *path)
            {
                return Some(pos);
            }
            return Some(filtered.len() - 1);
        }
        Some(0)
    }

    fn select_pos(&mut self, pos: usize) {
        let filtered = self.filtered();
        if let Some(&i) = filtered.get(pos) {
            self.selected = Some(self.candidates[i].clone());
        }
    }

    /// Bold icon only; counts stay NORMAL weight.
    fn icon_style(color: Color) -> Style {
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    }

    /// Git status for one card's second line. Nonrepo is blank.
    /// Measurable rows show both counts or neither. Width is the
    /// card inner width (not a table GIT column).
    fn git_spans(&self, i: usize, inner_w: usize) -> Vec<Span<'static>> {
        if inner_w == 0 {
            return vec![];
        }
        let icon_of = |linked: bool| {
            Span::styled(work_icon(linked), Self::icon_style(ACCENT))
        };
        match self.states.get(&self.candidates[i]) {
            None => {
                vec![Span::styled("…", Style::new().fg(COMMENT))]
            }
            // Inspect failure first: red regardless of whether
            // a root resolved, so a failed resolution under an
            // enclosing `.git` never reads as nonrepo.
            Some(s) if s.state == WorkState::Failed => {
                vec![Span::styled("\u{f467}", Self::icon_style(RED))]
            }
            Some(s) if s.root.is_none() => vec![],
            Some(s) => match s.state {
                WorkState::Clean => vec![icon_of(s.linked)],
                WorkState::Measurable { added, deleted } => {
                    let budget = inner_w.saturating_sub(2);
                    match measurable_texts(added, deleted, budget) {
                        Some((a, d)) => vec![
                            icon_of(s.linked),
                            Span::from(" "),
                            Span::styled(a, Style::new().fg(GREEN)),
                            Span::from(" "),
                            Span::styled(d, Style::new().fg(RED)),
                        ],
                        None => vec![icon_of(s.linked)],
                    }
                }
                WorkState::Marker => {
                    let full = vec![
                        icon_of(s.linked),
                        Span::from(" "),
                        Span::styled("•", Style::new().fg(YELLOW)),
                    ];
                    if status_width(&full) <= inner_w {
                        full
                    } else {
                        vec![icon_of(s.linked)]
                    }
                }
                WorkState::Failed => vec![icon_of(s.linked)],
            },
        }
    }

    /// Spatial move on a row-major grid. Left/Right stay in-row
    /// (no wrap). Up/Down keep the column; an empty last-row
    /// cell snaps to that row's last card. No wrap.
    fn move_sel(&mut self, dx: i32, dy: i32) {
        let n = self.filtered().len();
        if n == 0 {
            return;
        }
        let cols = self.grid_cols.max(1);
        let pos = self.visual_pos().unwrap_or(0);
        let row = pos / cols;
        let col = pos % cols;
        let total_rows = n.div_ceil(cols);
        let row_len = |r: usize| {
            if r + 1 == total_rows {
                let rem = n % cols;
                if rem == 0 { cols } else { rem }
            } else {
                cols
            }
        };
        if dx != 0 {
            let new_col = col as i32 + dx;
            let len = row_len(row);
            if new_col >= 0 && (new_col as usize) < len {
                self.select_pos(row * cols + new_col as usize);
            }
            return;
        }
        if dy != 0 {
            let new_row = row as i32 + dy;
            if new_row < 0 || (new_row as usize) >= total_rows {
                return;
            }
            let new_row = new_row as usize;
            let len = row_len(new_row);
            let new_col = col.min(len - 1);
            self.select_pos(new_row * cols + new_col);
        }
    }

    /// Handle one key press. Returns true when the UI should exit.
    /// Digits append to the query; Tab cycles variants; arrows
    /// move spatially; Enter deliberately does nothing (no tmux).
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
            }
            KeyCode::Backspace => {
                let _ = self.query.pop();
            }
            KeyCode::Tab => {
                self.variant = self.variant.next();
            }
            KeyCode::Left => self.move_sel(-1, 0),
            KeyCode::Right => self.move_sel(1, 0),
            KeyCode::Up => self.move_sel(0, -1),
            KeyCode::Down => self.move_sel(0, 1),
            KeyCode::Enter => {}
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
        let area = frame.area();
        frame.render_widget(
            Block::default().style(Style::new().bg(BG).fg(FG)),
            area,
        );
        if area.width == 0 || area.height == 0 {
            return;
        }
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![Constraint::Length(1), Constraint::Fill(1)])
            .split(area);
        self.render_filter(frame, areas[0]);
        self.render_cards(frame, areas[1]);
    }

    fn render_filter(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let label = self.variant.label();
        let lab_w = (label.chars().count() as u16).min(area.width);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Fill(1), Constraint::Length(lab_w)])
            .split(area);
        frame.render_widget(
            Line::from(vec![
                Span::styled("> ", Style::new().fg(FG).bg(BG)),
                Span::styled(self.query.as_str(), Style::new().fg(FG).bg(BG)),
                Span::styled("█", Style::new().fg(ACCENT).bg(BG)),
            ]),
            chunks[0],
        );
        if lab_w > 0 {
            frame.render_widget(
                Line::from(Span::styled(
                    label,
                    Style::new().fg(COMMENT).bg(BG),
                )),
                chunks[1],
            );
        }
    }

    fn render_cards(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let (cols, card_w) = card_geometry(area.width as usize);
        self.grid_cols = cols;
        let filtered = self.filtered();
        let n = filtered.len();
        let total_rows = if n == 0 { 0 } else { n.div_ceil(cols) };
        let rows_visible = (area.height as usize) / CARD_H;
        let sel_row = self.visual_pos().map(|p| p / cols).unwrap_or(0);
        if rows_visible == 0 {
            self.scroll = self.scroll.min(total_rows);
            return;
        }
        if sel_row < self.scroll {
            self.scroll = sel_row;
        } else if sel_row >= self.scroll + rows_visible {
            self.scroll = sel_row + 1 - rows_visible;
        }
        let max_scroll = total_rows.saturating_sub(rows_visible);
        self.scroll = self.scroll.min(max_scroll);
        let vis = self.visual_pos();
        for r in 0..rows_visible {
            let row_idx = self.scroll + r;
            if row_idx >= total_rows {
                break;
            }
            for c in 0..cols {
                let pos = row_idx * cols + c;
                if pos >= n {
                    break;
                }
                let rect = Rect {
                    x: area.x.saturating_add((c * card_w) as u16),
                    y: area.y.saturating_add((r * CARD_H) as u16),
                    width: card_w as u16,
                    height: CARD_H as u16,
                }
                .intersection(area);
                self.render_card(frame, rect, filtered[pos], vis == Some(pos));
            }
        }
    }

    fn render_card(
        &self,
        frame: &mut Frame,
        rect: Rect,
        cand_idx: usize,
        selected: bool,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let fill = if selected && self.variant == Variant::OutlinedTint {
            BG_ALT
        } else {
            BG
        };
        let border_style = if selected {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(OPERATOR)
        };
        let block = Block::bordered()
            .border_set(self.variant.border_set())
            .border_style(border_style)
            .style(Style::new().bg(fill).fg(FG));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let inner_w = inner.width as usize;
        let path = truncate_path(&self.display[cand_idx], inner_w);
        let mut path_spans =
            vec![Span::styled(path, Style::new().fg(FG).bg(fill))];
        let mut git_spans = self.git_spans(cand_idx, inner_w);
        for span in &mut git_spans {
            span.style = span.style.bg(fill);
        }
        for span in &mut path_spans {
            span.style = span.style.bg(fill);
        }
        let mut lines = vec![Line::from(path_spans)];
        if inner.height >= 2 {
            lines.push(Line::from(git_spans));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::new().bg(fill).fg(FG)),
            inner,
        );
    }
}

/// 1–3 columns of target-width cards; leftover unused once 3
/// fit. Shrink only when even one target-width card cannot fit.
fn card_geometry(available: usize) -> (usize, usize) {
    let full = CONTENT_TARGET + 2;
    if available >= full * 3 {
        (3, full)
    } else if available >= full * 2 {
        (2, full)
    } else if available >= full {
        (1, full)
    } else if available == 0 {
        (1, 0)
    } else {
        (1, available)
    }
}

/// Worktree icon: ordinary checkout vs linked worktree.
fn work_icon(linked: bool) -> &'static str {
    if linked { "\u{ec7d}" } else { "\u{ec6f}" }
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
    eprintln!("session_cards_ui_prototype: {message}");
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
    let display: Vec<String> =
        candidates.iter().map(|p| display_path(p, &home)).collect();
    let (tx, rx) = mpsc::channel();
    let worker_candidates = candidates.clone();
    thread::spawn(move || poll_worker(worker_candidates, tx));
    let selected = candidates.first().cloned();
    let mut app = App {
        candidates,
        display,
        query: String::new(),
        selected,
        variant: Variant::Outlined,
        states: HashMap::new(),
        rx,
        scroll: 0,
        grid_cols: 1,
    };
    if let Err(e) =
        ratatui::run(|terminal: &mut DefaultTerminal| app.run(terminal))
    {
        fail(&format!("terminal error: {e}"));
    }
}
