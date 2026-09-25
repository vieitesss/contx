//! Spine Accordion overlay (prototype A).

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Padding, Paragraph, Widget},
};

use crate::theme::Theme;

use super::prompt::mask_secret;
use super::state::{CloneAuth, CloneKind, DeleteConfirm, DeleteStage};
use super::{ActionDialog, CancelState, DialogOutcome, FocusItem, Op};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Success,
    Cancel,
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

fn center(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn fill(
    spans: Vec<Span<'static>>,
    width: usize,
    bg: ratatui::style::Color,
) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let mut spans = spans;
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), Style::new().bg(bg)));
    }
    Line::from(spans).style(Style::new().bg(bg))
}

fn st(fg: ratatui::style::Color, bg: ratatui::style::Color) -> Style {
    Style::new().fg(fg).bg(bg)
}

fn put_lines(buf: &mut Buffer, area: Rect, lines: Vec<Line>) {
    for (i, line) in lines.into_iter().take(area.height as usize).enumerate() {
        line.render(Rect::new(area.x, area.y + i as u16, area.width, 1), buf);
    }
}

/// Floating dialog rect: ~70% width, A height 76% capped at 82%, margin.
pub(crate) fn modal_rect(area: Rect) -> Rect {
    let mx = 4u16.min(area.width / 8).max(2);
    let w = (area.width.saturating_mul(70) / 100)
        .max(52)
        .min(area.width.saturating_sub(mx.saturating_mul(2)));
    let cap = (area.height.saturating_mul(82) / 100)
        .min(area.height.saturating_sub(2));
    let h = (area.height.saturating_mul(76) / 100)
        .max(16)
        .min(cap)
        .max(12.min(cap));
    center(area, w, h)
}

pub(crate) fn render_dialog(
    dialog: &ActionDialog,
    area: Rect,
    buf: &mut Buffer,
    t: Theme,
) {
    let rect = modal_rect(area);
    let title = match &dialog.op {
        Op::Clone { form, .. } if form.kind == CloneKind::Directory => {
            " New directory "
        }
        Op::Clone { .. } => " Clone ",
        Op::Delete { .. } => " Delete ",
    };
    let border_fg = if dialog_has_error(dialog) {
        t.red
    } else if dialog.running() {
        t.accent
    } else {
        t.git_icon
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Double)
        .border_style(st(border_fg, t.bg))
        .style(st(t.fg, t.bg))
        .padding(Padding::horizontal(1));
    let inner = block.inner(rect);
    Clear.render(rect, buf);
    block.render(rect, buf);
    render_a(dialog, inner, buf, t);
}

fn dialog_has_error(dialog: &ActionDialog) -> bool {
    matches!(
        &dialog.outcome,
        Some(DialogOutcome::Failed { .. })
            | Some(DialogOutcome::Completed {
                config_error: Some(_),
                ..
            })
            | Some(DialogOutcome::Completed {
                refresh_error: Some(_),
                ..
            })
    )
}

fn footer_chunks(area: Rect) -> (Rect, Rect, Rect) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    (parts[0], parts[1], parts[2])
}

fn render_a(dialog: &ActionDialog, area: Rect, buf: &mut Buffer, t: Theme) {
    let (body, hint, btns) = footer_chunks(area);
    let lines = accordion_lines(dialog, body.width as usize, t);
    let max_skip = lines.len().saturating_sub(body.height as usize);
    let mut skip = dialog.view_scroll.min(max_skip);
    if dialog.focus_scroll
        && body.height > 0
        && let Some(row) = focused_form_row(dialog)
    {
        if row < skip {
            skip = row;
        } else if row >= skip + body.height as usize {
            skip = (row + 1 - body.height as usize).min(max_skip);
        }
    }
    put_lines(buf, body, lines.into_iter().skip(skip).collect());
    put_lines(buf, hint, vec![hint_line(dialog, hint.width as usize, t)]);
    put_lines(
        buf,
        btns,
        vec![buttons_line(dialog, btns.width as usize, t)],
    );
}

