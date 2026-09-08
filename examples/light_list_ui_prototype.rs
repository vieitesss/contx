// PROTOTYPE (throwaway, not for production).
//
// Question: "what should the session list look like as a
// full-width two-line path+git list?"
//
// Run: `cargo run --example light_list_ui_prototype`.
// In-memory fixtures only: no config load, no git poll
// thread, no tmux, no filesystem writes. Delete this file
// once the design question is settled.

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::{io, path::Path, time::Duration};

/// Local light palette for this prototype only.
/// Warm-white bg, dark text, restrained blue accent, muted
/// secondary; green/red/gold match the existing Git counts.
const BG: Color = Color::Rgb(0xF5, 0xF5, 0xF5);
const FG: Color = Color::Rgb(0x28, 0x28, 0x28);
const BG_ALT: Color = Color::Rgb(0xEA, 0xE7, 0xE1);
const BLUE: Color = Color::Rgb(0x37, 0x52, 0x6D);
const MUTED: Color = Color::Rgb(0xAA, 0xA4, 0x9C);
const COMMENT: Color = Color::Rgb(0x45, 0x55, 0x4D);
const GREEN: Color = Color::Rgb(0x82, 0xA7, 0x62);
const RED: Color = Color::Rgb(0x98, 0x22, 0x2A);
const GOLD: Color = Color::Rgb(0xCC, 0x96, 0x00);

/// FIXTURE Git states for the look-demo only. Shapes mirror
/// `src/tui/git.rs` (`Clean` / `Measurable` / `Marker` /
/// nonrepo-blank / loading / failed) but are hardcoded here:
/// never computed from a real repo, no git subprocesses.
#[derive(Clone, Copy)]
enum FixtureGit {
    /// Still resolving: muted `…` on the Git line.
    Loading,
    /// No enclosing worktree: Git line stays blank.
    Nonrepo,
    /// Inspect failure: red `` glyph.
    Failed,
    Clean {
        linked: bool,
    },
    Measurable {
        linked: bool,
        added: u64,
        deleted: u64,
    },
    /// Dirty without measurable lines: gold `•`.
    Marker {
        linked: bool,
    },
}

/// One hardcoded look-demo row: absolute path plus its
/// FIXTURE Git state. No branch, tmux, or agent data.
struct Fixture {
    path: &'static str,
    git: FixtureGit,
}

/// FIXTURE sample: every row is hardcoded for the demo.
/// Covers long paths (truncation), ordinary vs linked
/// worktree icons, clean, small + large measurable counts,
/// marker, nonrepo-blank, loading, and failed states.
fn fixtures() -> Vec<Fixture> {
    vec![
        // FIXTURE: short clean ordinary checkout.
        Fixture {
            path: "/home/me/personal/contx",
            git: FixtureGit::Clean { linked: false },
        },
        // FIXTURE: clean linked worktree (second icon).
        Fixture {
            path: "/home/me/work/prefapp/skills",
            git: FixtureGit::Clean { linked: true },
        },
        // FIXTURE: small measurable pair `+3 −4`.
        Fixture {
            path: "/home/me/projects/alpha",
            git: FixtureGit::Measurable {
                linked: false,
                added: 3,
                deleted: 4,
            },
        },
        // FIXTURE: large counts needing k/m compaction.
        Fixture {
            path: "/home/me/projects/monorepo/packages/frontend",
            git: FixtureGit::Measurable {
                linked: false,
                added: 12_400,
                deleted: 300,
            },
        },
        // FIXTURE: long path forcing parent shortening.
        Fixture {
            path: "/home/me/personal/very-long-parent/projects/contx",
            git: FixtureGit::Measurable {
                linked: true,
                added: 7,
                deleted: 1,
            },
        },
        // FIXTURE: dirty without lines (gold `•`).
        Fixture {
            path: "/home/me/notes",
            git: FixtureGit::Marker { linked: false },
        },
        // FIXTURE: outside any worktree (blank Git line).
        Fixture {
            path: "/tmp/scratch",
            git: FixtureGit::Nonrepo,
        },
        // FIXTURE: still resolving (muted `…`).
        Fixture {
            path: "/home/me/personal/new-clone",
            git: FixtureGit::Loading,
        },
        // FIXTURE: inspect failure (red ``).
        Fixture {
            path: "/home/me/personal/broken-link",
            git: FixtureGit::Failed,
        },
    ]
}

