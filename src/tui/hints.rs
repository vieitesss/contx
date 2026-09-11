use crate::tui::action::CancelState;

/// One shortcut chip on the persistent hint bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HintChip {
    pub label: &'static str,
    pub enabled: bool,
    pub selected: bool,
}

impl HintChip {
    /// Key and action wording split at the first space, matching
    /// the badge renderer.
    fn key_and_action(&self) -> Option<(&'static str, &'static str)> {
        self.label.split_once(' ')
    }

    /// Display width as rendered: the key badge pads one space on
    /// each side of the key, then the action label follows.
    pub(crate) fn width(&self) -> usize {
        match self.key_and_action() {
            Some((key, action)) => {
                key.chars().count() + action.chars().count() + 2
            }
            None => self.label.chars().count() + 2,
        }
    }

    /// Plain text equivalent of the badge spans, used for packing
    /// and truncation accounting.
    fn painted(&self) -> String {
        match self.key_and_action() {
            Some((key, action)) => format!(" {key} {action}"),
            None => format!(" {} ", self.label),
        }
    }
}

fn on(label: &'static str) -> HintChip {
    HintChip {
        label,
        enabled: true,
        selected: false,
    }
}

fn off(label: &'static str) -> HintChip {
    HintChip {
        label,
        enabled: false,
        selected: false,
    }
}