fn focused_form_row(dialog: &ActionDialog) -> Option<usize> {
    let item = dialog.item()?;
    let Op::Clone { form, .. } = &dialog.op else {
        return None;
    };
    if form.kind != CloneKind::Repository || dialog.git_started() {
        return None;
    }

    // The source/destination stage title is row 0 and its form starts at row 1.
    let row = 1 + match item {
        FocusItem::ProtocolSsh | FocusItem::ProtocolHttps => 0,
        FocusItem::Source => 2,
        FocusItem::Dest => 5,
        FocusItem::PresetsToggle => {
            5 + usize::from(dialog.abs_dest().is_ok()) + 1
        }
        FocusItem::SshPrefix => 6 + usize::from(dialog.abs_dest().is_ok()) + 1,
        FocusItem::HttpsPrefix => {
            7 + usize::from(dialog.abs_dest().is_ok()) + 1
        }
        FocusItem::AddParent => {
            8 + usize::from(dialog.abs_dest().is_ok())
                + if form.show_prefixes { 2 } else { 0 }
        }
        _ => return None,
    };
    Some(row)
}

pub(crate) fn render_toast(
    area: Rect,
    buf: &mut Buffer,
    kind: ToastKind,
    msg: &str,
    t: Theme,
) {
    let (mark, fg) = match kind {
        ToastKind::Success => (" ✓  ", t.green),
        ToastKind::Cancel => (" ↷  ", t.accent),
    };
    let text = format!("{mark}{msg}");
    let w =
        ((text.chars().count() as u16) + 4).min(area.width.saturating_sub(2));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y.saturating_add(1),
        width: w,
        height: 3.min(area.height),
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(st(fg, t.bg))
        .style(st(t.fg, t.bg));
    let inner = block.inner(rect);
    Clear.render(rect, buf);
    block.render(rect, buf);
    Paragraph::new(Line::from(vec![Span::styled(
        trunc(&text, inner.width as usize),
        st(fg, t.bg),
    )]))
    .render(inner, buf);
}

fn blank(width: usize, t: Theme) -> Line<'static> {
    fill(vec![], width, t.bg)
}

fn hint_line(dialog: &ActionDialog, width: usize, t: Theme) -> Line<'static> {
    if !dialog.git_started()
        && let Some(err) = dialog.clone_validation_error()
    {
        return fill(
            vec![Span::styled(trunc(&err, width), st(t.red, t.bg))],
            width,
            t.bg,
        );
    }
    let text = if !dialog.hint.is_empty() {
        dialog.hint.clone()
    } else if dialog.running() {
        match dialog.cancel {
            CancelState::Idle => {
                "Ctrl-G requests cancel · Esc does not stop git".into()
            }
            CancelState::Grace => "Cancelling… waiting for git to exit".into(),
            CancelState::ForceReady => {
                "git did not exit · Force Stop is available".into()
            }
        }
    } else if dialog.git_started() {
        "prior inputs locked · [/] then i inspects a completed stage".into()
    } else if repository_source_dest(dialog) {
        source_dest_hint(dialog, width)
    } else {
        "Esc cancels · completed stages reopen with i".into()
    };
    fill(
        vec![Span::styled(trunc(&text, width), st(t.operator, t.bg))],
        width,
        t.bg,
    )
}

fn repository_source_dest(dialog: &ActionDialog) -> bool {
    matches!(
        &dialog.op,
        Op::Clone { form, .. }
            if form.kind == CloneKind::Repository
                && form.stage == super::CloneStage::SourceDest
                && !dialog.git_started()
    )
}

fn source_dest_hint(dialog: &ActionDialog, width: usize) -> String {
    let focus = dialog.item();
    let mut candidates = Vec::with_capacity(6);
    match focus {
        Some(FocusItem::ProtocolSsh | FocusItem::ProtocolHttps) => {
            candidates.push("h/l Switch");
        }
        Some(
            FocusItem::Source
            | FocusItem::Dest
            | FocusItem::SshPrefix
            | FocusItem::HttpsPrefix,
        ) => {
            candidates.push("Ctrl-W Word");
        }
        Some(FocusItem::PresetsToggle | FocusItem::AddParent) => {
            candidates.push("Space Toggle");
        }
        _ => {}
    }
    candidates.extend([
        "Tab Next",
        "Enter Clone",
        "Esc Cancel",
        "Shift-Tab Prev",
    ]);

    let mut line = String::new();
    for candidate in candidates {
        let separator = if line.is_empty() { "" } else { " · " };
        let needed = separator.chars().count() + candidate.chars().count();
        if line.chars().count() + needed > width {
            continue;
        }
        line.push_str(separator);
        line.push_str(candidate);
    }
    if line.is_empty() {
        trunc("Tab Next", width)
    } else {
        line
    }
}

