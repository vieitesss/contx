// PROTOTYPE (throwaway, not for production).
//
// Question: "How should indent read in grouping B so nested
// worktrees and group children are obvious?"
//
// Fixed B layout for all variants: config group headers
// (comment-colored, not selectable) + remainder-only children
// as two-line items (path then git) + linked worktrees nested
// under their main (extra indent, basename path). Blank row
// between groups, never between siblings. Yellow hits, blue
// ordinary icon / teal linked icon, git_icon branch not bold,
// selection tint on selected child only.
//
// Three structurally different INDENT CUES for that same B
// layout, switchable live:
//   1 — Tree: muted comment-colored box-drawing in the indent
//       columns (`├`/`└` on each child's path row, `│`
//       continuing down through that child's git row so the
//       two-line item does not break the spine). Nested
//       worktrees get a second tree column under the main.
//       Last sibling uses `└`. Headers have no tree glyph.
//       Tests whether a spine makes parent/child/nest read.
//   2 — Gutter bar: a 1-cell vertical stripe in the indent
//       column (`▎`, git_icon blue for ordinary children,
//       teal for nested worktrees). Stripe runs through both
//       path and git rows. No box-drawing. Tests whether a
//       color-coded edge beats box-drawing.
//   3 — Depth band: no tree, no stripe glyph. The indent
//       columns (and only those) paint a stepped background:
//       children a slightly darker band (mix toward bg_alt),
//       nested worktrees a second step further. Content text
//       stays on the normal row bg/selection tint. Tests
//       whether a left margin alone carries depth.
//
// Run: `cargo run --example list_grouping_ui_prototype`.
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
use std::{io, time::Duration};

/// Local light palette for this prototype only, from
/// production `Theme::LIGHT`. Hits read yellow (accent);
/// branch text is git_icon blue, never bold. The ordinary
/// checkout icon shares git_icon blue; the linked-worktree
/// icon is a distinct teal (not the yellow accent, not the
/// red/green counts) so repo vs worktree is obvious.
const BG: Color = Color::Rgb(0xF5, 0xF5, 0xF5);
const FG: Color = Color::Rgb(0x28, 0x28, 0x28);
const BG_ALT: Color = Color::Rgb(0xEA, 0xE7, 0xE1);
const GIT_ICON: Color = Color::Rgb(0x37, 0x52, 0x6D);
const WORKTREE: Color = Color::Rgb(0x3D, 0x7A, 0x6F);
const ACCENT: Color = Color::Rgb(0xCC, 0x96, 0x00);
const COMMENT: Color = Color::Rgb(0x45, 0x55, 0x4D);
const GREEN: Color = Color::Rgb(0x82, 0xA7, 0x62);
const RED: Color = Color::Rgb(0x98, 0x22, 0x2A);
const MUTED: Color = Color::Rgb(0xAA, 0xA4, 0x9C);
/// Depth-band steps for variant 3 only: indent columns (and
/// only those) paint a stepped margin. Child band mixes toward
/// bg_alt; nested inner step goes further so depth reads as a
/// left margin, not as extra characters.
const BAND_CHILD: Color = Color::Rgb(0xEC, 0xE8, 0xE0);
const BAND_NESTED: Color = Color::Rgb(0xDB, 0xD1, 0xBC);

