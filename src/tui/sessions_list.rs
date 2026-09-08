use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{StatefulWidget, Widget},
};
use terminal_colorsaurus::ThemeMode;

use crate::{
    fuzzy::Match,
    theme::Theme,
    tui::{
        git::{
            CandidateState, GitStates, Head, Upstream, WorkState,
            measurable_texts,
        },
        selection::Selection,
    },
};

/// Two terminal rows per item: path first, Git metadata underneath.
/// The list is always a single full-width column.
pub(crate) const ITEM_ROWS: u16 = 2;
/// Leading indent on the path line; no selection marker arrow.
pub(crate) const PATH_INDENT: usize = 2;
/// Git line pads the path indent plus a two-space subordinate indent.
pub(crate) const GIT_INDENT: usize = PATH_INDENT + 2;
/// Grouped-list indents from grouping prototype B: headers sit
/// at the selection width, children hang one tree column
/// inside, nested worktrees one more. Git lines subordinate
/// two further under their path row.
pub(crate) const HEADER_INDENT: usize = PATH_INDENT;
pub(crate) const CHILD_PATH_INDENT: usize = 4;
pub(crate) const CHILD_GIT_INDENT: usize = 6;
pub(crate) const NEST_PATH_INDENT: usize = 6;
pub(crate) const NEST_GIT_INDENT: usize = 8;

/// Display-only home abbreviation. Exact `home` becomes `~`;
/// descendants become `~/rest`. Sibling string prefixes and
/// missing/empty home leave `path` unchanged. Private helper of
/// `highlighted_path`, which maps hits onto its output without
/// re-deriving the shortening from it.
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

/// One shortening cut over `display`, chosen once so visible text
/// and surviving-hit positions follow the same decision instead
/// of re-deriving the branch from an inserted `…` (which a real
/// path may also start with). Returns the visible text plus, for
/// every display char index, its visible position when kept.
/// Dropped chars map to `None`; the inserted `…` is decoration
/// and never appears as a kept position, so it cannot be a hit.
fn shape_path(display: &str, max: usize) -> (String, Vec<Option<usize>>) {
    let chars: Vec<char> = display.chars().collect();
    let n = chars.len();
    if n <= max {
        return (display.to_string(), (0..n).map(Some).collect());
    }
    if max == 0 {
        return (String::new(), vec![None; n]);
    }
    let basename_len = display
        .rsplit('/')
        .next()
        .map(|b| b.chars().count())
        .unwrap_or(n);
    let mut visible = String::new();
    let mut at: Vec<Option<usize>> = vec![None; n];
    let mut v = 0;
    if display.contains('/') && basename_len + 2 <= max {
        let prefix_len = max - basename_len - 1;
        for (i, c) in chars.iter().enumerate().take(prefix_len) {
            visible.push(*c);
            at[i] = Some(v);
            v += 1;
        }
        visible.push('…');
        v += 1;
        for (i, c) in chars.iter().enumerate().skip(n - basename_len) {
            visible.push(*c);
            at[i] = Some(v);
            v += 1;
        }
        return (visible, at);
    }
    visible.push('…');
    v += 1;
    for (i, c) in chars.iter().enumerate().skip(n - (max - 1)) {
        visible.push(*c);
        at[i] = Some(v);
        v += 1;
    }
    (visible, at)
}

/// Visible path plus the fuzzy hits that survive display shaping.
///
/// Maps byte `ranges` (into the raw candidate `entry`, as stored in
/// `Match::match_ranges`) through home abbreviation then one
/// `shape_path` cut. Returns the truncated display string
/// and the sorted char indices of surviving hits within it. The
/// inserted `…` and the collapsed `$HOME` prefix are never hits;
/// an empty query yields no hits. Ranges that fail to map (hits in
/// a dropped middle or prefix) disappear; text and positions come
/// from the same cut, so a surprise shape loses highlights
/// instead of mispainting them.
pub(crate) fn highlighted_path(
    entry: &str,
    ranges: &[(usize, usize)],
    home: Option<&str>,
    max: usize,
) -> (String, Vec<usize>) {
    let display = abbreviate_home(entry, home);
    let carried = carry_abbreviated(entry, ranges, &display);
    finish_highlight(&display, &carried, max)
}

/// Carry byte ranges from `entry` over to home-abbreviated
/// `display`. Abbreviation only swaps a leading byte-prefix
/// for `~`, so a range survives exactly when it sits in the
/// verbatim tail.
fn carry_abbreviated(
    entry: &str,
    ranges: &[(usize, usize)],
    display: &str,
) -> Vec<(usize, usize)> {
    let mut carried: Vec<(usize, usize)> = vec![];
    if display == entry {
        carried.extend_from_slice(ranges);
    } else if let Some(tail) = display.strip_prefix('~') {
        if entry.ends_with(tail) {
            let base = entry.len() - tail.len();
            for &(s, e) in ranges {
                if s >= base {
                    carried.push((s - base + 1, e - base + 1));
                }
            }
        }
    }
    carried
}

/// Visible text plus surviving hits for one display string:
/// byte offsets to char indices, then one `shape_path` cut.
/// The inserted `…` and anything the cut dropped are never
/// hits; text and positions come from the same cut, so a
/// surprise shape loses highlights instead of mispainting
/// them.
fn finish_highlight(
    display: &str,
    carried: &[(usize, usize)],
    max: usize,
) -> (String, Vec<usize>) {
    // Display byte offset -> char index; each range covers one char.
    let starts: Vec<usize> = display
        .char_indices()
        .map(|(i, _)| i)
        .chain([display.len()])
        .collect();
    let mut hits: Vec<usize> = vec![];
    for (s, e) in carried {
        if let Ok(idx) = starts.binary_search(s) {
            if starts.get(idx + 1) == Some(e) {
                hits.push(idx);
            }
        }
    }
    let (visible, at) = shape_path(display, max);
    // Keep only hits on surviving chars, at the visible positions
    // the same cut produced. Anything the cut dropped — collapsed
    // prefix, shortened middle, squeezed prefix — maps to `None`
    // and disappears instead of mispainting.
    let mut mapped: Vec<usize> = hits
        .into_iter()
        .filter_map(|i| at.get(i).copied().flatten())
        .collect();
    mapped.sort_unstable();
    mapped.dedup();
    (visible, mapped)
}