#[derive(Clone, Copy)]
enum BtnKind {
    Primary,
    Danger,
    Ack,
}

fn btn(label: &str, focused: bool, kind: BtnKind, t: Theme) -> Span<'static> {
    let (fg, md) = match (focused, kind) {
        (true, BtnKind::Danger) => (t.red, Modifier::BOLD),
        (true, BtnKind::Primary) => (t.accent, Modifier::BOLD),
        (true, BtnKind::Ack) => (t.accent, Modifier::BOLD),
        (false, BtnKind::Danger) => (t.red, Modifier::empty()),
        (false, _) => (t.fg, Modifier::empty()),
    };
    Span::styled(
        format!("[{label}]"),
        Style::new().fg(fg).bg(t.bg).add_modifier(md),
    )
}

fn buttons_line(
    dialog: &ActionDialog,
    width: usize,
    t: Theme,
) -> Line<'static> {
    let items = dialog.items();
    let focus = dialog.item();
    let mut spans = vec![];
    let mut push = |label: &str, on: bool, kind: BtnKind| {
        if !spans.is_empty() {
            spans.push(Span::styled("  ", st(t.fg, t.bg)));
        }
        spans.push(btn(label, on, kind, t));
    };
    if items.contains(&FocusItem::RejectKey) {
        push(
            "Reject",
            focus == Some(FocusItem::RejectKey),
            BtnKind::Danger,
        );
    }
    if items.contains(&FocusItem::AcceptKey) {
        push(
            "Accept",
            focus == Some(FocusItem::AcceptKey),
            BtnKind::Primary,
        );
    }
    let repository_source_dest = matches!(
        &dialog.op,
        Op::Clone { form, .. }
            if form.kind == CloneKind::Repository
                && form.stage == super::CloneStage::SourceDest
    );
    if items.contains(&FocusItem::Cancel) && !repository_source_dest {
        push("Cancel", focus == Some(FocusItem::Cancel), BtnKind::Primary);
    }
    if items.contains(&FocusItem::RequestCancel) {
        push(
            "Cancel git",
            focus == Some(FocusItem::RequestCancel),
            BtnKind::Primary,
        );
    }
    if items.contains(&FocusItem::ForceStop) {
        push(
            "Force Stop",
            focus == Some(FocusItem::ForceStop),
            BtnKind::Danger,
        );
    }
    if items.contains(&FocusItem::Action) && !repository_source_dest {
        let kind = if let Op::Delete { form } = &dialog.op
            && matches!(
                form.confirm,
                DeleteConfirm::Permanent | DeleteConfirm::Worktree
            )
            && form.stage == DeleteStage::Confirm
        {
            BtnKind::Danger
        } else {
            BtnKind::Primary
        };
        let label = dialog.action_label();
        push(&label, focus == Some(FocusItem::Action), kind);
    }
    if items.contains(&FocusItem::Ack) {
        push("Acknowledge", focus == Some(FocusItem::Ack), BtnKind::Ack);
    }
    fill(spans, width, t.bg)
}