/// Consistent leading indent width on the path line (no
/// selection marker arrow). The Git line pads the same width
/// plus a two-space subordinate indent under the path.
const SEL_W: usize = 2;
const GIT_INDENT: usize = SEL_W + 2;
/// Filter row on top; no frame around the bare two-line
/// items. A one-line FIXTURE status closes the frame.
const FILTER_ROWS: usize = 1;
const STATUS_ROWS: usize = 1;
/// Two terminal rows per item: path first, Git underneath.
const ITEM_ROWS: usize = 2;

const POLL_TICK: Duration = Duration::from_millis(100);

fn main() -> io::Result<()> {
    ratatui::run(|terminal: &mut DefaultTerminal| run(terminal))
}

/// Display-only home abbreviation: exact `$HOME` becomes
/// `~`, descendants become `~/rest`, anything else stays.
fn abbreviate_home(path: &str, home: Option<&str>) -> String {
    let home = match home {
        Some(h) if !h.is_empty() => h,
        _ => return path.to_string(),
    };
    match Path::new(path).strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.to_string(),
    }
}

/// Path-aware shortening to at most `max` chars: full string
/// when it fits; else leading prefix + `…` + full basename
/// (parents shorten first, basename never clips); else `…`
/// + tail. The leading indent is never clipped.
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

/// Char indices in `text` matching `query` as a case-sensitive
/// subsequence, in order; `None` when it does not match. An
/// empty query matches with no highlights. Local copy: the
/// real `crate::fuzzy` is private to the binary target.
fn subseq_match(text: &str, query: &str) -> Option<Vec<usize>> {
    let mut hit = vec![];
    let mut chars = text.chars().enumerate();
    for q in query.chars() {
        match chars.find(|(_, c)| *c == q) {
            Some((i, _)) => hit.push(i),
            None => return None,
        }
    }
    Some(hit)
}

/// Path text with fuzzy matches painted in the blue accent
/// (HEAD `format_line` treatment). Matching runs on the
/// truncated visible path, so highlights never break
/// truncation; an empty query renders plain dark text.
fn path_spans(path: &str, query: &str, bg: Color) -> Vec<Span<'static>> {
    let plain = Style::new().fg(FG).bg(bg);
    let hit_style = Style::new().fg(BLUE).bg(bg);
    let hit = match subseq_match(path, query) {
        Some(h) if !query.is_empty() => h,
        _ => return vec![Span::styled(path.to_string(), plain)],
    };
    let mut hits = hit.iter().peekable();
    let mut out = vec![];
    let mut buf = String::new();
    let mut in_hit = false;
    for (i, c) in path.chars().enumerate() {
        let is_hit = hits.peek().is_some_and(|&&h| h == i);
        if is_hit {
            hits.next();
        }
        if buf.is_empty() {
            in_hit = is_hit;
        } else if is_hit != in_hit {
            let style = if in_hit { hit_style } else { plain };
            out.push(Span::styled(std::mem::take(&mut buf), style));
            in_hit = is_hit;
        }
        buf.push(c);
    }
    if !buf.is_empty() {
        let style = if in_hit { hit_style } else { plain };
        out.push(Span::styled(buf, style));
    }
    out
}

/// Worktree icon: ordinary checkout vs linked worktree.
/// Same glyphs as `src/tui/sessions_list.rs`.
fn work_icon(linked: bool) -> &'static str {
    if linked { "\u{ec7d}" } else { "\u{ec6f}" }
}

fn icon_style(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

/// One side of a measurable pair at unit `level`: 0 exact,
/// 1 `k`, 2 `m`, 3 `b`. Integer division only; `None` below
/// the unit's threshold, so `0k`/`0m`/`0b` never render.
/// Same rule as `src/tui/git.rs`.
fn fmt_count(n: u64, level: u8) -> Option<String> {
    match level {
        0 => Some(format!("{n}")),
        1 if n >= 1_000 => Some(format!("{}k", n / 1_000)),
        2 if n >= 1_000_000 => Some(format!("{}m", n / 1_000_000)),
        3 if n >= 1_000_000_000 => Some(format!("{}b", n / 1_000_000_000)),
        _ => None,
    }
}

/// Paired `(+A, −D)` texts whose `+A −D` width fits
/// `budget`, least-coarse unit levels first. Both counts or
/// neither: `None` means glyph only, never plus-only.
/// Same rule as `src/tui/git.rs`.
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

/// Visible width of styled fragments, in characters.
fn status_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// Git metadata spans for one item's second line, within
/// `budget` content cells after the indent. Nonrepo is blank.
/// The Git line takes the full remaining width, so path
/// parents on line 1 shorten before these counts compact:
/// measurable rows show both counts or neither (glyph only
/// when no pair fits, never plus-only); the marker `•`
/// drops the same way when even `icon •` is too wide.
fn git_spans(git: FixtureGit, budget: usize) -> Vec<Span<'static>> {
    let icon_of =
        |linked: bool| Span::styled(work_icon(linked), icon_style(BLUE));
    match git {
        FixtureGit::Loading => {
            vec![Span::styled("…", Style::new().fg(COMMENT))]
        }
        FixtureGit::Nonrepo => vec![],
        FixtureGit::Failed => {
            vec![Span::styled("\u{f467}", icon_style(RED))]
        }
        FixtureGit::Clean { linked } => vec![icon_of(linked)],
        FixtureGit::Measurable {
            linked,
            added,
            deleted,
        } => {
            let counts = budget.saturating_sub(2);
            match measurable_texts(added, deleted, counts) {
                Some((a, d)) => vec![
                    icon_of(linked),
                    Span::from(" "),
                    Span::styled(a, Style::new().fg(GREEN)),
                    Span::from(" "),
                    Span::styled(d, Style::new().fg(RED)),
                ],
                None => vec![icon_of(linked)],
            }
        }
        FixtureGit::Marker { linked } => {
            let full = vec![
                icon_of(linked),
                Span::from(" "),
                Span::styled("•", Style::new().fg(GOLD)),
            ];
            if status_width(&full) <= budget {
                full
            } else {
                vec![icon_of(linked)]
            }
        }
    }
}