/// Visible remainder plus the fuzzy hits that survive group
/// stripping. Maps byte `ranges` (into the raw candidate
/// `entry`) onto `tail` — the group remainder, or its basename
/// for nested worktrees — then one `shape_path` cut. The
/// collapsed group prefix is never a hit, like the collapsed
/// `$HOME` prefix; unicode-safe throughout.
pub(crate) fn highlighted_remainder(
    entry: &str,
    ranges: &[(usize, usize)],
    tail: &str,
    max: usize,
) -> (String, Vec<usize>) {
    let mut carried: Vec<(usize, usize)> = vec![];
    if entry.ends_with(tail) {
        let base = entry.len() - tail.len();
        for &(s, e) in ranges {
            if s >= base {
                carried.push((s - base, e - base));
            }
        }
    }
    finish_highlight(tail, &carried, max)
}

/// Basename of a `/`-delimited path tail.
fn work_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Visible remainder of `entry` under its config `group`: the
/// tail past `group/`. Entries outside their group show their
/// basename, so the row still reads inside a group.
fn group_remainder<'a>(entry: &'a str, group: &str) -> &'a str {
    let prefix = format!("{group}/");
    match entry.strip_prefix(prefix.as_str()) {
        Some(rest) => rest,
        None => work_basename(entry),
    }
}

/// Whether any fuzzy range sits in the group-stripped tail.
/// Prefix-only hits on the collapsed group path do not count:
/// those children render with no remainder highlight and must
/// not list under a non-empty query.
fn remainder_carries_hit(
    entry: &str,
    ranges: &[(usize, usize)],
    tail: &str,
) -> bool {
    if !entry.ends_with(tail) {
        return false;
    }
    let base = entry.len() - tail.len();
    ranges.iter().any(|&(s, _)| s >= base)
}

/// Position in resolved `group_order`, or `usize::MAX` for a
/// key the resolved list never saw (appended in match order).
fn group_order_pos(group_order: &[String], key: &str) -> usize {
    group_order
        .iter()
        .position(|g| g == key)
        .unwrap_or(usize::MAX)
}

/// Path text with surviving fuzzy hits painted in the yellow accent.
/// `hits` holds char indices into `path`; an empty set renders
/// plain text in the default style.
fn path_spans(
    path: &str,
    hits: &[usize],
    plain: Style,
    hit: Style,
) -> Vec<Span<'static>> {
    if hits.is_empty() {
        return vec![Span::styled(path.to_string(), plain)];
    }
    let mut out = vec![];
    let mut buf = String::new();
    let mut in_hit = false;
    let mut remaining = hits.iter().peekable();
    for (i, c) in path.chars().enumerate() {
        let is_hit = remaining.peek().is_some_and(|&&h| h == i);
        if is_hit {
            remaining.next();
        }
        if buf.is_empty() {
            in_hit = is_hit;
        } else if is_hit != in_hit {
            let style = if in_hit { hit } else { plain };
            out.push(Span::styled(std::mem::take(&mut buf), style));
            in_hit = is_hit;
        }
        buf.push(c);
    }
    if !buf.is_empty() {
        let style = if in_hit { hit } else { plain };
        out.push(Span::styled(buf, style));
    }
    out
}

fn work_icon(linked: bool) -> &'static str {
    if linked { "\u{ec7d}" } else { "\u{ec6f}" }
}

fn icon_style(color: ratatui::style::Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

fn status_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// Checked-out name for the identity slot: the exact branch
/// name, or the short SHA when detached. `Absent` renders
/// nothing, never `HEAD` or an invented branch.
fn head_name(head: &Head) -> Option<&str> {
    match head {
        Head::Named(name) => Some(name),
        Head::Detached { short } => Some(short),
        Head::Absent => None,
    }
}

/// Icon plus the checked-out name fitted to `inner_w`: the
/// full name when it fits beside the icon, prefix-kept `…`
/// truncation when only that fits, bare icon otherwise. Never
/// truncates to keep later tokens: callers drop upstream,
/// then dirty, before the name compacts against the icon.
fn icon_with_head(
    linked: bool,
    name: Option<&str>,
    inner_w: usize,
    theme: Theme,
) -> Vec<Span<'static>> {
    // Ordinary checkouts share git_icon blue; linked worktrees
    // get the distinct teal. The name always stays git_icon,
    // never bold like the icon.
    let icon_color = if linked {
        theme.worktree
    } else {
        theme.git_icon
    };
    let icon = Span::styled(work_icon(linked), icon_style(icon_color));
    let Some(n) = name else {
        return vec![icon];
    };
    if 2 + n.chars().count() <= inner_w {
        return vec![
            icon,
            Span::from(" "),
            Span::styled(n.to_string(), Style::new().fg(theme.git_icon)),
        ];
    }
    let room = inner_w.saturating_sub(1);
    // After the separating space, a truncation needs at least
    // one kept char plus the `…`: never a lone ellipsis.
    if room >= 3 {
        let kept: String = n.chars().take(room - 2).collect();
        return vec![
            icon,
            Span::from(" "),
            Span::styled(format!("{kept}…"), Style::new().fg(theme.git_icon)),
        ];
    }
    vec![icon]
}

/// Nonzero divergence tokens in render order, behind first:
/// `(glyph, count)` with exact decimals and no unit compaction
/// (arrows never read `1k`). `↓` is U+2193, `↑` is U+2191.
/// Zeros and `Absent` yield no tokens: the caller omits them,
/// never a placeholder.
fn upstream_tokens(upstream: Upstream) -> Vec<(&'static str, u64)> {
    match upstream {
        Upstream::Counts { ahead, behind } => {
            let mut out = Vec::new();
            if behind > 0 {
                out.push(("\u{2193}", behind));
            }
            if ahead > 0 {
                out.push(("\u{2191}", ahead));
            }
            out
        }
        Upstream::Absent => Vec::new(),
    }
}