/// Surface whose enabled shortcuts feed the persistent hint bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HintSurface {
    PickerIdle {
        expand: bool,
        collapse: bool,
    },
    PickerPrefix {
        delete_enabled: bool,
        selected: Option<usize>,
    },
    Dialog(DialogHintState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DialogHintState {
    pub running: bool,
    pub cancel: CancelState,
    pub item_count: usize,
    pub sticky: bool,
    pub awaiting: DialogAwaiting,
    pub typing: bool,
    pub inspect_enabled: bool,
    pub add_parent_focused: bool,
    /// A catalog refresh is still in flight: Escape and
    /// Acknowledge wait for it, so the bar never advertises a
    /// cancel that would drop the pending picker update.
    pub refresh_pending: bool,
}

impl Default for DialogHintState {
    fn default() -> Self {
        Self {
            running: false,
            cancel: CancelState::Idle,
            item_count: 0,
            sticky: false,
            awaiting: DialogAwaiting::None,
            typing: false,
            inspect_enabled: false,
            add_parent_focused: false,
            refresh_pending: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogAwaiting {
    None,
    Config,
    Inspect,
    Mutate,
}

/// One packed hint-bar row: complete chips, optional trailing ellipsis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HintRow {
    pub chips: Vec<HintChip>,
    pub ellipsis: bool,
}

impl HintRow {
    pub(crate) fn text(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let raw = match self.chips.len() {
            0 if self.ellipsis => "…".to_string(),
            0 => String::new(),
            1 if self.chips[0].width() > width => {
                return trunc(&self.chips[0].painted(), width);
            }
            _ => {
                let joined = self
                    .chips
                    .iter()
                    .map(HintChip::painted)
                    .collect::<Vec<_>>()
                    .join(" ");
                if self.ellipsis {
                    format!("{joined}…")
                } else {
                    joined
                }
            }
        };
        trunc(&raw, width)
    }
}

/// Pack complete chips into at most `max_rows` rows of `width` cells.
/// Join with one space. Ellipsize a chip only when it is wider than
/// `width`, or the last visible row when more chips remain.
pub(crate) fn format_hint_rows(
    chips: &[HintChip],
    width: usize,
    max_rows: usize,
) -> Vec<HintRow> {
    if max_rows == 0 || width == 0 {
        return Vec::new();
    }
    let unlimited = pack_unlimited(chips, width);
    if unlimited.len() <= max_rows {
        return unlimited
            .into_iter()
            .map(|chips| HintRow {
                chips,
                ellipsis: false,
            })
            .collect();
    }
    let mut rows: Vec<HintRow> = unlimited[..max_rows - 1]
        .iter()
        .cloned()
        .map(|chips| HintRow {
            chips,
            ellipsis: false,
        })
        .collect();
    let rest: Vec<HintChip> = unlimited[max_rows - 1..]
        .iter()
        .flatten()
        .copied()
        .collect();
    rows.push(pack_last(&rest, width));
    rows
}

fn pack_unlimited(chips: &[HintChip], width: usize) -> Vec<Vec<HintChip>> {
    let mut rows: Vec<Vec<HintChip>> = Vec::new();
    let mut cur: Vec<HintChip> = Vec::new();
    let mut used = 0usize;
    for &chip in chips {
        let n = chip.width();
        if n > width {
            if !cur.is_empty() {
                rows.push(std::mem::take(&mut cur));
                used = 0;
            }
            rows.push(vec![chip]);
            continue;
        }
        let sep = if cur.is_empty() { 0 } else { 1 };
        if used + sep + n <= width {
            cur.push(chip);
            used += sep + n;
        } else {
            if !cur.is_empty() {
                rows.push(std::mem::take(&mut cur));
            }
            cur.push(chip);
            used = n;
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    rows
}

fn pack_last(chips: &[HintChip], width: usize) -> HintRow {
    if chips.is_empty() {
        return HintRow {
            chips: Vec::new(),
            ellipsis: false,
        };
    }
    let mut fitted = Vec::new();
    let mut used = 0usize;
    for (idx, &chip) in chips.iter().enumerate() {
        let n = chip.width();
        let more = idx + 1 < chips.len();
        if n > width {
            if fitted.is_empty() {
                return HintRow {
                    chips: vec![chip],
                    ellipsis: more,
                };
            }
            break;
        }
        let sep = if fitted.is_empty() { 0 } else { 1 };
        let extra = if more { 1 } else { 0 };
        if used + sep + n + extra <= width {
            fitted.push(chip);
            used += sep + n;
        } else {
            break;
        }
    }
    if fitted.is_empty() {
        HintRow {
            chips: vec![chips[0]],
            ellipsis: chips.len() > 1,
        }
    } else {
        HintRow {
            ellipsis: fitted.len() < chips.len(),
            chips: fitted,
        }
    }
}

fn trunc(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

/// Nonobvious shortcuts for `surface`. Arrows, Enter, and typing
/// are omitted. Labels match keys `handle_key` actually honors.
pub(crate) fn shortcut_hints(surface: HintSurface) -> Vec<HintChip> {
    match surface {
        HintSurface::PickerIdle { expand, collapse } => {
            idle_hints(expand, collapse)
        }
        HintSurface::PickerPrefix {
            delete_enabled,
            selected,
        } => prefix_hints(delete_enabled, selected),
        HintSurface::Dialog(dialog) => dialog_hints(dialog),
    }
}

fn idle_hints(expand: bool, collapse: bool) -> Vec<HintChip> {
    let mut chips = vec![
        on("Ctrl-X Actions"),
        on("Ctrl-J/K Move"),
        on("Ctrl-T First"),
        on("Ctrl-G/B Last"),
        on("Ctrl-W/Alt-BS Word"),
        on("Ctrl-C Quit"),
    ];
    if expand {
        chips.push(on("Space Expand"));
    }
    if collapse {
        chips.push(on("Left Collapse"));
    }
    chips
}

fn prefix_hints(
    delete_enabled: bool,
    selected: Option<usize>,
) -> Vec<HintChip> {
    vec![
        HintChip {
            label: "c Clone",
            enabled: true,
            selected: selected == Some(0),
        },
        HintChip {
            label: "n New directory",
            enabled: true,
            selected: selected == Some(1),
        },
        HintChip {
            label: "d Delete",
            enabled: delete_enabled,
            selected: delete_enabled && selected == Some(2),
        },
        on("Esc/Ctrl-X Cancel"),
        on("Ctrl-C Quit"),
    ]
}

fn dialog_hints(dialog: DialogHintState) -> Vec<HintChip> {
    let mut chips = Vec::new();
    if dialog.item_count >= 2 {
        chips.push(on("Tab Next"));
        chips.push(on("Shift-Tab Prev"));
    }
    if esc_cancels(dialog) {
        chips.push(on("Esc Cancel"));
    }
    if dialog.running && dialog.cancel == CancelState::Idle {
        chips.push(on("Ctrl-G Cancel"));
    }
    if !dialog.typing {
        chips.push(on("[/] Stage"));
        let inspect = if dialog.add_parent_focused {
            "i Inspect"
        } else {
            "i/Space Inspect"
        };
        if dialog.inspect_enabled {
            chips.push(on(inspect));
        } else {
            chips.push(off(inspect));
        }
        if dialog.add_parent_focused {
            chips.push(on("Space Toggle"));
        }
    }
    chips.push(on("Ctrl-C Quit"));
    chips
}

fn esc_cancels(dialog: DialogHintState) -> bool {
    if dialog.sticky || dialog.refresh_pending {
        return false;
    }
    match dialog.awaiting {
        DialogAwaiting::Config | DialogAwaiting::Mutate => false,
        DialogAwaiting::Inspect => true,
        DialogAwaiting::None => !dialog.running,
    }
}

#[cfg(test)]
#[path = "hints_tests.rs"]
mod tests;
