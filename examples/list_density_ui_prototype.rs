// PROTOTYPE (throwaway, not for production).
//
// Question: "How should path vs git (especially the branch) be
// laid out so they are easier to tell apart?" Production packs
// two flush rows per item (path, then indented git with
// icon+branch+dirty+arrows), so path and branch sit on
// consecutive lines in similar weight.
//
// Three structurally different variants, switchable live:
//   A — Item gap: current two-line item (path then git) with one
//       empty row BETWEEN items. Selection tint covers only the
//       two content rows. Tests "sessions glue together".
//   B — Inner gap: each item is path, one empty row, git, with
//       the tint covering all three rows. Tests "path sits on
//       the branch".
//   C — Identity first: line 1 is the git identity (worktree
//       icon + branch/SHA + dirty/upstream), line 2 is a muted
//       comment-colored path, more indented. No extra blank.
//       Tests flipping which line is primary.
//
// Run: `cargo run --example list_density_ui_prototype`.
// Keys: 1/2/3 switch variant, Tab cycles, Up/Down select,
// printable keys filter, q/Esc quit. (Digits 1/2/3 and `q` are
// reserved for switching/quitting, so they cannot be typed into
// the filter; fixtures avoid `q` and 1/2/3 in filterable text.)
// In-memory fixtures only: no config load, no git poll thread,
// no tmux, no filesystem writes. Delete this file once the
// design question is settled.

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::{io, path::Path, time::Duration};

/// Local light palette for this prototype only, from
/// production `Theme::LIGHT`. Hits and the dirty marker read
/// yellow (accent); branch text and the worktree icon share
/// git_icon, icon bold, name not bold.
const BG: Color = Color::Rgb(0xF5, 0xF5, 0xF5);
const FG: Color = Color::Rgb(0x28, 0x28, 0x28);
const BG_ALT: Color = Color::Rgb(0xEA, 0xE7, 0xE1);
const GIT_ICON: Color = Color::Rgb(0x37, 0x52, 0x6D);
const ACCENT: Color = Color::Rgb(0xCC, 0x96, 0x00);
const COMMENT: Color = Color::Rgb(0x45, 0x55, 0x4D);
const GREEN: Color = Color::Rgb(0x82, 0xA7, 0x62);
const RED: Color = Color::Rgb(0x98, 0x22, 0x2A);
const MUTED: Color = Color::Rgb(0xAA, 0xA4, 0x9C);