/// Append nonzero cached `↓behind ↑ahead` after full local Git
/// content when the whole line still fits `inner_w`. Callers
/// pass only uncompacted content: once a dirty pair or marker
/// drops, upstream stays dropped too, so narrower widths never
/// reattach arrows to a bare icon. Behind is red like
/// deletions, ahead green like additions, both normal weight;
/// one space separates tokens, none inside a token. A diverged
/// pair appends both or neither, and never compacts itself.
fn append_upstream(
    mut local: Vec<Span<'static>>,
    upstream: Upstream,
    inner_w: usize,
    theme: Theme,
) -> Vec<Span<'static>> {
    let tokens = upstream_tokens(upstream);
    if tokens.is_empty() {
        return local;
    }
    let extra: usize = tokens
        .iter()
        .map(|(g, n)| g.chars().count() + n.to_string().len() + 1)
        .sum();
    // One separating space per token; char counts match the
    // local pair logic, so PUA icons still count as one cell.
    if status_width(&local) + extra > inner_w {
        return local;
    }
    for (glyph, count) in tokens {
        let color = if glyph == "\u{2193}" {
            theme.red
        } else {
            theme.green
        };
        local.push(Span::from(" "));
        local.push(Span::styled(
            format!("{glyph}{count}"),
            Style::new().fg(color),
        ));
    }
    local
}

/// Git status for one item's second line. Nonrepo is blank.
/// The worktree icon pairs with the checked-out name, then
/// the local dirty pair or marker, then nonzero upstream
/// arrows. As width shrinks, upstream drops first, then the
/// dirty pair/marker, then the name truncates against the
/// icon; the icon itself never drops for a name.
pub(crate) fn git_spans(
    state: Option<&CandidateState>,
    inner_w: usize,
    theme: Theme,
) -> Vec<Span<'static>> {
    if inner_w == 0 {
        return vec![];
    }
    let icon_of = |linked: bool| {
        let icon_color = if linked {
            theme.worktree
        } else {
            theme.git_icon
        };
        Span::styled(work_icon(linked), icon_style(icon_color))
    };
    match state {
        None => vec![Span::styled(
            "…",
            Style::new().fg(theme.comment).add_modifier(Modifier::DIM),
        )],
        Some(s) if s.state == WorkState::Failed => {
            vec![Span::styled("\u{f467}", icon_style(theme.red))]
        }
        Some(s) if s.root.is_none() => vec![],
        Some(s) => {
            // Identity pair first: the name reserves its full
            // width before dirty budgets are computed, so the
            // name never compacts to keep dirty or upstream.
            // Compacted fallbacks (name-only, bare icon) never
            // reach `append_upstream`: a dropped pair or marker
            // keeps upstream dropped, and narrower widths cannot
            // show more than wider ones did.
            let name = head_name(&s.head);
            let head_extra = name.map(|n| 1 + n.chars().count()).unwrap_or(0);
            match s.state {
                WorkState::Clean => append_upstream(
                    icon_with_head(s.linked, name, inner_w, theme),
                    s.upstream,
                    inner_w,
                    theme,
                ),
                WorkState::Measurable { added, deleted } => {
                    let base = icon_with_head(s.linked, name, inner_w, theme);
                    // Full identity fits: dirty may follow it.
                    // Otherwise the name already compacted
                    // against the icon, so neither dirty nor
                    // upstream may join it.
                    if 1 + head_extra > inner_w {
                        base
                    } else {
                        let mut full = base;
                        let budget = inner_w.saturating_sub(2 + head_extra);
                        match measurable_texts(added, deleted, budget) {
                            Some((a, d)) => {
                                full.push(Span::from(" "));
                                full.push(Span::styled(
                                    a,
                                    Style::new().fg(theme.green),
                                ));
                                full.push(Span::from(" "));
                                full.push(Span::styled(
                                    d,
                                    Style::new().fg(theme.red),
                                ));
                                append_upstream(
                                    full, s.upstream, inner_w, theme,
                                )
                            }
                            None => full,
                        }
                    }
                }
                WorkState::Marker => {
                    let base = icon_with_head(s.linked, name, inner_w, theme);
                    if 1 + head_extra > inner_w {
                        base
                    } else {
                        let mut full = base;
                        // The marker trails the identity pair
                        // when it fits; otherwise the pair
                        // stands without it or upstream.
                        if status_width(&full) + 2 <= inner_w {
                            full.push(Span::from(" "));
                            full.push(Span::styled(
                                "•",
                                Style::new().fg(theme.accent),
                            ));
                            append_upstream(full, s.upstream, inner_w, theme)
                        } else {
                            full
                        }
                    }
                }
                WorkState::Failed => vec![icon_of(s.linked)],
            }
        }
    }
}

pub struct SessionsListState {
    pub(crate) scroll: usize,
    pub(crate) theme_mode: ThemeMode,
    /// Last-known observed Git state: the observation module
    /// owns omitted-retain, so rendering only presents it.
    pub(crate) git: GitStates,
    /// Home directory for display-only `~` abbreviation.
    /// Defaults from `$HOME`; tests assign a fake home directly
    /// (same pattern as `git`) so parallel tests never touch the
    /// process environment.
    pub(crate) home: Option<String>,
    /// Config group per candidate path, threaded through from
    /// `config::resolve` so the list can group without re-reading
    /// config.
    pub(crate) groups: HashMap<String, String>,
    /// Group keys in first-seen resolved-candidate order,
    /// threaded through from `Tui::new` beside `groups`.
    /// Empty-query grouping sequences groups by this; a
    /// non-empty query lays out remainder-hit children in
    /// fuzzy rank order (headers still index into this).
    /// An empty order (hand-built states) falls back to
    /// first-seen match order.
    pub(crate) group_order: Vec<String>,
    /// Groups currently rendered folded: header only, no
    /// children. All groups start folded; a non-empty query
    /// auto-unfolds groups with a remainder hit and keeps
    /// prefix-only matching groups folded; empty groups stay
    /// hidden. Clearing the query refolds everything. Manual
    /// expand/collapse is NOT persisted across query changes
    /// — the query recompute always overrides.
    pub(crate) folded: HashSet<String>,
    /// Index into `group_order` when a folded header is the
    /// focused visual target. `None` when a child entry is
    /// focused (the normal case). A folded header is a single
    /// selectable row: Enter/Right/Space expands, Left is a
    /// no-op; on a child, Left collapses the parent group and
    /// lands here.
    pub(crate) active_header: Option<usize>,
    /// Group key of home-repository discovery, if any surviving
    /// candidate still carries `from_home_discovery`. Prefix-only
    /// headers skip this key; remainder-hit and empty-query
    /// headers do not. Not inferred from `home`.
    pub(crate) home_discovery_group: Option<String>,
}