/// FIXTURE checked-out identity: a named branch or a detached
/// short SHA. Shapes mirror `src/tui/git.rs` `Head` but are
/// hardcoded strings here, never read from git.
#[derive(Clone, Copy)]
enum Head {
    Named(&'static str),
    Detached(&'static str),
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

/// One hardcoded look-demo candidate. `remainder` is the path
/// with the group parent already stripped (`skills`, not
/// `~/work/prefapp/skills`) — the repeated prefix the
/// prototype is testing away. `worktree_of` names the
/// remainder of the main checkout this linked worktree nests
/// under in variants B/C (`None` for mains and plain rows).
struct Fixture {
    remainder: &'static str,
    git: FixtureGit,
    worktree_of: Option<&'static str>,
}

/// One hardcoded look-demo group. The header key mirrors a
/// production grouping parent: a config `paths` entry (its
/// children become candidates), a `dir/*` intermediate
/// parent, or `~` for git-from-home discoveries. No new
/// config format is invented here.
struct Group {
    header: &'static str,
    children: Vec<Fixture>,
}

/// FIXTURE sample: every row is hardcoded for the demo.
/// `~/work/prefapp` holds plain repos (long names, dirty
/// pairs, arrows), a main `portal` plus two linked worktrees,
/// and loading/failed rows; `~/personal` holds a detached SHA
/// and a marker; `~` holds one home discovery; `/tmp` holds a
/// nonrepo so the blank git line still reads. Fake branches.
fn fixtures() -> Vec<Group> {
    use Dirty::{Clean, Marker, Pair};
    use Head::{Detached, Named};
    vec![
        Group {
            // FIXTURE: config `paths` parent; children are
            // session candidates grouped under it.
            header: "~/work/prefapp",
            children: vec![
                // FIXTURE: short clean ordinary checkout.
                Fixture {
                    remainder: "skills",
                    git: FixtureGit::Present {
                        linked: false,
                        head: Named("main"),
                        dirty: Clean,
                        upstream: None,
                    },
                    worktree_of: None,
                },
                // FIXTURE: dirty pair with diverged arrows.
                Fixture {
                    remainder: "gitops-k8s",
                    git: FixtureGit::Present {
                        linked: false,
                        head: Named("feat/long-branch-name"),
                        dirty: Pair {
                            added: 3,
                            deleted: 4,
                        },
                        upstream: Some((2, 1)),
                    },
                    worktree_of: None,
                },
                // FIXTURE: long repo + long branch + large
                // counts + ahead arrow: crowding stress.
                Fixture {
                    remainder: "platform-services-with-a-very-long-repo-name",
                    git: FixtureGit::Present {
                        linked: false,
                        head: Named("feat/very-long-branch-name-that-crowds"),
                        dirty: Pair {
                            added: 12_400,
                            deleted: 300,
                        },
                        upstream: Some((1, 0)),
                    },
                    worktree_of: None,
                },
                // FIXTURE: main checkout; its two linked
                // worktrees nest under it in variants B/C.
                Fixture {
                    remainder: "portal",
                    git: FixtureGit::Present {
                        linked: false,
                        head: Named("main"),
                        dirty: Marker,
                        upstream: Some((0, 3)),
                    },
                    worktree_of: None,
                },
                // FIXTURE: linked worktree of `portal`.
                Fixture {
                    remainder: "portal-wt-redesign",
                    git: FixtureGit::Present {
                        linked: true,
                        head: Named("feat/redesign"),
                        dirty: Pair {
                            added: 7,
                            deleted: 1,
                        },
                        upstream: Some((0, 2)),
                    },
                    worktree_of: Some("portal"),
                },
                // FIXTURE: second linked worktree of `portal`.
                Fixture {
                    remainder: "portal-wt-hotfix",
                    git: FixtureGit::Present {
                        linked: true,
                        head: Named("fix/hotfix"),
                        dirty: Clean,
                        upstream: None,
                    },
                    worktree_of: Some("portal"),
                },
                // FIXTURE: still resolving (muted `…`).
                Fixture {
                    remainder: "new-clone",
                    git: FixtureGit::Loading,
                    worktree_of: None,
                },
                // FIXTURE: inspect failure (red ``).
                Fixture {
                    remainder: "broken-link",
                    git: FixtureGit::Failed,
                    worktree_of: None,
                },
            ],
        },
        Group {
            // FIXTURE: second config parent.
            header: "~/personal",
            children: vec![
                // FIXTURE: detached short SHA, dirty, never
                // tracks upstream.
                Fixture {
                    remainder: "contx",
                    git: FixtureGit::Present {
                        linked: false,
                        head: Detached("a9b4c8d"),
                        dirty: Pair {
                            added: 5,
                            deleted: 2,
                        },
                        upstream: None,
                    },
                    worktree_of: None,
                },
                // FIXTURE: marker with a behind arrow.
                Fixture {
                    remainder: "dotfiles",
                    git: FixtureGit::Present {
                        linked: false,
                        head: Named("topic"),
                        dirty: Marker,
                        upstream: Some((0, 2)),
                    },
                    worktree_of: None,
                },
            ],
        },
        Group {
            // FIXTURE: git-from-home discovery; repos found
            // as `$HOME` children group under `~`.
            header: "~",
            children: vec![Fixture {
                remainder: "home-notes",
                git: FixtureGit::Present {
                    linked: false,
                    head: Named("main"),
                    dirty: Clean,
                    upstream: None,
                },
                worktree_of: None,
            }],
        },
        Group {
            // FIXTURE: outside any worktree (blank git line)
            // keeps its own parent key so the blank still
            // reads inside a group.
            header: "/tmp",
            children: vec![Fixture {
                remainder: "scratch",
                git: FixtureGit::Nonrepo,
                worktree_of: None,
            }],
        },
    ]
}

/// Indent cue under test. Same B information hierarchy in all
/// three (headers + remainder-only two-line children + nested
/// worktrees + blank between groups); only the indent treatment
/// differs, not color tweaks of one treatment.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Tree,
    Gutter,
    Band,
}

impl Variant {
    fn next(self) -> Self {
        match self {
            Self::Tree => Self::Gutter,
            Self::Gutter => Self::Band,
            Self::Band => Self::Tree,
        }
    }

    fn key(self) -> char {
        match self {
            Self::Tree => '1',
            Self::Gutter => '2',
            Self::Band => '3',
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Tree => "Tree spine",
            Self::Gutter => "Gutter bar",
            Self::Band => "Depth band",
        }
    }

    /// One-line description surfaced in the status line.
    fn describe(self) -> &'static str {
        match self {
            Self::Tree => {
                "box-drawing spine, second column under main; does the spine carry nest?"
            }
            Self::Gutter => {
                "blue/teal edge stripe through both rows; does color-coded edge beat tree?"
            }
            Self::Band => {
                "stepped margin bg, no glyphs; does a left margin alone carry depth?"
            }
        }
    }

    fn has_headers(self) -> bool {
        // B layout is locked in for all indent cues.
        true
    }

    fn nests_worktrees(self) -> bool {
        // B layout is locked in for all indent cues.
        true
    }
}

/// Leading indent (no selection arrow); group headers sit at
/// it, children hang inside it, nested worktrees inside that.
/// Git lines subordinate two further under their path.
const SEL_W: usize = 2;
const HEADER_INDENT: usize = SEL_W;
const CHILD_INDENT: usize = SEL_W + 2;
const CHILD_GIT_INDENT: usize = CHILD_INDENT + 2;
const WT_INDENT: usize = CHILD_INDENT + 2;
const WT_GIT_INDENT: usize = WT_INDENT + 2;
/// Filter row on top; two status rows at the bottom (variant
/// name + description, then counts + key hints).
const FILTER_ROWS: usize = 1;
const STATUS_ROWS: usize = 2;

const POLL_TICK: Duration = Duration::from_millis(100);

fn main() -> io::Result<()> {
    ratatui::run(|terminal: &mut DefaultTerminal| run(terminal))
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

/// Remainder text with subsequence matches painted in the
/// accent (production `format_line` treatment: hits yellow).
/// Runs on the truncated visible remainder so highlights
/// never break truncation; an empty query renders plain text.
fn path_spans(path: &str, query: &str, bg: Color) -> Vec<Span<'static>> {
    let plain = Style::new().fg(FG).bg(bg);
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

/// Icon color: ordinary checkouts share git_icon blue; linked
/// worktrees get the distinct teal. Bold like production.
fn icon_style(linked: bool) -> Style {
    let color = if linked { WORKTREE } else { GIT_ICON };
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
/// name, or the short SHA when detached.
fn head_name(head: Head) -> &'static str {
    match head {
        Head::Named(n) => n,
        Head::Detached(s) => s,
    }
}

/// Icon plus the checked-out name fitted to `inner_w`: the
/// full name when it fits beside the icon, prefix-kept `…`
/// truncation when only that fits, bare icon otherwise. The
/// branch name is always git_icon blue, never bold; only the
/// icon varies (blue ordinary, teal linked). Same rule as
/// production: never truncates to keep later tokens.
fn icon_with_head(
    linked: bool,
    name: &'static str,
    inner_w: usize,
) -> Vec<Span<'static>> {
    let icon = Span::styled(work_icon(linked), icon_style(linked));
    if 2 + name.chars().count() <= inner_w {
        return vec![
            icon,
            Span::from(" "),
            Span::styled(name.to_string(), Style::new().fg(GIT_ICON)),
        ];
    }
    let room = inner_w.saturating_sub(1);
    if room >= 3 {
        let kept: String = name.chars().take(room - 2).collect();
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

/// Git identity spans for one child's git line, within
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
                vec![Span::styled("\u{f467}", icon_style(false).fg(RED))]
            }
            FixtureGit::Present { .. } => unreachable!(),
        };
    };
    let name = head_name(head);
    let head_extra = 1 + name.chars().count();
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