/// Demo sample mode, cycled with Tab: the full FIXTURE
/// list, an empty candidate set, or a forced filter-miss so
/// empty vs no-match are both visible without typing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sample {
    Populated,
    Empty,
    NoMatch,
}

impl Sample {
    fn next(self) -> Self {
        match self {
            Self::Populated => Self::Empty,
            Self::Empty => Self::NoMatch,
            Self::NoMatch => Self::Populated,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Populated => "populated",
            Self::Empty => "empty",
            Self::NoMatch => "no-match",
        }
    }
}

/// In-memory demo state: FIXTURE rows, sample mode, linear
/// selection into the shown rows, scroll offset, `$HOME`
/// for display, and the filter query. No persistence, no I/O.
struct App {
    fixtures: Vec<Fixture>,
    sample: Sample,
    selected: usize,
    scroll: usize,
    home: Option<String>,
    query: String,
}

impl App {
    fn new() -> Self {
        Self {
            fixtures: fixtures(),
            sample: Sample::Populated,
            selected: 0,
            scroll: 0,
            home: std::env::var("HOME").ok(),
            query: String::new(),
        }
    }

    /// Display path used for filtering (untruncated; the
    /// highlight runs on the truncated visible path).
    fn display(&self, i: usize) -> String {
        abbreviate_home(self.fixtures[i].path, self.home.as_deref())
    }

    /// Shown fixture indices: query subsequence matches in
    /// `Populated`, nothing in the other samples.
    fn shown(&self) -> Vec<usize> {
        match self.sample {
            Sample::Empty | Sample::NoMatch => vec![],
            Sample::Populated => (0..self.fixtures.len())
                .filter(|&i| {
                    subseq_match(&self.display(i), &self.query).is_some()
                })
                .collect(),
        }
    }

    /// Clamp selection after the shown set shrinks.
    fn clamp_sel(&mut self) {
        let n = self.shown().len();
        if n == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(n - 1);
        }
    }

    /// Linear move with no wrap; no-op when empty.
    fn move_sel(&mut self, dy: i32) {
        let n = self.shown().len();
        if n == 0 {
            return;
        }
        let next = self.selected as i32 + dy;
        self.selected = next.clamp(0, n as i32 - 1) as usize;
    }

    /// Keep the selected row inside the visible window.
    fn update_scroll(&mut self, visible: usize, total: usize) {
        if visible == 0 {
            self.scroll = self.scroll.min(total);
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected + 1 - visible;
        }
        let max = total.saturating_sub(visible);
        self.scroll = self.scroll.min(max);
    }

    /// One key press; true quits. Printable keys filter,
    /// Up/Down (Ctrl-j/k, Home/End) move, Enter is a no-op
    /// (no tmux in a prototype), Tab cycles the sample.
    /// Only in-memory UI state changes: no tmux, no writes.
    fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => return true,
                KeyCode::Char('j') => self.move_sel(1),
                KeyCode::Char('k') => self.move_sel(-1),
                _ => {}
            }
            return false;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        match key.code {
            KeyCode::Esc => return true,
            KeyCode::Enter => {}
            KeyCode::Tab => {
                self.sample = self.sample.next();
                self.selected = 0;
                self.scroll = 0;
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.clamp_sel();
            }
            KeyCode::Up => self.move_sel(-1),
            KeyCode::Down => self.move_sel(1),
            KeyCode::Home => {
                if !self.shown().is_empty() {
                    self.selected = 0;
                }
            }
            KeyCode::End => {
                let n = self.shown().len();
                if n > 0 {
                    self.selected = n - 1;
                }
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.clamp_sel();
            }
            _ => {}
        }
        false
    }
}