impl SessionsListState {
    pub fn new(theme_mode: ThemeMode) -> Self {
        Self {
            scroll: 0,
            theme_mode,
            git: GitStates::new(),
            home: std::env::var("HOME").ok(),
            groups: HashMap::new(),
            group_order: vec![],
            folded: HashSet::new(),
            active_header: None,
            home_discovery_group: None,
        }
    }

    fn update_scroll(
        &mut self,
        rows_visible: usize,
        total_rows: usize,
        sel_row: usize,
    ) {
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
    }

    /// Keep the selected child's whole two-line block inside a
    /// row-offset window: `sel_start..sel_end` spans its path
    /// plus git rows. Headers and blanks occupy rows but never
    /// shift selection; `scroll` is a row offset here, while the
    /// flat list above keeps it an item index.
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
}

/// One visible match placed in its config group: the
/// group-stripped remainder plus the tree topology for its
/// rows (`last` selects corner over branch; `parent_last`
/// decides the outer column of nested rows). The group key
/// itself travels on the bucket tuple below.
struct PlacedChild<'a> {
    m: &'a Match,
    match_idx: usize,
    remainder: String,
    nested: bool,
    last: bool,
    parent_last: bool,
}

/// Worktree root of one visible match, if observed yet.
fn match_root(git: &GitStates, m: &Match) -> Option<String> {
    git.get(&m.entry).and_then(|s| s.root.clone())
}

/// Primary worktree a visible match nests under, if it is a
/// linked checkout that names one.
fn match_primary(git: &GitStates, m: &Match) -> Option<String> {
    git.get(&m.entry)
        .filter(|s| s.linked)
        .and_then(|s| s.primary.clone())
}

/// Whether one visible match nests under its main: a linked
/// checkout whose primary equals another current same-group
/// match's root. Anything else — ordinary checkouts, unknown
/// or failed git, a primary with no current match — stays a
/// flat group child, never a hanging nest.
fn is_nested(
    matches: &[Match],
    members: &[usize],
    git: &GitStates,
    idx: usize,
) -> bool {
    let Some(primary) = match_primary(git, &matches[idx]) else {
        return false;
    };
    members.iter().any(|&j| {
        j != idx && match_root(git, &matches[j]).as_deref() == Some(&primary)
    })
}

/// Rows for one group's members: each top followed by the nest
/// it parents (first top carrying that root wins), then
/// unparented nested as flat closers.
fn nest_order(
    matches: &[Match],
    members: &[usize],
    git: &GitStates,
) -> Vec<(usize, Option<usize>)> {
    let flags: Vec<(usize, bool)> = members
        .iter()
        .map(|&i| (i, is_nested(matches, members, git, i)))
        .collect();
    let tops: Vec<usize> = flags
        .iter()
        .filter_map(|&(i, nested)| (!nested).then_some(i))
        .collect();
    let mut ordered: Vec<(usize, Option<usize>)> = vec![];
    let mut attached = vec![false; matches.len()];
    for &top in &tops {
        ordered.push((top, None));
        let top_root = match_root(git, &matches[top]);
        let first = tops[..tops.iter().position(|&u| u == top).unwrap_or(0)]
            .iter()
            .any(|&u| match_root(git, &matches[u]) == top_root);
        if first {
            continue;
        }
        for &(i, nested) in &flags {
            if nested
                && !attached[i]
                && match_primary(git, &matches[i]) == top_root
            {
                ordered.push((i, Some(top)));
                attached[i] = true;
            }
        }
    }
    for &(i, nested) in &flags {
        if nested && !attached[i] {
            ordered.push((i, None));
        }
    }
    ordered
}

/// Remainder-hit children in `matches()` rank order, with nested
/// worktrees tucked under their primary even when that disagrees
/// with rank. Nested entries are skipped at their own rank slot.
fn remainder_rank_order(
    matches: &[Match],
    keys: &[String],
    remainder_of: &HashMap<String, Vec<usize>>,
    git: &GitStates,
) -> Vec<(usize, Option<usize>)> {
    let rem: HashSet<usize> =
        remainder_of.values().flatten().copied().collect();
    let nested_at = |i: usize| {
        remainder_of
            .get(&keys[i])
            .is_some_and(|members| is_nested(matches, members, git, i))
    };
    let mut tops_of: HashMap<String, Vec<usize>> = HashMap::new();
    for i in 0..matches.len() {
        if rem.contains(&i) && !nested_at(i) {
            tops_of.entry(keys[i].clone()).or_default().push(i);
        }
    }
    let mut ordered = vec![];
    let mut attached = vec![false; matches.len()];
    for i in 0..matches.len() {
        if !rem.contains(&i) || nested_at(i) {
            continue;
        }
        ordered.push((i, None));
        let top_root = match_root(git, &matches[i]);
        let tops = &tops_of[&keys[i]];
        let first = tops[..tops.iter().position(|&u| u == i).unwrap_or(0)]
            .iter()
            .any(|&u| match_root(git, &matches[u]) == top_root);
        if first {
            continue;
        }
        if let Some(members) = remainder_of.get(&keys[i]) {
            for &j in members {
                if nested_at(j)
                    && !attached[j]
                    && match_primary(git, &matches[j]) == top_root
                {
                    ordered.push((j, Some(i)));
                    attached[j] = true;
                }
            }
        }
    }
    for i in 0..matches.len() {
        if rem.contains(&i) && nested_at(i) && !attached[i] {
            ordered.push((i, None));
        }
    }
    ordered
}

/// Consecutive runs of the same group key over a rank-ordered
/// remainder list. The same key may appear more than once when
/// other groups interleave.
fn split_group_runs(
    keys: &[String],
    ordered: Vec<(usize, Option<usize>)>,
) -> Vec<(String, Vec<(usize, Option<usize>)>)> {
    let mut runs: Vec<(String, Vec<(usize, Option<usize>)>)> = vec![];
    for (idx, parent) in ordered {
        let key = keys[idx].clone();
        match runs.last_mut() {
            Some((k, kids)) if *k == key => kids.push((idx, parent)),
            _ => runs.push((key, vec![(idx, parent)])),
        }
    }
    runs
}