fn accordion_lines(
    dialog: &ActionDialog,
    width: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    let n = dialog.stage_n();
    let cur = dialog.current_stage();
    let mut lines = vec![];
    for i in 0..n {
        if i > 0 {
            lines.push(fill(
                vec![Span::styled(" │", st(t.comment, t.bg))],
                width,
                t.bg,
            ));
        }
        let done = i < cur;
        let current = i == cur;
        let selected = i == dialog.selected_stage;
        let mark = if current {
            '●'
        } else if done {
            '✓'
        } else {
            '○'
        };
        let mark_fg = if current {
            t.accent
        } else if done {
            t.green
        } else {
            t.comment
        };
        let title_fg = if current { t.fg } else { t.comment };
        let bg = if selected && !current { t.bg_alt } else { t.bg };
        let sel = if selected { "›" } else { " " };
        lines.push(fill(
            vec![
                Span::styled(format!("{sel}{mark}  "), st(mark_fg, bg)),
                Span::styled(
                    dialog.stage_title(i).to_string(),
                    Style::new().fg(title_fg).bg(bg).add_modifier(if current {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                ),
            ],
            width,
            bg,
        ));
        let expanded = current || dialog.inspect == Some(i);
        if expanded {
            let inner_w = width.saturating_sub(4);
            let body = stage_body(dialog, i, inner_w, t);
            let tint = if current { t.bg_alt } else { t.bg };
            for b in body {
                let mut row = vec![Span::styled(" │ ", st(t.comment, tint))];
                for mut sp in b.spans {
                    // Preserve the cursor's dark block; tinting its background
                    // makes the inverse space indistinguishable from the row.
                    if !sp.style.add_modifier.contains(Modifier::SLOW_BLINK) {
                        sp.style = sp.style.bg(tint);
                    }
                    row.push(sp);
                }
                lines.push(fill(row, width, tint));
            }
        } else {
            let sum = dialog.stage_summary(i);
            if !sum.is_empty() {
                lines.push(fill(
                    vec![
                        Span::styled(" │  ", st(t.comment, t.bg)),
                        Span::styled(
                            trunc(&sum, width.saturating_sub(4)),
                            st(t.comment, t.bg),
                        ),
                    ],
                    width,
                    t.bg,
                ));
            }
        }
    }
    lines
}

fn label_line(
    text: &str,
    width: usize,
    fg: ratatui::style::Color,
    t: Theme,
) -> Line<'static> {
    fill(
        vec![Span::styled(trunc(text, width), st(fg, t.bg))],
        width,
        t.bg,
    )
}

fn field_spans(
    value: &str,
    cursor: usize,
    focused: bool,
    secret: bool,
    t: Theme,
) -> Vec<Span<'static>> {
    let shown: String = if secret {
        mask_secret(value)
    } else {
        value.to_string()
    };
    if !focused {
        return vec![Span::styled(shown, st(t.fg, t.bg))];
    }
    let chars: Vec<char> = shown.chars().collect();
    let i = cursor.min(chars.len());
    let left: String = chars[..i].iter().collect();
    let right: String = if i < chars.len() {
        chars[i + 1..].iter().collect()
    } else {
        String::new()
    };
    let mut s = vec![];
    if !left.is_empty() {
        s.push(Span::styled(left, st(t.fg, t.bg)));
    }
    let cursor_char = chars.get(i).copied().unwrap_or(' ');
    s.push(Span::styled(
        cursor_char.to_string(),
        st(t.bg, t.fg).add_modifier(Modifier::SLOW_BLINK),
    ));
    if !right.is_empty() {
        s.push(Span::styled(right, st(t.fg, t.bg)));
    }
    s
}

fn stage_body(
    dialog: &ActionDialog,
    i: usize,
    width: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    match &dialog.op {
        Op::Clone { form, .. } => match form.kind {
            CloneKind::Directory => match i {
                0 => form_lines(dialog, width, dialog.git_started(), t),
                _ => error_or_result_lines(dialog, width, t),
            },
            CloneKind::Repository => match i {
                0 => form_lines(dialog, width, dialog.git_started(), t),
                1 => auth_lines(dialog, width, t),
                2 => run_lines(dialog, width, "Clone", t),
                _ => error_or_result_lines(dialog, width, t),
            },
        },
        Op::Delete { form } => match i {
            0 => target_lines(dialog, width, dialog.git_started(), t),
            1 => run_lines(dialog, width, "Remote verification fetch", t),
            2 => findings_lines(dialog, width, t),
            3 => confirm_body(dialog, width, t),
            _ => {
                if matches!(dialog.outcome, Some(DialogOutcome::Failed { .. }))
                    && form.stage == DeleteStage::Delete
                {
                    error_or_result_lines(dialog, width, t)
                } else {
                    run_lines(
                        dialog,
                        width,
                        &format!("{} `{}`", form.strategy, form.path),
                        t,
                    )
                }
            }
        },
    }
}