/// One selectable child in display order: its group, its
/// index inside that group, and whether it renders nested
/// under its main (extra indent, basename path).
#[derive(Clone, Copy)]
struct DisplayChild {
    group: usize,
    child: usize,
    nested: bool,
}

/// Basename shown for a nested worktree path.
fn worktree_basename(remainder: &str) -> &str {
    remainder.rsplit('/').next().unwrap_or(remainder)
}

/// In-memory demo state: FIXTURE groups, layout variant,
/// linear selection into the shown children, row scroll
/// offset, and the filter query. No persistence, no I/O.
struct App {
    groups: Vec<Group>,
    variant: Variant,
    selected: usize,
    scroll: usize,
    query: String,
}

impl App {
    fn new() -> Self {
        Self {
            groups: fixtures(),
            variant: Variant::Tree,
            selected: 0,
            scroll: 0,
            query: String::new(),
        }
    }

    /// Filterable branch/SHA text for one fixture, if any.
    fn branch_text(child: &Fixture) -> Option<&'static str> {
        match child.git {
            FixtureGit::Present { head, .. } => Some(head_name(head)),
            _ => None,
        }
    }

    /// Whether one fixture survives the filter: the query is
    /// a subsequence of the remainder or of the branch/SHA.
    /// Runs on the full remainder in every variant so
    /// switching variants never reshuffles the match set.
    fn matches(child: &Fixture, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        subseq_match(child.remainder, query).is_some()
            || Self::branch_text(child)
                .is_some_and(|b| subseq_match(b, query).is_some())
    }

    /// Visible children of one group in display order. B layout
    /// is locked: each visible main is immediately followed by
    /// its visible linked worktrees (nested), and a worktree
    /// whose main is hidden or absent falls back to a flat
    /// row so a matching worktree is never lost.
    fn ordered_children(&self, group: usize) -> Vec<DisplayChild> {
        let visible: Vec<usize> = (0..self.groups[group].children.len())
            .filter(|&c| {
                Self::matches(&self.groups[group].children[c], &self.query)
            })
            .collect();
        if !self.variant.nests_worktrees() {
            return visible
                .into_iter()
                .map(|child| DisplayChild {
                    group,
                    child,
                    nested: false,
                })
                .collect();
        }
        let mut out = vec![];
        let mut emitted = vec![false; self.groups[group].children.len()];
        for &c in &visible {
            let child = &self.groups[group].children[c];
            if child.worktree_of.is_some() {
                continue;
            }
            out.push(DisplayChild {
                group,
                child: c,
                nested: false,
            });
            emitted[c] = true;
            for &w in &visible {
                let wt = &self.groups[group].children[w];
                if wt.worktree_of == Some(child.remainder) && !emitted[w] {
                    out.push(DisplayChild {
                        group,
                        child: w,
                        nested: true,
                    });
                    emitted[w] = true;
                }
            }
        }
        // Orphan worktrees (main hidden or absent): flat rows.
        for &w in &visible {
            if !emitted[w] {
                out.push(DisplayChild {
                    group,
                    child: w,
                    nested: false,
                });
                emitted[w] = true;
            }
        }
        out
    }

    /// Visible groups with their display-ordered children.
    /// Empty groups are hidden entirely (no header, no
    /// blank), in every variant.
    fn visible_groups(&self) -> Vec<(usize, Vec<DisplayChild>)> {
        (0..self.groups.len())
            .filter_map(|g| {
                let kids = self.ordered_children(g);
                if kids.is_empty() {
                    None
                } else {
                    Some((g, kids))
                }
            })
            .collect()
    }

    /// Flattened selectable children across visible groups.
    fn flattened(&self) -> Vec<DisplayChild> {
        self.visible_groups()
            .into_iter()
            .flat_map(|(_, kids)| kids)
            .collect()
    }

    fn total_children(&self) -> usize {
        self.groups.iter().map(|g| g.children.len()).sum()
    }

    /// Clamp selection after the shown set shrinks.
    fn clamp_sel(&mut self) {
        let n = self.flattened().len();
        if n == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(n - 1);
        }
    }

    /// Linear move with no wrap; no-op when empty. Headers
    /// are labels only and never enter this index.
    fn move_sel(&mut self, dy: i32) {
        let n = self.flattened().len();
        if n == 0 {
            return;
        }
        let next = self.selected as i32 + dy;
        self.selected = next.clamp(0, n as i32 - 1) as usize;
    }

    /// Keep the whole selected child block inside the visible
    /// window. `sel_start..sel_end` is the flat-row range of
    /// the selected child (empty when the list is empty).
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
    /// cycles, Up/Down (Ctrl-j/k, Home/End) move among
    /// selectable children only, printable keys filter,
    /// Enter is a no-op, q/Esc (Ctrl-C) quit. Only in-memory
    /// UI state changes: no tmux, no writes.
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
                self.clamp_sel();
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.clamp_sel();
            }
            KeyCode::Up => self.move_sel(-1),
            KeyCode::Down => self.move_sel(1),
            KeyCode::Home => {
                if !self.flattened().is_empty() {
                    self.selected = 0;
                }
            }
            KeyCode::End => {
                let n = self.flattened().len();
                if n > 0 {
                    self.selected = n - 1;
                }
            }
            // Variant switcher: digits are reserved, so they
            // never reach the filter query.
            KeyCode::Char('1') => {
                self.variant = Variant::Tree;
                self.clamp_sel();
            }
            KeyCode::Char('2') => {
                self.variant = Variant::Gutter;
                self.clamp_sel();
            }
            KeyCode::Char('3') => {
                self.variant = Variant::Band;
                self.clamp_sel();
            }
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