/// FIXTURE checked-out identity: a named branch, a detached
/// short SHA, or nothing. Shapes mirror `src/tui/git.rs`
/// `Head` but are hardcoded strings here, never read from git.
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum Head {
    Named(&'static str),
    Detached(&'static str),
    Absent,
}

/// FIXTURE local dirt: clean, a measurable (+added −deleted)
/// pair, or a marker without line counts.
#[derive(Clone, Copy)]
enum Dirty {
    Clean,
    Pair { added: u64, deleted: u64 },
    Marker,
}

/// FIXTURE git state for the look-demo only. `upstream` is
/// `Some((ahead, behind))` when configured (zeros render no
/// arrows); `None` means absent. Never computed from a real
/// repo, no git subprocesses.
#[derive(Clone, Copy)]
enum FixtureGit {
    /// Still resolving: muted `…` on the git line.
    Loading,
    /// No enclosing worktree: git line stays blank.
    Nonrepo,
    /// Inspect failure: red `` glyph.
    Failed,
    Present {
        linked: bool,
        head: Head,
        dirty: Dirty,
        upstream: Option<(u64, u64)>,
    },
}

/// One hardcoded look-demo row: absolute path plus its
/// FIXTURE git state. No tmux or agent data.
struct Fixture {
    path: &'static str,
    git: FixtureGit,
}

/// FIXTURE sample: every row is hardcoded for the demo.
/// Covers named branches (`main`, `feat/long-branch-name`,
/// `topic`), one detached short SHA, ordinary vs linked
/// worktree icons, a dirty pair, a marker, upstream arrows
/// (behind-only, ahead-only, diverged), a long path with a
/// long branch for maximum crowding, plus loading, failed,
/// and nonrepo rows.
fn fixtures() -> Vec<Fixture> {
    use Dirty::{Clean, Marker, Pair};
    use Head::{Detached, Named};
    vec![
        // FIXTURE: short clean ordinary checkout on main.
        Fixture {
            path: "/home/me/personal/contx",
            git: FixtureGit::Present {
                linked: false,
                head: Named("main"),
                dirty: Clean,
                upstream: None,
            },
        },
        // FIXTURE: clean linked worktree with a long branch.
        Fixture {
            path: "/home/me/work/prefapp/skills",
            git: FixtureGit::Present {
                linked: true,
                head: Named("feat/long-branch-name"),
                dirty: Clean,
                upstream: None,
            },
        },
        // FIXTURE: dirty pair crowded with diverged arrows.
        Fixture {
            path: "/home/me/projects/alpha",
            git: FixtureGit::Present {
                linked: false,
                head: Named("topic"),
                dirty: Pair {
                    added: 3,
                    deleted: 4,
                },
                upstream: Some((2, 1)),
            },
        },
        // FIXTURE: large counts needing k-compaction plus an
        // ahead arrow.
        Fixture {
            path: "/home/me/projects/monorepo/packages/frontend",
            git: FixtureGit::Present {
                linked: false,
                head: Named("main"),
                dirty: Pair {
                    added: 12_400,
                    deleted: 300,
                },
                upstream: Some((1, 0)),
            },
        },
        // FIXTURE: long path plus a long branch plus a dirty
        // pair plus a behind arrow: the crowding stress case.
        Fixture {
            path: "/home/me/personal/very-long-parent/projects/contx",
            git: FixtureGit::Present {
                linked: true,
                head: Named("feat/very-long-branch-name-that-crowds"),
                dirty: Pair {
                    added: 7,
                    deleted: 1,
                },
                upstream: Some((0, 3)),
            },
        },
        // FIXTURE: marker (gold `•`) with a behind arrow.
        Fixture {
            path: "/home/me/notes",
            git: FixtureGit::Present {
                linked: false,
                head: Named("topic"),
                dirty: Marker,
                upstream: Some((0, 2)),
            },
        },
        // FIXTURE: detached short SHA with a dirty pair and
        // no upstream (detached never tracks one).
        Fixture {
            path: "/home/me/personal/detached-checkout",
            git: FixtureGit::Present {
                linked: false,
                head: Detached("a9b4c8d"),
                dirty: Pair {
                    added: 5,
                    deleted: 2,
                },
                upstream: None,
            },
        },
        // FIXTURE: outside any worktree (blank git line).
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

/// Layout variant under test. Structurally different
/// information hierarchy per variant, not color tweaks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    A,
    B,
    C,
}

impl Variant {
    fn next(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::C,
            Self::C => Self::A,
        }
    }

    fn key(self) -> char {
        match self {
            Self::A => '1',
            Self::B => '2',
            Self::C => '3',
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::A => "Item gap",
            Self::B => "Inner gap",
            Self::C => "Identity first",
        }
    }

    /// One-line description surfaced in the status line.
    fn describe(self) -> &'static str {
        match self {
            Self::A => {
                "two-line items, blank row between; do sessions glue together?"
            }
            Self::B => "blank row inside the tint, path sits on the branch?",
            Self::C => {
                "branch line primary, muted path below; flip the hierarchy?"
            }
        }
    }
}

/// Path-line indent (variants A/B) and git-line subordinate
/// indent, matching production `PATH_INDENT`/`GIT_INDENT`.
/// Variant C flips the hierarchy: the identity line sits at
/// the outer indent while the path hangs further inside.
const SEL_W: usize = 2;
const GIT_INDENT: usize = SEL_W + 2;
const C_PATH_INDENT: usize = GIT_INDENT + 2;
/// Filter row on top; two status rows at the bottom (variant
/// name + description, then counts + key hints).
const FILTER_ROWS: usize = 1;
const STATUS_ROWS: usize = 2;

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
/// empty query matches with no highlights.
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

/// Path text with subsequence matches painted in the accent
/// (production `format_line` treatment: hits yellow). Runs on
/// the truncated visible path so highlights never break
/// truncation; an empty query renders plain text.
fn path_spans(
    path: &str,
    query: &str,
    bg: Color,
    plain_fg: Color,
) -> Vec<Span<'static>> {
    let plain = Style::new().fg(plain_fg).bg(bg);
    let hit_style = Style::new().fg(ACCENT).bg(bg);
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
/// Same glyphs as production.
fn work_icon(linked: bool) -> &'static str {
    if linked { "\u{ec7d}" } else { "\u{ec6f}" }
}

