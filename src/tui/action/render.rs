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

const T: Theme = Theme::LIGHT;

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
        T.red
    } else if dialog.running() {
        T.accent
    } else {
        T.git_icon
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Double)
        .border_style(st(border_fg, T.bg))
        .style(st(T.fg, T.bg))
        .padding(Padding::horizontal(1));
    let inner = block.inner(rect);
    Clear.render(rect, buf);
    block.render(rect, buf);
    render_a(dialog, inner, buf);
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

fn render_a(dialog: &ActionDialog, area: Rect, buf: &mut Buffer) {
    let (body, hint, btns) = footer_chunks(area);
    let lines = accordion_lines(dialog, body.width as usize);
    let skip = dialog.view_scroll.min(lines.len().saturating_sub(1));
    put_lines(buf, body, lines.into_iter().skip(skip).collect());
    put_lines(buf, hint, vec![hint_line(dialog, hint.width as usize)]);
    put_lines(buf, btns, vec![buttons_line(dialog, btns.width as usize)]);
}

pub(crate) fn render_toast(
    area: Rect,
    buf: &mut Buffer,
    kind: ToastKind,
    msg: &str,
) {
    let (mark, fg) = match kind {
        ToastKind::Success => (" ✓  ", T.green),
        ToastKind::Cancel => (" ↷  ", T.accent),
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
        .border_style(st(fg, T.bg))
        .style(st(T.fg, T.bg));
    let inner = block.inner(rect);
    Clear.render(rect, buf);
    block.render(rect, buf);
    Paragraph::new(Line::from(vec![Span::styled(
        trunc(&text, inner.width as usize),
        st(fg, T.bg),
    )]))
    .render(inner, buf);
}

fn blank(width: usize) -> Line<'static> {
    fill(vec![], width, T.bg)
}

fn hint_line(dialog: &ActionDialog, width: usize) -> Line<'static> {
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
    } else {
        "Esc cancels · completed stages reopen with i".into()
    };
    fill(
        vec![Span::styled(trunc(&text, width), st(T.operator, T.bg))],
        width,
        T.bg,
    )
}

#[derive(Clone, Copy)]
enum BtnKind {
    Primary,
    Danger,
    Ack,
}

fn btn(label: &str, focused: bool, kind: BtnKind) -> Span<'static> {
    let (fg, md) = match (focused, kind) {
        (true, BtnKind::Danger) => (T.red, Modifier::BOLD),
        (true, BtnKind::Primary) => (T.accent, Modifier::BOLD),
        (true, BtnKind::Ack) => (T.accent, Modifier::BOLD),
        (false, BtnKind::Danger) => (T.red, Modifier::empty()),
        (false, _) => (T.fg, Modifier::empty()),
    };
    Span::styled(
        format!("[{label}]"),
        Style::new().fg(fg).bg(T.bg).add_modifier(md),
    )
}