fn form_lines(
    dialog: &ActionDialog,
    width: usize,
    locked: bool,
    t: Theme,
) -> Vec<Line<'static>> {
    let Op::Clone { form, .. } = &dialog.op else {
        return vec![];
    };
    let directory = form.kind == CloneKind::Directory;
    let src_focus =
        !directory && !locked && dialog.item() == Some(FocusItem::Source);
    let dst_focus = !locked && dialog.item() == Some(FocusItem::Dest);
    let lock = if locked { "  (locked)" } else { "" };
    let dest_label = match &form.parent {
        Some(parent) => {
            format!("Destination (relative to `{parent}`){lock}")
        }
        None => format!("Destination (absolute or ~){lock}"),
    };
    let mut lines = Vec::new();
    if !directory {
        let ssh_focused =
            !locked && dialog.item() == Some(FocusItem::ProtocolSsh);
        let https_focused =
            !locked && dialog.item() == Some(FocusItem::ProtocolHttps);
        let ssh_style = Style::new()
            .fg(
                if form.protocol == crate::config::CloneProtocol::Ssh
                    || ssh_focused
                {
                    t.accent
                } else {
                    t.fg
                },
            )
            .add_modifier(if ssh_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let https_style = Style::new()
            .fg(
                if form.protocol == crate::config::CloneProtocol::Https
                    || https_focused
                {
                    t.accent
                } else {
                    t.fg
                },
            )
            .add_modifier(if https_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        lines.push(Line::from(vec![
            Span::styled("[ SSH ]", ssh_style),
            Span::styled(" ", st(t.bg, t.bg)),
            Span::styled("[ HTTPS ]", https_style),
        ]));
        let hint = "owner/repo";
        let label = trunc(
            "Repository path",
            width.saturating_sub(hint.chars().count() + 1),
        );
        lines.push(Line::from(vec![
            Span::styled(label, st(t.operator, t.bg)),
            Span::raw(" "),
            Span::styled(
                trunc(
                    hint,
                    width.saturating_sub("Repository path ".chars().count()),
                ),
                st(t.comment, t.bg),
            ),
        ]));
        let source_spans = field_spans(
            form.source.text(),
            dialog.cursor(),
            src_focus,
            false,
            t,
        );
        lines.push(fill(source_spans, width, t.bg));
        lines.push(blank(width, t));
    }
    lines.push(label_line(&dest_label, width, t.operator, t));
    lines.push(fill(
        field_spans(form.dest.text(), dialog.cursor(), dst_focus, false, t),
        width,
        t.bg,
    ));
    if let Ok(abs) = dialog.abs_dest() {
        lines.push(fill(
            vec![
                Span::styled("→ ", st(t.operator, t.bg)),
                Span::styled(
                    trunc(&abs, width.saturating_sub(2)),
                    st(t.comment, t.bg),
                ),
            ],
            width,
            t.bg,
        ));
    }
    if !directory {
        let focused =
            !locked && dialog.item() == Some(FocusItem::PresetsToggle);
        let label = if form.show_prefixes {
            "[ Edit prefixes: hide ]"
        } else {
            "[ Edit prefixes: show ]"
        };
        lines.push(fill(
            vec![Span::styled(
                label,
                Style::new()
                    .fg(if focused { t.accent } else { t.operator })
                    .bg(t.bg)
                    .add_modifier(Modifier::BOLD),
            )],
            width,
            t.bg,
        ));
        if form.show_prefixes {
            for (name, item, field) in [
                ("SSH  ", FocusItem::SshPrefix, &form.ssh_prefix),
                ("HTTPS", FocusItem::HttpsPrefix, &form.https_prefix),
            ] {
                let focused = !locked && dialog.item() == Some(item);
                let mut spans = vec![Span::styled(
                    format!("{name} "),
                    st(t.operator, t.bg),
                )];
                spans.extend(field_spans(
                    field.text(),
                    dialog.cursor(),
                    focused,
                    false,
                    t,
                ));
                lines.push(fill(spans, width, t.bg));
            }
        }
    }
    if dialog.show_add_parent() {
        lines.push(blank(width, t));
        let mark = if form.add_parent { "×" } else { " " };
        let fg = if !locked && dialog.item() == Some(FocusItem::AddParent) {
            t.accent
        } else {
            t.fg
        };
        let parent = dialog
            .abs_dest()
            .ok()
            .as_deref()
            .and_then(|abs| abs.rsplit_once('/').map(|(p, _)| p.to_string()))
            .unwrap_or_else(|| form.parent.clone().unwrap_or_default());
        lines.push(fill(
            vec![Span::styled(
                trunc(&format!("[{mark}] Add `{parent}` to paths"), width),
                st(fg, t.bg),
            )],
            width,
            t.bg,
        ));
    }
    if let Some(err) = dialog.clone_validation_error() {
        lines.push(blank(width, t));
        lines.push(fill(
            vec![Span::styled(trunc(&err, width), st(t.red, t.bg))],
            width,
            t.bg,
        ));
    }
    lines
}

fn auth_lines(
    dialog: &ActionDialog,
    width: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    let Op::Clone { form, .. } = &dialog.op else {
        return vec![];
    };
    match form.auth {
        CloneAuth::Passphrase => {
            prompt_lines(dialog, width, "Git needs a passphrase", true, t)
        }
        CloneAuth::Username => {
            prompt_lines(dialog, width, "Git needs a username", false, t)
        }
        CloneAuth::HostKey => {
            let mut lines = vec![
                label_line(
                    "Git needs a host-key decision",
                    width,
                    t.operator,
                    t,
                ),
                blank(width, t),
            ];
            for l in dialog.log.iter().take(4) {
                for w in wrap_text(l, width) {
                    lines.push(label_line(&w, width, t.comment, t));
                }
            }
            lines.push(blank(width, t));
            lines.push(label_line(
                "Native control: Accept or Reject. Esc does not stop git.",
                width,
                t.comment,
                t,
            ));
            lines
        }
        CloneAuth::None => {
            let mut lines = vec![fill(
                vec![Span::styled(
                    trunc(" embedded git · unknown interaction", width),
                    st(t.git_icon, t.bg_alt),
                )],
                width,
                t.bg_alt,
            )];
            for l in dialog.log.iter().rev().take(6).rev() {
                lines.push(fill(
                    vec![
                        Span::styled("│ ", st(t.comment, t.bg_alt)),
                        Span::styled(
                            trunc(l, width.saturating_sub(2)),
                            st(t.comment, t.bg_alt),
                        ),
                    ],
                    width,
                    t.bg_alt,
                ));
            }
            lines
        }
    }
}

fn prompt_lines(
    dialog: &ActionDialog,
    width: usize,
    title: &str,
    secret: bool,
    t: Theme,
) -> Vec<Line<'static>> {
    let mut lines =
        vec![label_line(title, width, t.operator, t), blank(width, t)];
    for l in dialog.log.iter().rev().take(3).rev() {
        for w in wrap_text(l, width) {
            lines.push(label_line(&w, width, t.comment, t));
        }
    }
    lines.push(blank(width, t));
    lines.push(fill(
        field_spans(
            dialog.prompt(),
            dialog.cursor(),
            dialog.item() == Some(FocusItem::Prompt),
            secret,
            t,
        ),
        width,
        t.bg,
    ));
    lines
}