fn icon_style(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

/// One side of a measurable pair at unit `level`: 0 exact,
/// 1 `k`, 2 `m`, 3 `b`. Integer division only; `None` below
/// the unit's threshold, so `0k` never renders. Same rule as
/// production `src/tui/git.rs`.
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
/// neither: `None` means glyph only, never plus-only. Same
/// rule as production.
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

/// Checked-out name for the identity slot: the exact branch
/// name, or the short SHA when detached. `Absent` renders
/// nothing, never `HEAD`.
fn head_name(head: Head) -> Option<&'static str> {
    match head {
        Head::Named(n) => Some(n),
        Head::Detached(s) => Some(s),
        Head::Absent => None,
    }
}

/// Icon plus the checked-out name fitted to `inner_w`: the
/// full name when it fits beside the icon, prefix-kept `…`
/// truncation when only that fits, bare icon otherwise. Same
/// rule as production: never truncates to keep later tokens.
fn icon_with_head(
    linked: bool,
    name: Option<&'static str>,
    inner_w: usize,
) -> Vec<Span<'static>> {
    let icon = Span::styled(work_icon(linked), icon_style(GIT_ICON));
    let Some(n) = name else {
        return vec![icon];
    };
    if 2 + n.chars().count() <= inner_w {
        return vec![
            icon,
            Span::from(" "),
            Span::styled(n.to_string(), Style::new().fg(GIT_ICON)),
        ];
    }
    let room = inner_w.saturating_sub(1);
    if room >= 3 {
        let kept: String = n.chars().take(room - 2).collect();
        return vec![
            icon,
            Span::from(" "),
            Span::styled(format!("{kept}…"), Style::new().fg(GIT_ICON)),
        ];
    }
    vec![icon]
}

/// Append nonzero cached `↓behind ↑ahead` after full local git
/// content when the whole line still fits `inner_w`. Behind is
/// red like deletions, ahead green like additions, both normal
/// weight. Once a dirty pair or marker drops, upstream stays
/// dropped too. `↓` is U+2193, `↑` is U+2191.
fn append_upstream(
    mut local: Vec<Span<'static>>,
    upstream: Option<(u64, u64)>,
    inner_w: usize,
) -> Vec<Span<'static>> {
    let Some((ahead, behind)) = upstream else {
        return local;
    };
    let mut tokens: Vec<(&str, u64, Color)> = vec![];
    if behind > 0 {
        tokens.push(("↓", behind, RED));
    }
    if ahead > 0 {
        tokens.push(("↑", ahead, GREEN));
    }
    if tokens.is_empty() {
        return local;
    }
    let extra: usize = tokens
        .iter()
        .map(|(g, n, _)| g.chars().count() + n.to_string().len() + 1)
        .sum();
    if status_width(&local) + extra > inner_w {
        return local;
    }
    for (glyph, count, color) in tokens {
        local.push(Span::from(" "));
        local.push(Span::styled(
            format!("{glyph}{count}"),
            Style::new().fg(color),
        ));
    }
    local
}

/// Git identity spans for one item's git line, within
/// `budget` content cells after the indent. Nonrepo is blank.
/// The icon pairs with the checked-out name, then the local
/// dirty pair or marker, then nonzero upstream arrows. As
/// width shrinks, upstream drops first, then the dirty
/// pair/marker, then the name truncates against the icon.
fn git_spans(git: FixtureGit, budget: usize) -> Vec<Span<'static>> {
    let FixtureGit::Present {
        linked,
        head,
        dirty,
        upstream,
    } = git
    else {
        return match git {
            FixtureGit::Loading => {
                vec![Span::styled("…", Style::new().fg(COMMENT))]
            }
            FixtureGit::Nonrepo => vec![],
            FixtureGit::Failed => {
                vec![Span::styled("\u{f467}", icon_style(RED))]
            }
            FixtureGit::Present { .. } => unreachable!(),
        };
    };
    let name = head_name(head);
    let head_extra = name.map(|n| 1 + n.chars().count()).unwrap_or(0);
    match dirty {
        Dirty::Clean => append_upstream(
            icon_with_head(linked, name, budget),
            upstream,
            budget,
        ),
        Dirty::Pair { added, deleted } => {
            let base = icon_with_head(linked, name, budget);
            if 1 + head_extra > budget {
                base
            } else {
                let mut full = base;
                let counts = budget.saturating_sub(2 + head_extra);
                match measurable_texts(added, deleted, counts) {
                    Some((a, d)) => {
                        full.push(Span::from(" "));
                        full.push(Span::styled(a, Style::new().fg(GREEN)));
                        full.push(Span::from(" "));
                        full.push(Span::styled(d, Style::new().fg(RED)));
                        append_upstream(full, upstream, budget)
                    }
                    None => full,
                }
            }
        }
        Dirty::Marker => {
            let base = icon_with_head(linked, name, budget);
            if 1 + head_extra > budget {
                base
            } else {
                let mut full = base;
                if status_width(&full) + 2 <= budget {
                    full.push(Span::from(" "));
                    full.push(Span::styled("•", Style::new().fg(ACCENT)));
                    append_upstream(full, upstream, budget)
                } else {
                    full
                }
            }
        }
    }
}