fn buttons_line(dialog: &ActionDialog, width: usize) -> Line<'static> {
    let items = dialog.items();
    let focus = dialog.item();
    let mut spans = vec![];
    let mut push = |label: &str, on: bool, kind: BtnKind| {
        if !spans.is_empty() {
            spans.push(Span::styled("  ", st(T.fg, T.bg)));
        }
        spans.push(btn(label, on, kind));
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
    if items.contains(&FocusItem::Cancel) {
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
    if items.contains(&FocusItem::Action) {
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
    fill(spans, width, T.bg)
}

fn accordion_lines(dialog: &ActionDialog, width: usize) -> Vec<Line<'static>> {
    let n = dialog.stage_n();
    let cur = dialog.current_stage();
    let mut lines = vec![];
    for i in 0..n {
        if i > 0 {
            lines.push(fill(
                vec![Span::styled(" │", st(T.comment, T.bg))],
                width,
                T.bg,
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
            T.accent
        } else if done {
            T.green
        } else {
            T.comment
        };
        let title_fg = if current { T.fg } else { T.comment };
        let bg = if selected && !current { T.bg_alt } else { T.bg };
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
            let body = stage_body(dialog, i, inner_w);
            let tint = if current { T.bg_alt } else { T.bg };
            for b in body {
                let mut row = vec![Span::styled(" │ ", st(T.comment, tint))];
                for mut sp in b.spans {
                    sp.style = sp.style.bg(tint);
                    row.push(sp);
                }
                lines.push(fill(row, width, tint));
            }
        } else {
            let sum = dialog.stage_summary(i);
            if !sum.is_empty() {
                lines.push(fill(
                    vec![
                        Span::styled(" │  ", st(T.comment, T.bg)),
                        Span::styled(
                            trunc(&sum, width.saturating_sub(4)),
                            st(T.comment, T.bg),
                        ),
                    ],
                    width,
                    T.bg,
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
) -> Line<'static> {
    fill(
        vec![Span::styled(trunc(text, width), st(fg, T.bg))],
        width,
        T.bg,
    )
}

fn field_spans(
    value: &str,
    cursor: usize,
    focused: bool,
    secret: bool,
) -> Vec<Span<'static>> {
    let shown: String = if secret {
        mask_secret(value)
    } else {
        value.to_string()
    };
    if !focused {
        return vec![Span::styled(shown, st(T.fg, T.bg))];
    }
    let chars: Vec<char> = shown.chars().collect();
    let i = cursor.min(chars.len());
    let left: String = chars[..i].iter().collect();
    let right: String = chars[i..].iter().collect();
    let mut s = vec![];
    if !left.is_empty() {
        s.push(Span::styled(left, st(T.fg, T.bg)));
    }
    s.push(Span::styled("█", st(T.git_icon, T.bg)));
    if !right.is_empty() {
        s.push(Span::styled(right, st(T.fg, T.bg)));
    }
    s
}

fn stage_body(
    dialog: &ActionDialog,
    i: usize,
    width: usize,
) -> Vec<Line<'static>> {
    match &dialog.op {
        Op::Clone { form, .. } => match form.kind {
            CloneKind::Directory => match i {
                0 => form_lines(dialog, width, dialog.git_started()),
                _ => error_or_result_lines(dialog, width),
            },
            CloneKind::Repository => match i {
                0 => form_lines(dialog, width, dialog.git_started()),
                1 => auth_lines(dialog, width),
                2 => run_lines(dialog, width, "Clone"),
                _ => error_or_result_lines(dialog, width),
            },
        },
        Op::Delete { form } => match i {
            0 => target_lines(dialog, width, dialog.git_started()),
            1 => run_lines(dialog, width, "Remote verification fetch"),
            2 => findings_lines(dialog, width),
            3 => confirm_body(dialog, width),
            _ => {
                if matches!(dialog.outcome, Some(DialogOutcome::Failed { .. }))
                    && form.stage == DeleteStage::Delete
                {
                    error_or_result_lines(dialog, width)
                } else {
                    run_lines(
                        dialog,
                        width,
                        &format!("{} `{}`", form.strategy, form.path),
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
        lines.push(label_line(&format!("Source{lock}"), width, T.operator));
        lines.push(fill(
            field_spans(form.source.text(), dialog.cursor(), src_focus, false),
            width,
            T.bg,
        ));
        lines.push(blank(width));
    }
    lines.push(label_line(&dest_label, width, T.operator));
    lines.push(fill(
        field_spans(form.dest.text(), dialog.cursor(), dst_focus, false),
        width,
        T.bg,
    ));
    if let Ok(abs) = dialog.abs_dest() {
        lines.push(fill(
            vec![
                Span::styled("→ ", st(T.operator, T.bg)),
                Span::styled(
                    trunc(&abs, width.saturating_sub(2)),
                    st(T.comment, T.bg),
                ),
            ],
            width,
            T.bg,
        ));
    }
    if dialog.show_add_parent() {
        lines.push(blank(width));
        let mark = if form.add_parent { "×" } else { " " };
        let fg = if !locked && dialog.item() == Some(FocusItem::AddParent) {
            T.accent
        } else {
            T.fg
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
                st(fg, T.bg),
            )],
            width,
            T.bg,
        ));
    }
    if let Some(err) = dialog.clone_validation_error() {
        lines.push(blank(width));
        lines.push(fill(
            vec![Span::styled(trunc(&err, width), st(T.red, T.bg))],
            width,
            T.bg,
        ));
    }
    lines
}

fn auth_lines(dialog: &ActionDialog, width: usize) -> Vec<Line<'static>> {
    let Op::Clone { form, .. } = &dialog.op else {
        return vec![];
    };
    match form.auth {
        CloneAuth::Passphrase => {
            prompt_lines(dialog, width, "Git needs a passphrase", true)
        }
        CloneAuth::Username => {
            prompt_lines(dialog, width, "Git needs a username", false)
        }
        CloneAuth::HostKey => {
            let mut lines = vec![
                label_line("Git needs a host-key decision", width, T.operator),
                blank(width),
            ];
            for l in dialog.log.iter().take(4) {
                for w in wrap_text(l, width) {
                    lines.push(label_line(&w, width, T.comment));
                }
            }
            lines.push(blank(width));
            lines.push(label_line(
                "Native control: Accept or Reject. Esc does not stop git.",
                width,
                T.comment,
            ));
            lines
        }
        CloneAuth::None => {
            let mut lines = vec![fill(
                vec![Span::styled(
                    trunc(" embedded git · unknown interaction", width),
                    st(T.git_icon, T.bg_alt),
                )],
                width,
                T.bg_alt,
            )];
            for l in dialog.log.iter().rev().take(6).rev() {
                lines.push(fill(
                    vec![
                        Span::styled("│ ", st(T.comment, T.bg_alt)),
                        Span::styled(
                            trunc(l, width.saturating_sub(2)),
                            st(T.comment, T.bg_alt),
                        ),
                    ],
                    width,
                    T.bg_alt,
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
) -> Vec<Line<'static>> {
    let mut lines = vec![label_line(title, width, T.operator), blank(width)];
    for l in dialog.log.iter().rev().take(3).rev() {
        for w in wrap_text(l, width) {
            lines.push(label_line(&w, width, T.comment));
        }
    }
    lines.push(blank(width));
    lines.push(fill(
        field_spans(
            dialog.prompt(),
            dialog.cursor(),
            dialog.item() == Some(FocusItem::Prompt),
            secret,
        ),
        width,
        T.bg,
    ));
    lines
}

fn run_lines(
    dialog: &ActionDialog,
    width: usize,
    title: &str,
) -> Vec<Line<'static>> {
    let mut lines = vec![label_line(title, width, T.operator), blank(width)];
    match dialog.cancel {
        CancelState::Grace => {
            lines.push(label_line(
                "SIGINT sent · waiting for git to exit",
                width,
                T.accent,
            ));
        }
        CancelState::ForceReady => {
            lines.push(label_line(
                "git still running · Force Stop will kill it",
                width,
                T.red,
            ));
        }
        CancelState::Idle => {}
    }
    for l in dialog.log.iter().rev().take(6).rev() {
        lines.push(label_line(l, width, T.comment));
    }
    if dialog.log.is_empty() && dialog.running() {
        lines.push(label_line("waiting for git", width, T.comment));
    }
    lines
}

fn error_or_result_lines(
    dialog: &ActionDialog,
    width: usize,
) -> Vec<Line<'static>> {
    match &dialog.outcome {
        Some(DialogOutcome::Failed { message }) => {
            let mut lines = vec![label_line(
                "Error — remains until acknowledged",
                width,
                T.red,
            )];
            for w in wrap_text(message, width) {
                lines.push(fill(
                    vec![Span::styled(trunc(&w, width), st(T.red, T.bg))],
                    width,
                    T.bg,
                ));
            }
            lines
        }
        Some(DialogOutcome::Completed {
            summary,
            config_error,
            refresh_error,
        }) => {
            let mut lines = vec![label_line(summary, width, T.green)];
            if let Some(err) = config_error {
                for w in wrap_text(err, width) {
                    lines.push(label_line(&w, width, T.red));
                }
            }
            if let Some(err) = refresh_error {
                for w in wrap_text(err, width) {
                    lines.push(label_line(&w, width, T.red));
                }
            }
            lines
        }
        _ => vec![label_line("waiting", width, T.comment)],
    }
}

fn target_lines(
    dialog: &ActionDialog,
    width: usize,
    locked: bool,
) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    let lock = if locked { "  (locked)" } else { "" };
    vec![
        label_line(&format!("Exact target{lock}"), width, T.operator),
        fill(
            vec![Span::styled(trunc(&form.path, width), st(T.fg, T.bg))],
            width,
            T.bg,
        ),
        blank(width),
        label_line(&format!("Class     {}", form.class), width, T.comment),
        label_line(&format!("Strategy  {}", form.strategy), width, T.comment),
        blank(width),
        label_line(
            "Linked worktrees use git; others follow config trash/permanent.",
            width,
            T.comment,
        ),
    ]
}

fn findings_lines(dialog: &ActionDialog, width: usize) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    let mut lines = vec![label_line(
        &format!("Preflight  {}", form.path),
        width,
        T.operator,
    )];
    let all = &form.findings;
    let vis = 8.min(all.len());
    let start = dialog.view_scroll.min(all.len().saturating_sub(vis));
    for (i, l) in all.iter().skip(start).take(vis).enumerate() {
        let fg = if form.blocked {
            T.red
        } else if i == 0 {
            T.accent
        } else {
            T.fg
        };
        let bg = if dialog.item() == Some(FocusItem::Warnings) && i == 0 {
            T.bg_alt
        } else {
            T.bg
        };
        lines.push(fill(
            vec![Span::styled(trunc(l, width), st(fg, bg))],
            width,
            bg,
        ));
    }
    if all.len() > vis {
        lines.push(label_line("↑↓ scroll", width, T.operator));
    }
    lines
}

fn confirm_body(dialog: &ActionDialog, width: usize) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    match form.confirm {
        DeleteConfirm::Worktree => confirm_lines(
            width,
            &format!("Delete git worktree `{}`?", form.path),
            "Non-force git worktree remove. Link object only.",
        ),
        DeleteConfirm::Permanent => permanent_lines(dialog, width),
        DeleteConfirm::Trash => confirm_lines(
            width,
            &format!("Move `{}` to trash?", form.path),
            "Recoverable. [y/N] as a TUI choice.",
        ),
    }
}

fn confirm_lines(width: usize, title: &str, note: &str) -> Vec<Line<'static>> {
    let mut lines = vec![label_line(title, width, T.accent)];
    for w in wrap_text(note, width) {
        lines.push(label_line(&w, width, T.comment));
    }
    lines
}

fn permanent_lines(dialog: &ActionDialog, width: usize) -> Vec<Line<'static>> {
    let Op::Delete { form } = &dialog.op else {
        return vec![];
    };
    let mut lines = vec![
        label_line(
            &format!("Permanently delete `{}`.", form.path),
            width,
            T.red,
        ),
        label_line("Type the exact path to confirm:", width, T.comment),
        fill(
            field_spans(
                form.perm.text(),
                dialog.cursor(),
                dialog.item() == Some(FocusItem::PermPath),
                false,
            ),
            width,
            T.bg,
        ),
    ];
    if !form.perm.text().is_empty() && form.perm.text().trim() != form.path {
        lines.push(label_line("path does not match", width, T.red));
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