/// One full-width two-line item: path first with consistent
/// indent (no selection marker arrow), Git metadata indented
/// underneath. Selected items get the `bg_alt` tint on both
/// lines.
fn item_lines(
    fixture: &Fixture,
    selected: bool,
    width: usize,
    home: Option<&str>,
    query: &str,
) -> [Line<'static>; ITEM_ROWS] {
    let bg = if selected { BG_ALT } else { BG };
    let line_style = Style::new().bg(bg).fg(FG);
    let display = abbreviate_home(fixture.path, home);
    let path_w = width.saturating_sub(SEL_W).max(1);
    let path = truncate_path(&display, path_w);
    let mut path_row =
        vec![Span::styled(" ".repeat(SEL_W), Style::new().bg(bg))];
    path_row.extend(path_spans(&path, query, bg));
    // Pad to the full width so the selection tint covers the
    // whole row instead of ending after the last glyph.
    let pad = width.saturating_sub(SEL_W + path.chars().count());
    if pad > 0 {
        path_row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    let path_line = Line::from(path_row).style(line_style);
    let mut git: Vec<Span<'static>> =
        vec![Span::styled(" ".repeat(GIT_INDENT), Style::new().bg(bg))];
    let git_budget = width.saturating_sub(GIT_INDENT);
    let content = git_spans(fixture.git, git_budget);
    let git_pad = width.saturating_sub(GIT_INDENT + status_width(&content));
    for mut span in content {
        span.style = span.style.bg(bg);
        git.push(span);
    }
    if git_pad > 0 {
        git.push(Span::styled(" ".repeat(git_pad), Style::new().bg(bg)));
    }
    let git_line = Line::from(git).style(line_style);
    [path_line, git_line]
}

/// Full-frame render: warm-white bg, filter row, the bare
/// two-line items, and a one-line FIXTURE status. No boxes, no vertical borders,
/// no list frame. Empty and no-match samples render muted
/// copy in the list area; loading/error stay FIXTURE Git
/// rows inside `populated`.
fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();
    frame
        .buffer_mut()
        .set_style(area, Style::new().bg(BG).fg(FG));
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled("Search: ", Style::new().fg(MUTED).bg(BG)),
        Span::styled(app.query.clone(), Style::new().fg(FG).bg(BG)),
        Span::styled("█", Style::new().fg(BLUE).bg(BG)),
    ])];
    let list_h =
        (area.height as usize).saturating_sub(FILTER_ROWS + STATUS_ROWS);
    let visible = list_h / ITEM_ROWS;
    let shown = app.shown();
    app.update_scroll(visible, shown.len());
    let query = app.query.clone();
    if shown.is_empty() {
        let copy = if app.sample == Sample::Empty {
            "no session candidates — FIXTURE empty sample".to_string()
        } else {
            format!(
                "no matches for \"{}\" — FIXTURE no-match sample",
                app.query
            )
        };
        lines.push(Line::from(Span::styled(
            format!("{}{copy}", " ".repeat(GIT_INDENT)),
            Style::new().fg(COMMENT).bg(BG),
        )));
    }
    for (pos, i) in shown.iter().enumerate().skip(app.scroll).take(visible) {
        lines.extend(item_lines(
            &app.fixtures[*i],
            pos == app.selected,
            width,
            app.home.as_deref(),
            &query,
        ));
    }
    let sel = if shown.is_empty() {
        "—".to_string()
    } else {
        format!("{}", app.selected + 1)
    };
    lines.push(Line::from(Span::styled(
        format!(
            "FIXTURE {} · {}/{} shown · sel {} · query \"{}\"",
            app.sample.label(),
            shown.len(),
            app.fixtures.len(),
            sel,
            app.query,
        ),
        Style::new().fg(COMMENT).bg(BG),
    )));
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(BG).fg(FG)),
        area,
    );
}

/// Demo loop: filter, navigate, and cycle samples with the
/// keyboard; Esc / Ctrl-C quits. Everything stays in
/// memory: no tmux, no git, no writes.
fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    loop {
        terminal.draw(|frame| render(&mut app, frame))?;
        if event::poll(POLL_TICK)?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key)
        {
            return Ok(());
        }
    }
}