/// Tree topology (`last` / `parent_last`) for one visible run.
fn place_ordered_kids<'a>(
    matches: &'a [Match],
    keys: &[String],
    ordered: Vec<(usize, Option<usize>)>,
) -> Vec<PlacedChild<'a>> {
    let top_count = ordered.iter().filter(|(_, p)| p.is_none()).count();
    let last_top = ordered
        .iter()
        .filter_map(|(i, p)| p.is_none().then_some(*i))
        .last();
    let mut nest_total: HashMap<usize, usize> = HashMap::new();
    for (_, parent) in &ordered {
        if let Some(p) = parent {
            *nest_total.entry(*p).or_insert(0) += 1;
        }
    }
    let mut nest_seen: HashMap<usize, usize> = HashMap::new();
    let mut top_seen = 0;
    let mut kids = vec![];
    for (idx, parent) in ordered {
        let m = &matches[idx];
        let group = keys[idx].clone();
        let remainder = group_remainder(&m.entry, &group).to_string();
        match parent {
            None => {
                let last = top_seen + 1 == top_count;
                top_seen += 1;
                kids.push(PlacedChild {
                    m,
                    match_idx: idx,
                    remainder,
                    nested: false,
                    last,
                    parent_last: false,
                });
            }
            Some(p) => {
                let seen = nest_seen.entry(p).or_insert(0);
                *seen += 1;
                let last = *seen == nest_total.get(&p).copied().unwrap_or(1);
                kids.push(PlacedChild {
                    m,
                    match_idx: idx,
                    remainder,
                    nested: true,
                    last,
                    parent_last: Some(p) == last_top,
                });
            }
        }
    }
    kids
}

/// Visible matches bucketed by config group. Selection and
/// fuzzy keep ranking the full candidate path; this is
/// presentation over the current matches, so empty groups
/// never appear. Under an empty query, groups sequence by
/// `group_order` — first-seen resolved-candidate order — with
/// unseen keys appended in first-seen match order. Under a
/// non-empty query, remainder-hit children (tier 1) follow
/// `matches()` rank order globally; a group header introduces
/// each consecutive run of the same group key, so a group may
/// appear more than once when rank interleaves other groups.
/// Prefix-only groups (tier 2) keep every matching child for
/// the folded `(N)`, sort groups by best member then
/// `group_order` after every remainder-hit run, and list
/// children in catalog / first-seen order (not fuzzy rank).
/// Prefix-only buckets whose key equals `home_discovery_group`
/// are dropped (remainder-hit and empty query keep that group).
/// Linked worktrees still tuck under their primary checkout
/// even when that disagrees with rank, and are skipped at their
/// own rank slot; linked orphans whose main is not a current
/// match stay flat. A match missing from the group map falls
/// back to its own path as group, showing its basename.
fn place_children<'a>(
    matches: &'a [Match],
    groups: &HashMap<String, String>,
    git: &GitStates,
    group_order: &[String],
    catalog: &[String],
    home_discovery_group: Option<&str>,
    query_empty: bool,
) -> Vec<(String, Vec<PlacedChild<'a>>)> {
    let keys: Vec<String> = matches
        .iter()
        .map(|m| {
            groups
                .get(&m.entry)
                .cloned()
                .unwrap_or_else(|| m.entry.clone())
        })
        .collect();
    let mut out = vec![];
    if query_empty {
        let mut members_of: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, key) in keys.iter().enumerate() {
            members_of.entry(key.clone()).or_default().push(i);
        }
        let mut order: Vec<String> = members_of.keys().cloned().collect();
        order.sort_by(|a, b| {
            group_order_pos(group_order, a)
                .cmp(&group_order_pos(group_order, b))
                .then_with(|| members_of[a][0].cmp(&members_of[b][0]))
        });
        for key in order {
            let ordered = nest_order(matches, &members_of[&key], git);
            out.push((key, place_ordered_kids(matches, &keys, ordered)));
        }
        return out;
    }
    let mut remainder_of: HashMap<String, Vec<usize>> = HashMap::new();
    let mut prefix_of: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, key) in keys.iter().enumerate() {
        let tail = group_remainder(&matches[i].entry, key);
        if remainder_carries_hit(
            &matches[i].entry,
            &matches[i].match_ranges,
            tail,
        ) {
            remainder_of.entry(key.clone()).or_default().push(i);
        } else {
            prefix_of.entry(key.clone()).or_default().push(i);
        }
    }
    prefix_of.retain(|k, _| !remainder_of.contains_key(k));
    if let Some(skip) = home_discovery_group {
        prefix_of.remove(skip);
    }
    let rem_ordered = remainder_rank_order(matches, &keys, &remainder_of, git);
    for (key, ordered) in split_group_runs(&keys, rem_ordered) {
        out.push((key, place_ordered_kids(matches, &keys, ordered)));
    }
    let mut prefix_order: Vec<String> = prefix_of.keys().cloned().collect();
    prefix_order.sort_by(|a, b| {
        prefix_of[a][0].cmp(&prefix_of[b][0]).then_with(|| {
            group_order_pos(group_order, a)
                .cmp(&group_order_pos(group_order, b))
        })
    });
    for members in prefix_of.values_mut() {
        members.sort_by_key(|&i| {
            catalog
                .iter()
                .position(|c| c == &matches[i].entry)
                .unwrap_or(usize::MAX)
        });
    }
    for key in prefix_order {
        let ordered = nest_order(matches, &prefix_of[&key], git);
        out.push((key, place_ordered_kids(matches, &keys, ordered)));
    }
    out
}

/// Visible stops in navigation order: folded headers plus
/// children of unfolded groups, or plain match order without
/// the group map. Unfolded headers stay off this walk, so
/// Up/Down land on a folded header or a child, never a blank
/// and never an open header. Header indices are into
/// `group_order`.
pub(crate) fn visual_entries(
    sel: &Selection,
    state: &SessionsListState,
) -> Vec<VisualTarget> {
    if state.groups.is_empty() {
        return sel
            .matches()
            .iter()
            .map(|m| VisualTarget::Child(m.entry.clone()))
            .collect();
    }
    let mut out = vec![];
    for (key, kids) in place_children(
        sel.matches(),
        &state.groups,
        &state.git,
        &state.group_order,
        sel.candidates(),
        state.home_discovery_group.as_deref(),
        sel.query().is_empty(),
    ) {
        if state.folded.contains(&key) {
            if let Some(idx) = state.group_order.iter().position(|g| g == &key)
            {
                out.push(VisualTarget::Header(idx));
            }
            continue;
        }
        out.extend(
            kids.into_iter()
                .map(|kid| VisualTarget::Child(kid.m.entry.clone())),
        );
    }
    out
}