fn run_lines(
    dialog: &ActionDialog,
    width: usize,
    title: &str,
    t: Theme,
) -> Vec<Line<'static>> {
    let mut lines =
        vec![label_line(title, width, t.operator, t), blank(width, t)];
    match dialog.cancel {
        CancelState::Grace => {
            lines.push(label_line(
                "SIGINT sent · waiting for git to exit",
                width,
                t.accent,
                t,
            ));
        }
        CancelState::ForceReady => {
            lines.push(label_line(
                "git still running · Force Stop will kill it",
                width,
                t.red,
                t,
            ));
        }
        CancelState::Idle => {}
    }
    for l in dialog.log.iter().rev().take(6).rev() {
        lines.push(label_line(l, width, t.comment, t));
    }
    if dialog.log.is_empty() && dialog.running() {
        lines.push(label_line("waiting for git", width, t.comment, t));
    }
    lines
}

fn error_or_result_lines(
    dialog: &ActionDialog,
    width: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    match &dialog.outcome {
        Some(DialogOutcome::Failed { message }) => {
            let mut lines = vec![label_line(
                "Error — remains until acknowledged",
                width,
                t.red,
                t,
            )];
            for w in wrap_text(message, width) {
                lines.push(fill(
                    vec![Span::styled(trunc(&w, width), st(t.red, t.bg))],
                    width,
                    t.bg,
                ));
            }
            lines
        }
        Some(DialogOutcome::Completed {
            summary,
            config_error,
            refresh_error,
        }) => {
            let mut lines = vec![label_line(summary, width, t.green, t)];
            if let Some(err) = config_error {
                for w in wrap_text(err, width) {
                    lines.push(label_line(&w, width, t.red, t));
                }
            }
            if let Some(err) = refresh_error {
                for w in wrap_text(err, width) {
                    lines.push(label_line(&w, width, t.red, t));
                }
            }
            lines
        }
        _ => vec![label_line("waiting", width, t.comment, t)],
    }
}