/// In-memory demo state: FIXTURE rows, layout variant, linear
/// selection into the shown rows, row scroll offset, `$HOME`
/// for display, and the filter query. No persistence, no I/O.
struct App {
    fixtures: Vec<Fixture>,
    variant: Variant,
    selected: usize,
    scroll: usize,
    home: Option<String>,
    query: String,
}

impl App {
    fn new() -> Self {
        Self {
            fixtures: fixtures(),
            variant: Variant::A,
            selected: 0,
            scroll: 0,
            home: std::env::var("HOME").ok(),
            query: String::new(),
        }
    }

    /// Display path used for filtering (untruncated).
    fn display(&self, i: usize) -> String {
        abbreviate_home(self.fixtures[i].path, self.home.as_deref())
    }

    /// Filterable branch/SHA text for one fixture, if any.
    fn branch_text(i: &Fixture) -> Option<&'static str> {
        match i.git {
            FixtureGit::Present { head, .. } => head_name(head),
            _ => None,
        }
    }

    /// Shown fixture indices: query subsequence matches
    /// against the display path or the branch/SHA.
    fn shown(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.fixtures.len()).collect();
        }
        (0..self.fixtures.len())
            .filter(|&i| {
                subseq_match(&self.display(i), &self.query).is_some()
                    || Self::branch_text(&self.fixtures[i])
                        .is_some_and(|b| subseq_match(b, &self.query).is_some())
            })
            .collect()
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

    /// Keep the whole selected item block inside the visible
    /// window. `sel_start..sel_end` is the flat-row range of
    /// the selected item (empty when the list is empty).
    fn update_scroll_rows(
        &mut self,
        sel_start: usize,
        sel_end: usize,
        visible: usize,
        total: usize,
    ) {
        if visible == 0 {
            self.scroll = self.scroll.min(total);
            return;
        }
        if sel_end == sel_start {
            self.scroll = 0;
            return;
        }
        if sel_start < self.scroll {
            self.scroll = sel_start;
        } else if sel_end > self.scroll + visible {
            self.scroll = sel_end.saturating_sub(visible);
        }
        let max = total.saturating_sub(visible);
        self.scroll = self.scroll.min(max);
    }

    /// One key press; true quits. 1/2/3 switch variants, Tab
    /// cycles, Up/Down (Ctrl-j/k, Home/End) move, printable
    /// keys filter, Enter is a no-op, q/Esc (Ctrl-C) quit.
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
                self.variant = self.variant.next();
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
            // Variant switcher: digits are reserved, so they
            // never reach the filter query.
            KeyCode::Char('1') => self.variant = Variant::A,
            KeyCode::Char('2') => self.variant = Variant::B,
            KeyCode::Char('3') => self.variant = Variant::C,
            // Quit reserves `q`, like the switcher digits.
            KeyCode::Char('q') => return true,
            KeyCode::Char(c) => {
                self.query.push(c);
                self.clamp_sel();
            }
            _ => {}
        }
        false
    }
}

/// Full-width blank row on background `bg` (gap filler).
fn blank_line(width: usize, bg: Color) -> Line<'static> {
    let style = Style::new().bg(bg).fg(FG);
    if width == 0 {
        return Line::from(vec![]).style(style);
    }
    Line::from(vec![Span::styled(" ".repeat(width), Style::new().bg(bg))])
        .style(style)
}

/// Path row at `indent` with accent hits, padded so the
/// selection tint covers the full width. `plain_fg` is FG on
/// the primary path line, COMMENT on variant C's muted line.
fn path_line(
    fixture: &Fixture,
    selected: bool,
    width: usize,
    home: Option<&str>,
    query: &str,
    indent: usize,
    plain_fg: Color,
) -> Line<'static> {
    let bg = if selected { BG_ALT } else { BG };
    let line_style = Style::new().bg(bg).fg(FG);
    let display = abbreviate_home(fixture.path, home);
    let path_w = width.saturating_sub(indent).max(1);
    let path = truncate_path(&display, path_w);
    let mut row = vec![Span::styled(" ".repeat(indent), Style::new().bg(bg))];
    row.extend(path_spans(&path, query, bg, plain_fg));
    let pad = width.saturating_sub(indent + path.chars().count());
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(row).style(line_style)
}