/// Deliberate visual motion over grouped children. Neighbors
/// are visual, not fuzzy-rank: after a reorder the next row
/// on screen wins over the next ranked match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualMotion {
    Prev,
    Next,
    First,
    Last,
}

/// One stop on the grouped visual walk: a child candidate
/// (still a `Selection` match) or a folded header (never a
/// match, never activated). Unfolded headers stay off this
/// walk, matching today's skip-headers navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VisualTarget {
    Child(String),
    Header(usize),
}

/// Apply one visual motion by landing on the neighbor stop.
/// Folded headers set `active_header`; children call
/// `select_entry` and clear it. No wrap: moving past either
/// end, or moving with no stops, leaves the focus unchanged.
pub(crate) fn apply_visual_motion(
    sel: &mut Selection,
    state: &mut SessionsListState,
    motion: VisualMotion,
) -> Option<VisualTarget> {
    let entries = visual_entries(sel, state);
    if entries.is_empty() {
        return None;
    }
    let current = match state.active_header {
        Some(idx) => Some(VisualTarget::Header(idx)),
        None => sel
            .matches()
            .get(sel.selected_line().saturating_sub(1))
            .map(|m| VisualTarget::Child(m.entry.clone())),
    };
    let pos = current.and_then(|c| entries.iter().position(|x| *x == c));
    let target = match motion {
        VisualMotion::Prev => pos.map(|p| p.saturating_sub(1)),
        VisualMotion::Next => pos.map(|p| (p + 1).min(entries.len() - 1)),
        VisualMotion::First => Some(0),
        VisualMotion::Last => entries.len().checked_sub(1),
    };
    let landed = target.and_then(|p| entries.get(p).cloned())?;
    match &landed {
        VisualTarget::Child(entry) => {
            state.active_header = None;
            sel.select_entry(entry);
        }
        VisualTarget::Header(idx) => {
            state.active_header = Some(*idx);
        }
    }
    Some(landed)
}

/// Rebuild `folded` from the current query. Empty query folds
/// every group and clears header focus (no First; startup still
/// Firsts). A non-empty query unfolds remainder-hit groups only,
/// keeps prefix-only matching groups folded, and focuses the
/// first visual stop: the first remainder child if tier 1 is
/// nonempty, else the first prefix-only header. Empty groups
/// stay hidden by placement. Manual expand/collapse is not kept
/// across query changes. Callers must invoke this only when the
/// query itself changed — never after Up/Down — so
/// `active_header` survives motion.
pub(crate) fn apply_query_folds(
    state: &mut SessionsListState,
    sel: &mut Selection,
) {
    let mut all: HashSet<String> = state.group_order.iter().cloned().collect();
    all.extend(state.groups.values().cloned());
    if sel.query().is_empty() {
        state.folded = all;
        state.active_header = None;
        return;
    }
    let remainder: HashSet<String> = sel
        .matches()
        .iter()
        .filter_map(|m| {
            let key = state
                .groups
                .get(&m.entry)
                .cloned()
                .unwrap_or_else(|| m.entry.clone());
            let tail = group_remainder(&m.entry, &key);
            remainder_carries_hit(&m.entry, &m.match_ranges, tail)
                .then_some(key)
        })
        .collect();
    state.folded = all.into_iter().filter(|g| !remainder.contains(g)).collect();
    state.active_header = None;
    apply_visual_motion(sel, state, VisualMotion::First);
}

/// Unfold the focused folded header and select its first child.
/// No-op when no header is focused.
pub(crate) fn expand_group(state: &mut SessionsListState, sel: &mut Selection) {
    let Some(idx) = state.active_header else {
        return;
    };
    let Some(key) = state.group_order.get(idx).cloned() else {
        return;
    };
    state.folded.remove(&key);
    state.active_header = None;
    let first = visual_entries(sel, state)
        .into_iter()
        .find_map(|t| match t {
            VisualTarget::Child(entry) => {
                let g = state
                    .groups
                    .get(&entry)
                    .map(String::as_str)
                    .unwrap_or(entry.as_str());
                (g == key).then_some(entry)
            }
            VisualTarget::Header(_) => None,
        });
    if let Some(entry) = first {
        sel.select_entry(&entry);
    }
}

/// Fold `group_key` and land focus on that header when it is
/// in `group_order`.
pub(crate) fn collapse_group(state: &mut SessionsListState, group_key: &str) {
    state.folded.insert(group_key.to_string());
    if let Some(idx) = state.group_order.iter().position(|g| g == group_key) {
        state.active_header = Some(idx);
    }
}

/// Comment-colored tree glyph on the row background, so the
/// selection tint still spans the full width underneath.
fn tree_span(
    glyph: &'static str,
    theme: Theme,
    bg: ratatui::style::Color,
) -> Span<'static> {
    Span::styled(glyph.to_string(), Style::new().fg(theme.comment).bg(bg))
}

/// Tree prefix for one child row: muted box-drawing in the
/// indent columns (`├`/`└` on the path row, `│` continuing
/// through the git row), with a second column under a nested
/// main. Headers never call this. Widths match the B indents
/// (4/6 flat path/git, 6/8 nested path/git).
fn tree_prefix(
    nested: bool,
    last: bool,
    parent_last: bool,
    path_row: bool,
    theme: Theme,
    bg: ratatui::style::Color,
) -> (Vec<Span<'static>>, usize) {
    let spaces = |n: usize| Span::styled(" ".repeat(n), Style::new().bg(bg));
    if !nested {
        if path_row {
            (
                vec![
                    spaces(2),
                    tree_span(
                        if last { "\u{2514}" } else { "\u{251c}" },
                        theme,
                        bg,
                    ),
                    spaces(1),
                ],
                CHILD_PATH_INDENT,
            )
        } else if last {
            (vec![spaces(CHILD_GIT_INDENT)], CHILD_GIT_INDENT)
        } else {
            (
                vec![spaces(2), tree_span("\u{2502}", theme, bg), spaces(3)],
                CHILD_GIT_INDENT,
            )
        }
    } else if path_row {
        let mut prefix = vec![spaces(2)];
        prefix.push(if parent_last {
            spaces(1)
        } else {
            tree_span("\u{2502}", theme, bg)
        });
        prefix.push(spaces(1));
        prefix.push(tree_span(
            if last { "\u{2514}" } else { "\u{251c}" },
            theme,
            bg,
        ));
        prefix.push(spaces(1));
        (prefix, NEST_PATH_INDENT)
    } else {
        let mut prefix = vec![spaces(2)];
        prefix.push(if parent_last {
            spaces(1)
        } else {
            tree_span("\u{2502}", theme, bg)
        });
        prefix.push(spaces(1));
        prefix.push(if last {
            spaces(1)
        } else {
            tree_span("\u{2502}", theme, bg)
        });
        prefix.push(spaces(3));
        (prefix, NEST_GIT_INDENT)
    }
}