/// Full-width blank row on the base background (group
/// separator, never tinted, never selectable).
fn blank_line(width: usize) -> Line<'static> {
    let style = Style::new().bg(BG).fg(FG);
    if width == 0 {
        return Line::from(vec![]).style(style);
    }
    Line::from(vec![Span::styled(" ".repeat(width), Style::new().bg(BG))])
        .style(style)
}

/// Group header row: the abbreviated parent in comment
/// color, never selected, never tinted, never activated.
fn header_line(header: &str, width: usize) -> Line<'static> {
    let style = Style::new().bg(BG).fg(FG);
    let shown =
        truncate_path(header, width.saturating_sub(HEADER_INDENT).max(1));
    let mut row =
        vec![Span::styled(" ".repeat(HEADER_INDENT), Style::new().bg(BG))];
    row.push(Span::styled(shown.clone(), Style::new().fg(COMMENT).bg(BG)));
    let pad = width.saturating_sub(HEADER_INDENT + shown.chars().count());
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(BG)));
    }
    Line::from(row).style(style)
}

/// Remainder-only path row with an explicit indent prefix,
/// padded so the selection tint covers the full width when
/// this child is selected. `prefix` must already be
/// `prefix_width` cells wide; truncation and padding account
/// for it. Tree/gutter prefixes sit on the row bg (so the
/// tint still spans full width); band prefixes sit on their
/// stepped margin bgs while the content stays on the row bg.
fn path_line_with_prefix(
    prefix: Vec<Span<'static>>,
    prefix_width: usize,
    text: &str,
    query: &str,
    selected: bool,
    width: usize,
) -> Line<'static> {
    let bg = if selected { BG_ALT } else { BG };
    let line_style = Style::new().bg(bg).fg(FG);
    let shown = truncate_path(text, width.saturating_sub(prefix_width).max(1));
    let mut row = prefix;
    row.extend(path_spans(&shown, query, bg));
    let pad = width.saturating_sub(prefix_width + shown.chars().count());
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(row).style(line_style)
}