/// Git identity row at `indent`, padded so the selection tint
/// covers the full width.
fn git_line(
    fixture: &Fixture,
    selected: bool,
    width: usize,
    indent: usize,
) -> Line<'static> {
    let bg = if selected { BG_ALT } else { BG };
    let line_style = Style::new().bg(bg).fg(FG);
    let mut row = vec![Span::styled(" ".repeat(indent), Style::new().bg(bg))];
    let budget = width.saturating_sub(indent);
    let content = git_spans(fixture.git, budget);
    let pad = width.saturating_sub(indent + status_width(&content));
    for mut span in content {
        span.style = span.style.bg(bg);
        row.push(span);
    }
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(row).style(line_style)
}

/// Content rows for one item under the current variant. A: the
/// current two-line item (path, git). B: path, tinted blank,
/// git. C: git identity first, muted path below.
fn item_lines(
    fixture: &Fixture,
    selected: bool,
    width: usize,
    home: Option<&str>,
    query: &str,
    variant: Variant,
) -> Vec<Line<'static>> {
    let bg = if selected { BG_ALT } else { BG };
    match variant {
        Variant::A => vec![
            path_line(fixture, selected, width, home, query, SEL_W, FG),
            git_line(fixture, selected, width, GIT_INDENT),
        ],
        Variant::B => vec![
            path_line(fixture, selected, width, home, query, SEL_W, FG),
            blank_line(width, bg),
            git_line(fixture, selected, width, GIT_INDENT),
        ],
        Variant::C => vec![
            git_line(fixture, selected, width, SEL_W),
            path_line(
                fixture,
                selected,
                width,
                home,
                query,
                C_PATH_INDENT,
                COMMENT,
            ),
        ],
    }
}

/// Full-frame render: warm-white bg, the borderless
/// `Search: <query>█` filter row, the variant's item rows, and
/// a two-line status (variant name + description, then counts
/// + key hints). No cards, borders, headings, or selection
/// arrows; selection is the full-width tint only.
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
        Span::styled("█", Style::new().fg(GIT_ICON).bg(BG)),
    ])];
    let list_h =
        (area.height as usize).saturating_sub(FILTER_ROWS + STATUS_ROWS);
    let shown = app.shown();
    let query = app.query.clone();
    let variant = app.variant;
    // Flat rows plus the selected item's row range, so every
    // variant (2- or 3-row items, with or without gaps) shares
    // one row-window scroll.
    let mut flat: Vec<Line<'static>> = vec![];
    let mut sel_start = 0usize;
    let mut sel_end = 0usize;
    if shown.is_empty() {
        flat.push(Line::from(vec![
            Span::styled(" ".repeat(GIT_INDENT), Style::new().bg(BG)),
            Span::styled(
                format!("no matches for \"{}\"", app.query),
                Style::new().fg(COMMENT).bg(BG),
            ),
        ]));
    } else {
        for (pos, i) in shown.iter().enumerate() {
            let selected = pos == app.selected;
            // Variant A gap: an untinted blank row between
            // items; the tint covers only content rows.
            if variant == Variant::A && pos > 0 {
                flat.push(blank_line(width, BG));
            }
            if selected {
                sel_start = flat.len();
            }
            flat.extend(item_lines(
                &app.fixtures[*i],
                selected,
                width,
                app.home.as_deref(),
                &query,
                variant,
            ));
            if selected {
                sel_end = flat.len();
            }
        }
    }
    app.update_scroll_rows(sel_start, sel_end, list_h, flat.len());
    lines.extend(flat.into_iter().skip(app.scroll).take(list_h));
    let sel = if shown.is_empty() {
        "—".to_string()
    } else {
        format!("{}", app.selected + 1)
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} — {}: ", variant.key(), variant.name()),
            Style::new().fg(ACCENT).bg(BG).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            variant.describe().to_string(),
            Style::new().fg(COMMENT).bg(BG),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!(
            "{}/{} shown · sel {} · query \"{}\" · 1/2/3 switch · Tab cycle · q quit",
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

/// Demo loop: switch variants, filter, and navigate with the
/// keyboard; q / Esc / Ctrl-C quits. Everything stays in
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