/// Remainder-only path row with a tree prefix, padded so the
/// selection tint covers the full width when selected.
fn child_path_line(
    prefix: Vec<Span<'static>>,
    prefix_w: usize,
    text: &str,
    hits: &[usize],
    selected: bool,
    width: usize,
    theme: Theme,
) -> Line<'static> {
    let bg = if selected { theme.bg_alt } else { theme.bg };
    let plain = Style::new().fg(theme.fg).bg(bg);
    let hit = Style::new().fg(theme.accent).bg(bg);
    let mut row = prefix;
    row.extend(path_spans(text, hits, plain, hit));
    let pad = width.saturating_sub(prefix_w + text.chars().count());
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(row).style(Style::new().bg(bg).fg(theme.fg))
}

/// Git identity row with a tree prefix, padded like the path
/// row so the selection tint covers the full width.
fn child_git_line(
    prefix: Vec<Span<'static>>,
    prefix_w: usize,
    git: Option<&CandidateState>,
    selected: bool,
    width: usize,
    theme: Theme,
) -> Line<'static> {
    let bg = if selected { theme.bg_alt } else { theme.bg };
    let mut row = prefix;
    let content = git_spans(git, width.saturating_sub(prefix_w), theme);
    let pad = width.saturating_sub(prefix_w + status_width(&content));
    for mut span in content {
        span.style = span.style.bg(bg);
        row.push(span);
    }
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(row).style(Style::new().bg(bg).fg(theme.fg))
}

/// Full-width blank row on the base background: the group
/// separator, never tinted, never selectable.
fn blank_line(width: usize, theme: Theme) -> Line<'static> {
    let line_style = Style::new().bg(theme.bg).fg(theme.fg);
    if width == 0 {
        return Line::from(vec![]).style(line_style);
    }
    Line::from(vec![Span::styled(
        " ".repeat(width),
        Style::new().bg(theme.bg),
    )])
    .style(line_style)
}

/// Group header row: fold glyph plus the home-abbreviated
/// parent in comment color, with fuzzy hits that still sit on
/// that abbreviated text painted accent. `ranges` are byte
/// offsets into a member entry; prefix hits map onto the group
/// key, remainder and collapsed `$HOME` hits drop. Folded
/// headers add a `(N)` match count and take the selection tint
/// when focused; unfolded headers stay untinted and unselectable.
fn header_line(
    group: &str,
    home: Option<&str>,
    width: usize,
    theme: Theme,
    folded: bool,
    selected: bool,
    count: usize,
    ranges: &[(usize, usize)],
) -> Line<'static> {
    let bg = if selected { theme.bg_alt } else { theme.bg };
    let line_style = Style::new().bg(bg).fg(theme.fg);
    if width == 0 {
        return Line::from(vec![]).style(line_style);
    }
    let comment = Style::new().fg(theme.comment).bg(bg);
    let hit = Style::new().fg(theme.accent).bg(bg);
    let glyph = if folded { "\u{25b8}" } else { "\u{25be}" };
    let badge = if folded {
        format!(" ({count})")
    } else {
        String::new()
    };
    // indent + glyph + space, then the badge after the name.
    let fixed = HEADER_INDENT + 2 + badge.chars().count();
    let (shown, hits) = highlighted_path(
        group,
        ranges,
        home,
        width.saturating_sub(fixed).max(1),
    );
    let mut row = vec![Span::styled(
        " ".repeat(HEADER_INDENT.min(width)),
        Style::new().bg(bg),
    )];
    if width > HEADER_INDENT {
        row.push(Span::styled(glyph.to_string(), comment));
    }
    if width > HEADER_INDENT + 1 {
        row.push(Span::styled(" ", Style::new().bg(bg)));
    }
    row.extend(path_spans(&shown, &hits, comment, hit));
    if !badge.is_empty() {
        row.push(Span::styled(badge.clone(), comment));
    }
    let used =
        HEADER_INDENT + 2 + shown.chars().count() + badge.chars().count();
    let pad = width.saturating_sub(used);
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(row).style(line_style)
}

/// One full-width two-line item: path first with a consistent
/// indent (no selection marker arrow), Git metadata indented
/// underneath. Both lines are padded to the full width so the
/// selection tint covers the whole row. Fuzzy hits surviving
/// abbreviation and truncation paint yellow; anything dropped
/// paints plain. Budgets are content cells after the indent: the
/// path keeps the full remaining width and the Git line compacts
/// inside its own.
fn render_item(
    x: u16,
    y: u16,
    width: usize,
    buf: &mut Buffer,
    m: &Match,
    git: Option<&CandidateState>,
    home: Option<&str>,
    selected: bool,
    theme: Theme,
) {
    if width == 0 {
        return;
    }
    let bg = if selected { theme.bg_alt } else { theme.bg };
    let line_style = Style::new().bg(bg).fg(theme.fg);
    let path_w = width.saturating_sub(PATH_INDENT).max(1);
    let (path, hits) =
        highlighted_path(&m.entry, &m.match_ranges, home, path_w);
    let plain = Style::new().fg(theme.fg).bg(bg);
    let hit = Style::new().fg(theme.accent).bg(bg);
    let mut path_row =
        vec![Span::styled(" ".repeat(PATH_INDENT), Style::new().bg(bg))];
    path_row.extend(path_spans(&path, &hits, plain, hit));
    let pad = width.saturating_sub(PATH_INDENT + path.chars().count());
    if pad > 0 {
        path_row.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
    }
    Line::from(path_row)
        .style(line_style)
        .render(Rect::new(x, y, width as u16, 1), buf);
    let mut git_row =
        vec![Span::styled(" ".repeat(GIT_INDENT), Style::new().bg(bg))];
    let budget = width.saturating_sub(GIT_INDENT);
    let content = git_spans(git, budget, theme);
    let git_pad = width.saturating_sub(GIT_INDENT + status_width(&content));
    for mut span in content {
        span.style = span.style.bg(bg);
        git_row.push(span);
    }
    if git_pad > 0 {
        git_row.push(Span::styled(" ".repeat(git_pad), Style::new().bg(bg)));
    }
    Line::from(git_row)
        .style(line_style)
        .render(Rect::new(x, y.saturating_add(1), width as u16, 1), buf);
}