fn target_lines(
    dialog: &ActionDialog,
    width: usize,
    locked: bool,
    t: Theme,
) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    let lock = if locked { "  (locked)" } else { "" };
    vec![
        label_line(&format!("Exact target{lock}"), width, t.operator, t),
        fill(
            vec![Span::styled(trunc(&form.path, width), st(t.fg, t.bg))],
            width,
            t.bg,
        ),
        blank(width, t),
        label_line(&format!("Class     {}", form.class), width, t.comment, t),
        label_line(
            &format!("Strategy  {}", form.strategy),
            width,
            t.comment,
            t,
        ),
        blank(width, t),
        label_line(
            "Linked worktrees use git; others follow config trash/permanent.",
            width,
            t.comment,
            t,
        ),
    ]
}

fn findings_lines(
    dialog: &ActionDialog,
    width: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    let mut lines = vec![label_line(
        &format!("Preflight  {}", form.path),
        width,
        t.operator,
        t,
    )];
    let all = &form.findings;
    let vis = 8.min(all.len());
    let start = dialog.view_scroll.min(all.len().saturating_sub(vis));
    for (i, l) in all.iter().skip(start).take(vis).enumerate() {
        let fg = if form.blocked {
            t.red
        } else if i == 0 {
            t.accent
        } else {
            t.fg
        };
        let bg = if dialog.item() == Some(FocusItem::Warnings) && i == 0 {
            t.bg_alt
        } else {
            t.bg
        };
        lines.push(fill(
            vec![Span::styled(trunc(l, width), st(fg, bg))],
            width,
            bg,
        ));
    }
    if all.len() > vis {
        lines.push(label_line("↑↓ scroll", width, t.operator, t));
    }
    lines
}

fn confirm_body(
    dialog: &ActionDialog,
    width: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    match form.confirm {
        DeleteConfirm::Worktree => confirm_lines(
            width,
            &format!("Delete git worktree `{}`?", form.path),
            "Non-force git worktree remove. Link object only.",
            t,
        ),
        DeleteConfirm::Permanent => permanent_lines(dialog, width, t),
        DeleteConfirm::Trash => confirm_lines(
            width,
            &format!("Move `{}` to trash?", form.path),
            "Recoverable. [y/N] as a TUI choice.",
            t,
        ),
    }
}

fn confirm_lines(
    width: usize,
    title: &str,
    note: &str,
    t: Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![label_line(title, width, t.accent, t)];
    for w in wrap_text(note, width) {
        lines.push(label_line(&w, width, t.comment, t));
    }
    lines
}

fn permanent_lines(
    dialog: &ActionDialog,
    width: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    let mut lines = vec![
        label_line(
            &format!("Permanently delete `{}`.", form.path),
            width,
            t.red,
            t,
        ),
        label_line("Type the exact path to confirm:", width, t.comment, t),
        fill(
            field_spans(
                form.perm.text(),
                dialog.cursor(),
                dialog.item() == Some(FocusItem::PermPath),
                false,
                t,
            ),
            width,
            t.bg,
        ),
    ];
    if !form.perm.text().is_empty() && form.perm.text().trim() != form.path {
        lines.push(label_line("path does not match", width, t.red, t));
    }
    lines
}

fn wrap_text(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut out = vec![];
    for para in s.split('\n') {
        if para.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in para.split(' ') {
            if line.is_empty() {
                line = word.to_string();
            } else if line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