/// Git identity row with an explicit indent prefix, padded so
/// the selection tint covers the full width when this child
/// is selected. Same prefix-width contract as
/// `path_line_with_prefix`.
fn git_line_with_prefix(
    prefix: Vec<Span<'static>>,
    prefix_width: usize,
    git: FixtureGit,
    selected: bool,
    width: usize,
) -> Line<'static> {
    let bg = if selected { BG_ALT } else { BG };
    let line_style = Style::new().bg(bg).fg(FG);
    let mut row = prefix;
    let budget = width.saturating_sub(prefix_width);
    let content = git_spans(git, budget);
    let pad = width.saturating_sub(prefix_width + status_width(&content));
    for mut span in content {
        span.style = span.style.bg(bg);
        row.push(span);
    }
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(row).style(line_style)
}

/// Spaces on `bg`.
fn bg_spaces(n: usize, bg: Color) -> Span<'static> {
    Span::styled(" ".repeat(n), Style::new().bg(bg))
}

/// Comment-colored tree glyph on the row bg.
fn tree_span(glyph: &'static str, bg: Color) -> Span<'static> {
    Span::styled(glyph.to_string(), Style::new().fg(COMMENT).bg(bg))
}

/// Variant 1 prefix: muted box-drawing in the indent columns.
/// `nested` selects the one- vs two-column shape; `is_last`
/// is last-sibling (or last-nested under its main) selecting
/// `└`/blank over `├`/`│`; `parent_is_last` decides the outer
/// continuation for nested rows; `is_path_row` selects branch
/// (`├`/`└`) vs continuation (`│`/blank) so the spine runs
/// through the git row. Widths match the B indents
/// (4/6 ordinary path/git, 6/8 nested path/git). Headers never
/// call this (no tree glyph).
fn tree_prefix(
    nested: bool,
    is_last: bool,
    parent_is_last: bool,
    is_path_row: bool,
    bg: Color,
) -> (Vec<Span<'static>>, usize) {
    if !nested {
        if is_path_row {
            let glyph = if is_last { "└" } else { "├" };
            (
                vec![
                    bg_spaces(SEL_W, bg),
                    tree_span(glyph, bg),
                    bg_spaces(1, bg),
                ],
                CHILD_INDENT,
            )
        } else if is_last {
            (vec![bg_spaces(CHILD_GIT_INDENT, bg)], CHILD_GIT_INDENT)
        } else {
            (
                vec![
                    bg_spaces(SEL_W, bg),
                    tree_span("│", bg),
                    bg_spaces(CHILD_GIT_INDENT - SEL_W - 1, bg),
                ],
                CHILD_GIT_INDENT,
            )
        }
    } else if is_path_row {
        let outer_last = parent_is_last;
        let inner = if is_last { "└" } else { "├" };
        let mut prefix = vec![bg_spaces(SEL_W, bg)];
        if outer_last {
            prefix.push(bg_spaces(1, bg));
        } else {
            prefix.push(tree_span("│", bg));
        }
        prefix.push(bg_spaces(1, bg));
        prefix.push(tree_span(inner, bg));
        prefix.push(bg_spaces(1, bg));
        (prefix, WT_INDENT)
    } else {
        let outer_last = parent_is_last;
        let mut prefix = vec![bg_spaces(SEL_W, bg)];
        if outer_last {
            prefix.push(bg_spaces(1, bg));
        } else {
            prefix.push(tree_span("│", bg));
        }
        prefix.push(bg_spaces(1, bg));
        if is_last {
            prefix.push(bg_spaces(1, bg));
        } else {
            prefix.push(tree_span("│", bg));
        }
        prefix.push(bg_spaces(WT_GIT_INDENT - SEL_W - 3, bg));
        (prefix, WT_GIT_INDENT)
    }
}

/// Variant 2 prefix: a 1-cell vertical stripe (`▎`) in the
/// indent column, blue for ordinary children and teal for
/// nested worktrees. The stripe column is stepped (ordinary
/// at child depth, nested two cells deeper) and runs through
/// both path and git rows so the two-line item stays edged.
/// No box-drawing. Widths match the B indents.
fn gutter_prefix(
    nested: bool,
    is_path_row: bool,
    bg: Color,
) -> (Vec<Span<'static>>, usize) {
    let stripe = |color: Color| {
        Span::styled("▎".to_string(), Style::new().fg(color).bg(bg))
    };
    if !nested {
        if is_path_row {
            (
                vec![bg_spaces(SEL_W, bg), stripe(GIT_ICON), bg_spaces(1, bg)],
                CHILD_INDENT,
            )
        } else {
            (
                vec![
                    bg_spaces(SEL_W, bg),
                    stripe(GIT_ICON),
                    bg_spaces(CHILD_GIT_INDENT - SEL_W - 1, bg),
                ],
                CHILD_GIT_INDENT,
            )
        }
    } else if is_path_row {
        (
            vec![
                bg_spaces(WT_INDENT - 2, bg),
                stripe(WORKTREE),
                bg_spaces(1, bg),
            ],
            WT_INDENT,
        )
    } else {
        (
            vec![
                bg_spaces(WT_INDENT - 2, bg),
                stripe(WORKTREE),
                bg_spaces(WT_GIT_INDENT - (WT_INDENT - 2) - 1, bg),
            ],
            WT_GIT_INDENT,
        )
    }
}

/// Variant 3 prefix: stepped margin bg, no glyphs. The indent
/// columns (and only those) paint `BAND_CHILD` for children;
/// nested worktrees keep `BAND_CHILD` on the outer step and
/// add `BAND_NESTED` on the inner step. Content stays on the
/// normal row bg/selection tint. Widths match the B indents.
fn band_prefix(nested: bool, is_path_row: bool) -> (Vec<Span<'static>>, usize) {
    let child =
        |n: usize| Span::styled(" ".repeat(n), Style::new().bg(BAND_CHILD));
    let nested_span =
        |n: usize| Span::styled(" ".repeat(n), Style::new().bg(BAND_NESTED));
    if !nested {
        if is_path_row {
            (vec![child(CHILD_INDENT)], CHILD_INDENT)
        } else {
            (vec![child(CHILD_GIT_INDENT)], CHILD_GIT_INDENT)
        }
    } else if is_path_row {
        (
            vec![child(CHILD_INDENT), nested_span(WT_INDENT - CHILD_INDENT)],
            WT_INDENT,
        )
    } else {
        (
            vec![
                child(CHILD_INDENT),
                nested_span(WT_GIT_INDENT - CHILD_INDENT),
            ],
            WT_GIT_INDENT,
        )
    }
}

/// Full-frame render: warm-white bg, the borderless
/// `Search: <query>█` filter row, grouped two-line children,
/// and a two-line status naming the variant. No cards,
/// borders, path/git headings, or selection arrows;
/// selection is the full-width tint on the selected child
/// only, headers and blanks never tinted.
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
    let vgroups = app.visible_groups();
    let query = app.query.clone();
    let variant = app.variant;
    let selected = app.selected;
    // Flat rows plus the selected child's row range, so all
    // variants share one row-window scroll. Selection counts
    // selectable children; headers/blanks never shift it.
    let mut flat: Vec<Line<'static>> = vec![];
    let mut sel_start = 0usize;
    let mut sel_end = 0usize;
    if vgroups.iter().all(|(_, kids)| kids.is_empty()) {
        flat.push(Line::from(vec![
            Span::styled(" ".repeat(CHILD_GIT_INDENT), Style::new().bg(BG)),
            Span::styled(
                format!("no matches for \"{}\"", app.query),
                Style::new().fg(COMMENT).bg(BG),
            ),
        ]));
    } else {
        let mut pos = 0usize;
        for (gi_pos, (gi, kids)) in vgroups.iter().enumerate() {
            if gi_pos > 0 {
                flat.push(blank_line(width));
            }
            if variant.has_headers() {
                flat.push(header_line(app.groups[*gi].header, width));
            }
            for (kid_pos, dc) in kids.iter().enumerate() {
                let child = &app.groups[dc.group].children[dc.child];
                let is_sel = pos == selected;
                let bg = if is_sel { BG_ALT } else { BG };
                // Tree topology for this group: last top-level
                // selects `└`/blank over `├`/`│`; nested rows
                // also need parent-last (outer `│` vs blank)
                // and last-nested (inner `└`/blank vs `├`/`│`).
                let total_tops = kids.iter().filter(|d| !d.nested).count();
                let (is_last, parent_is_last) = if !dc.nested {
                    let order = kids
                        .iter()
                        .take(kid_pos + 1)
                        .filter(|d| !d.nested)
                        .count()
                        .saturating_sub(1);
                    (total_tops > 0 && order + 1 == total_tops, false)
                } else {
                    let parent = child.worktree_of.unwrap_or("");
                    let sib_total = kids
                        .iter()
                        .filter(|d| {
                            d.nested
                                && app.groups[d.group].children[d.child]
                                    .worktree_of
                                    == Some(parent)
                        })
                        .count();
                    let sib_order = kids
                        .iter()
                        .take(kid_pos + 1)
                        .filter(|d| {
                            d.nested
                                && app.groups[d.group].children[d.child]
                                    .worktree_of
                                    == Some(parent)
                        })
                        .count()
                        .saturating_sub(1);
                    let is_last_nested =
                        sib_total > 0 && sib_order + 1 == sib_total;
                    let parent_flat = kids.iter().position(|d| {
                        !d.nested
                            && app.groups[d.group].children[d.child].remainder
                                == parent
                    });
                    let parent_is_last = if let Some(pp) = parent_flat {
                        let porder = kids
                            .iter()
                            .take(pp + 1)
                            .filter(|d| !d.nested)
                            .count()
                            .saturating_sub(1);
                        porder + 1 == total_tops
                    } else {
                        true
                    };
                    (is_last_nested, parent_is_last)
                };
                let text = if dc.nested {
                    worktree_basename(child.remainder)
                } else {
                    child.remainder
                };
                let (path_prefix, path_w) = match variant {
                    Variant::Tree => tree_prefix(
                        dc.nested,
                        is_last,
                        parent_is_last,
                        true,
                        bg,
                    ),
                    Variant::Gutter => gutter_prefix(dc.nested, true, bg),
                    Variant::Band => band_prefix(dc.nested, true),
                };
                let (git_prefix, git_w) = match variant {
                    Variant::Tree => tree_prefix(
                        dc.nested,
                        is_last,
                        parent_is_last,
                        false,
                        bg,
                    ),
                    Variant::Gutter => gutter_prefix(dc.nested, false, bg),
                    Variant::Band => band_prefix(dc.nested, false),
                };
                if is_sel {
                    sel_start = flat.len();
                }
                flat.push(path_line_with_prefix(
                    path_prefix,
                    path_w,
                    text,
                    &query,
                    is_sel,
                    width,
                ));
                flat.push(git_line_with_prefix(
                    git_prefix, git_w, child.git, is_sel, width,
                ));
                if is_sel {
                    sel_end = flat.len();
                }
                pos += 1;
            }
        }
    }
    app.update_scroll_rows(sel_start, sel_end, list_h, flat.len());
    lines.extend(flat.into_iter().skip(app.scroll).take(list_h));
    let shown = app.flattened().len();
    let sel = if shown == 0 {
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
            "{shown}/{} shown · sel {sel} · query \"{}\" · 1/2/3 switch · Tab cycle · q quit",
            app.total_children(),
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