/// Empty-list copy: an empty catalog vs a filter miss. Comment
/// color at the git indent, padded to the full width on the base
/// background like the item rows.
fn render_empty(area: Rect, buf: &mut Buffer, sel: &Selection, theme: Theme) {
    let width = area.width as usize;
    if width == 0 {
        return;
    }
    let copy = if sel.has_candidates() {
        format!("no matches for \"{}\"", sel.query())
    } else {
        "no session candidates".to_string()
    };
    let mut spans = vec![Span::styled(
        " ".repeat(GIT_INDENT.min(width)),
        Style::new().bg(theme.bg),
    )];
    spans.push(Span::styled(
        copy,
        Style::new().fg(theme.comment).bg(theme.bg),
    ));
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = width.saturating_sub(used);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::new().bg(theme.bg)));
    }
    Line::from(spans)
        .style(Style::new().bg(theme.bg).fg(theme.fg))
        .render(Rect::new(area.x, area.y, area.width, 1), buf);
}

impl StatefulWidget for &Selection {
    type State = SessionsListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Grouped presentation needs the group map; without it
        // the list stays the legacy flat two-line column.
        if !state.groups.is_empty() {
            render_grouped(self, area, buf, state);
            return;
        }
        // Single full-width column: Left/Right are no-ops while
        // Up/Down move the selected row linearly.
        let n = self.matches().len();
        let rows_visible = (area.height / ITEM_ROWS) as usize;
        let sel_idx = self.selected_line().saturating_sub(1);
        state.update_scroll(rows_visible, n, sel_idx);
        if rows_visible == 0 {
            return;
        }
        let theme = Theme::get(state.theme_mode);
        if n == 0 {
            render_empty(area, buf, self, theme);
            return;
        }
        let width = area.width as usize;
        for r in 0..rows_visible {
            let pos = state.scroll + r;
            if pos >= n {
                break;
            }
            let y = area.y.saturating_add((r as u16).saturating_mul(ITEM_ROWS));
            let m = &self.matches()[pos];
            render_item(
                area.x,
                y,
                width,
                buf,
                m,
                state.git.get(&m.entry),
                state.home.as_deref(),
                pos + 1 == self.selected_line(),
                theme,
            );
        }
    }
}

/// Grouped two-line list: config-group headers plus
/// remainder-only children with a tree spine, laid out as
/// full-width rows and windowed by row offset. Selection and
/// fuzzy still rank the full candidate path; headers never
/// enter `Selection`, so activation keeps the full entry.
fn render_grouped(
    sel: &Selection,
    area: Rect,
    buf: &mut Buffer,
    state: &mut SessionsListState,
) {
    let theme = Theme::get(state.theme_mode);
    let width = area.width as usize;
    let list_h = area.height as usize;
    let matches = sel.matches();
    if matches.is_empty() {
        state.scroll = 0;
        render_empty(area, buf, sel, theme);
        return;
    }
    let placed = place_children(
        matches,
        &state.groups,
        &state.git,
        &state.group_order,
        sel.candidates(),
        state.home_discovery_group.as_deref(),
        sel.query().is_empty(),
    );
    let sel_idx = sel.selected_line().saturating_sub(1);
    let home = state.home.as_deref();
    let mut lines: Vec<Line<'static>> = vec![];
    let (mut sel_start, mut sel_end) = (0, 0);
    for (gi, (group, kids)) in placed.iter().enumerate() {
        if gi > 0 {
            lines.push(blank_line(width, theme));
        }
        let folded = state.folded.contains(group);
        let header_idx = state.group_order.iter().position(|g| g == group);
        let header_selected = folded && state.active_header == header_idx;
        if header_selected {
            sel_start = lines.len();
        }
        let ranges = kids
            .first()
            .map(|kid| kid.m.match_ranges.as_slice())
            .unwrap_or(&[]);
        lines.push(header_line(
            group,
            home,
            width,
            theme,
            folded,
            header_selected,
            kids.len(),
            ranges,
        ));
        if header_selected {
            sel_end = lines.len();
        }
        if folded {
            continue;
        }
        for kid in kids {
            let selected =
                state.active_header.is_none() && kid.match_idx == sel_idx;
            let bg = if selected { theme.bg_alt } else { theme.bg };
            let tail = if kid.nested {
                work_basename(&kid.remainder)
            } else {
                kid.remainder.as_str()
            };
            let (prefix, prefix_w) = tree_prefix(
                kid.nested,
                kid.last,
                kid.parent_last,
                true,
                theme,
                bg,
            );
            let (text, hits) = highlighted_remainder(
                &kid.m.entry,
                &kid.m.match_ranges,
                tail,
                width.saturating_sub(prefix_w).max(1),
            );
            if selected {
                sel_start = lines.len();
            }
            lines.push(child_path_line(
                prefix, prefix_w, &text, &hits, selected, width, theme,
            ));
            let (gprefix, gprefix_w) = tree_prefix(
                kid.nested,
                kid.last,
                kid.parent_last,
                false,
                theme,
                bg,
            );
            lines.push(child_git_line(
                gprefix,
                gprefix_w,
                state.git.get(&kid.m.entry),
                selected,
                width,
                theme,
            ));
            if selected {
                sel_end = lines.len();
            }
        }
    }
    state.update_scroll_rows(sel_start, sel_end, list_h, lines.len());
    for (r, line) in lines
        .into_iter()
        .skip(state.scroll)
        .take(list_h)
        .enumerate()
    {
        line.render(Rect::new(area.x, area.y + r as u16, width as u16, 1), buf);
    }
}

#[cfg(test)]
#[path = "sessions_list_tests.rs"]
mod tests;
