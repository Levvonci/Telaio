//! Main area: room header and messages.

use chrono::{DateTime, Datelike, Local, Utc};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::{icon, is_focused, spread, text::{truncate, wrap}};
use crate::app::event::RecoveryStatus;
use crate::app::model::{AttachmentKind, Content, HierarchyEntry, Message, Room};
use crate::app::timeline::HistoryState;
use crate::app::{App, Focus};
use crate::matrix::human_size;
use crate::theme::Theme;

/// Messages from the same sender within this interval are grouped
const GROUP_SECS: i64 = 5 * 60;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let [header, body] = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
    let width = header.width as usize;
    let t = &app.theme;
    let header_block = Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(t.border));
    let header_inner = header_block.inner(header);
    f.render_widget(header_block, header);

    if let Some(room) = app.current_room().filter(|r| r.invite.is_some()) {
        f.render_widget(Paragraph::new(room_header(app, room, width)), header_inner);
        f.render_widget(Paragraph::new(invite_card(app, room)).alignment(Alignment::Center), body);
        app.chat_view.at_top = false;
    } else if let Some(line) = app.current_room().map(|room| room_header(app, room, width)) {
        f.render_widget(Paragraph::new(line), header_inner);
        render_messages(f, app, body);
    } else if let Some(h) = app.selected_explore() {
        f.render_widget(Paragraph::new(Line::from(Span::styled(format!(" + {}", h.name), Style::default().fg(t.text).add_modifier(Modifier::BOLD)))), header_inner);
        f.render_widget(Paragraph::new(explore_preview(app, h)).alignment(Alignment::Center), body);
        app.chat_view.at_top = false;
    } else {
        f.render_widget(Paragraph::new(welcome(app)).alignment(Alignment::Center), body);
        app.chat_view.at_top = false;
    }
}

fn room_header(app: &App, room: &Room, width: usize) -> Line<'static> {
    let t = &app.theme;
    let mut right = Vec::new();
    if room.members > 0 && !room.is_direct {
        right.push(Span::styled(format!("{} members ", room.members), Style::default().fg(t.muted)));
    }
    let ic = if room.is_direct { "@" } else { "#" };
    let e2ee = if room.encrypted {
        Span::styled(format!("  {} encrypted", icon(app, "✓", "+")), Style::default().fg(t.success))
    } else {
        Span::styled(format!("  {} not encrypted", icon(app, "✗", "x")), Style::default().fg(t.muted))
    };
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let name = truncate(&room.name, width.saturating_sub(right_w + e2ee.content.width() + 4).max(8));
    let used = name.width() + e2ee.content.width();
    let mut left = vec![
        Span::styled(format!(" {ic} "), Style::default().fg(t.muted)),
        Span::styled(name, Style::default().fg(t.text).add_modifier(Modifier::BOLD)),
        e2ee,
    ];
    let detail = room.topic.as_deref().or(room.alias.as_deref()).or(room.dm_with.first().map(String::as_str));
    if let Some(detail) = detail {
        let avail = width.saturating_sub(used + right_w + 7);
        if avail > 8 {
            left.push(Span::styled(format!("  {}", truncate(detail.lines().next().unwrap_or(""), avail)), Style::default().fg(t.muted)));
        }
    }
    spread(left, right, width)
}

fn welcome(app: &App) -> Vec<Line<'static>> {
    let t = &app.theme;
    let muted = Style::default().fg(t.muted);
    let key = |k: &'static str| Span::styled(k, Style::default().fg(t.accent).add_modifier(Modifier::BOLD));
    vec![
        Line::default(),
        Line::default(),
        Line::from(Span::styled("No room open", Style::default().fg(t.text).add_modifier(Modifier::BOLD))),
        Line::default(),
        Line::from(vec![key("a"), Span::styled(" join a room or space   ", muted), key("d"), Span::styled(" new chat", muted)]),
        Line::from(vec![key("Ctrl+K"), Span::styled(" search your rooms   ", muted), key("F1"), Span::styled(" all keys", muted)]),
    ]
}

fn invite_card(app: &App, room: &Room) -> Vec<Line<'static>> {
    let t = &app.theme;
    let invite = room.invite.as_ref();
    let what = if invite.is_some_and(|i| i.is_space) { "a space" } else if room.is_direct { "a chat" } else { "a room" };
    let key = |k: &'static str| Span::styled(k, Style::default().fg(t.accent).add_modifier(Modifier::BOLD));
    let muted = Style::default().fg(t.muted);
    let mut lines = vec![
        Line::default(),
        Line::default(),
        Line::from(Span::styled(room.name.clone(), Style::default().fg(t.text).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(
            format!("{} invited you to {what}", invite.map(|i| i.inviter.as_str()).filter(|s| !s.is_empty()).unwrap_or("Someone")),
            muted,
        )),
    ];
    if let Some(topic) = &room.topic {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(topic.lines().next().unwrap_or("").to_string(), Style::default().fg(t.text))));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![key("Enter"), Span::styled(" accept   ", muted), key("L"), Span::styled(" decline", muted)]));
    lines
}

fn explore_preview(app: &App, h: &HierarchyEntry) -> Vec<Line<'static>> {
    let t = &app.theme;
    let mut lines = vec![
        Line::default(),
        Line::default(),
        Line::from(Span::styled(h.name.clone(), Style::default().fg(t.text).add_modifier(Modifier::BOLD))),
    ];
    if let Some(alias) = &h.alias {
        lines.push(Line::from(Span::styled(alias.clone(), Style::default().fg(t.muted))));
    }
    let kind = if h.is_space { "space" } else { "room" };
    lines.push(Line::from(Span::styled(format!("{kind} · {} members", h.members), Style::default().fg(t.muted))));
    if let Some(topic) = &h.topic {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(topic.lines().next().unwrap_or("").to_string(), Style::default().fg(t.text))));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("Enter", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
        Span::styled(" to join", Style::default().fg(t.muted)),
    ]));
    lines
}

fn render_messages(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;
    let height = area.height as usize;
    app.chat_view.height = height;
    // the selection shows in the Messages panel and in insert mode (↑↓ from the input)
    let focused = is_focused(app, Focus::Chat) || app.is_insert();

    let Some(tl) = app.current_timeline() else { return };
    let (messages, history) = (&tl.messages, tl.history);
    let t = &app.theme;

    if messages.is_empty() {
        let text = match history {
            HistoryState::Loading | HistoryState::NotLoaded => "Loading history…",
            _ => "No messages yet. Say hi!",
        };
        f.render_widget(
            Paragraph::new(vec![Line::default(), Line::from(Span::styled(text, Style::default().fg(t.muted)))]).alignment(Alignment::Center),
            area,
        );
        app.chat_view.at_top = history == HistoryState::Partial;
        return;
    }

    let selected = if focused { app.selected_msg.map(|s| s.min(messages.len() - 1)) } else { None };
    let mut scroll = app.chat_view.scroll;

    // Lines are built from the newest message backwards, stopping once the
    // screen is full: cost depends on what is visible, not on history size.
    let mut blocks: Vec<Vec<Line<'static>>> = Vec::new();
    let mut total = 0;
    let mut sel_range = None;
    let mut reached_top = true;
    for i in (0..messages.len()).rev() {
        if total >= scroll + height && selected.is_none_or(|s| i < s) {
            reached_top = false;
            break;
        }
        let prev = i.checked_sub(1).map(|p| &messages[p]);
        let lines = message_lines(app, t, &messages[i], prev, width, Some(i) == selected);
        if Some(i) == selected { sel_range = Some((total, total + lines.len())); }
        total += lines.len();
        blocks.push(lines);
    }

    if reached_top {
        let marker = match history {
            HistoryState::Complete => Some("start of the conversation"),
            HistoryState::Loading => Some("loading older messages…"),
            HistoryState::Partial => Some("↑ scroll to load older messages"),
            HistoryState::NotLoaded => None,
        };
        if let Some(m) = marker {
            blocks.push(vec![Line::from(Span::styled(m, Style::default().fg(t.timestamp))).alignment(Alignment::Center)]);
            total += 1;
        }
    }

    // keep the selection visible
    if let Some((bottom, top)) = sel_range {
        if bottom < scroll { scroll = bottom; } else if top > scroll + height { scroll = top - height; }
    }
    if reached_top { scroll = scroll.min(total.saturating_sub(height)); }
    let at_top = reached_top && scroll + height >= total;

    let all: Vec<Line> = blocks.into_iter().rev().flatten().collect();
    let end = all.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    let mut visible: Vec<Line> = Vec::with_capacity(height);
    // with few messages the conversation sticks to the bottom
    visible.extend(std::iter::repeat_n(Line::default(), height.saturating_sub(end - start)));
    visible.extend(all[start..end].iter().cloned());
    if scroll > 0 && height > 0 {
        let hint = format!(" {} newer messages below · End ", icon(app, "↓", "v"));
        visible[height - 1] = Line::from(Span::styled(hint, Style::default().fg(t.background).bg(t.accent))).alignment(Alignment::Right);
    }
    f.render_widget(Paragraph::new(visible), area);

    app.chat_view.scroll = scroll;
    app.chat_view.at_top = at_top;
    if focused { app.selected_msg = selected; }
}


fn day_label(ts: &DateTime<Utc>) -> String {
    let d = ts.with_timezone(&Local).date_naive();
    let today = Local::now().date_naive();
    if d == today { return "Today".into(); }
    if today.pred_opt() == Some(d) { return "Yesterday".into(); }
    let format = if d.year() == today.year() { "%A, %B %-d" } else { "%A, %B %-d %Y" };
    d.format(format).to_string()
}

/// Lines of a single message, already wrapped to the available width
fn message_lines(app: &App, t: &Theme, msg: &Message, prev: Option<&Message>, width: usize, selected: bool) -> Vec<Line<'static>> {
    let local_day = |m: &Message| m.timestamp.with_timezone(&Local).date_naive();
    let is_emote = |m: &Message| matches!(m.content, Content::Emote(_));
    let new_day = prev.is_none_or(|p| local_day(p) != local_day(msg));
    let grouped = app.ui.layout.group_messages
        && !new_day
        && !is_emote(msg)
        && prev.is_some_and(|p| p.sender == msg.sender && !is_emote(p) && (msg.timestamp - p.timestamp).num_seconds() < GROUP_SECS);

    let mut lines = Vec::new();
    if new_day {
        let label = format!(" {} ", day_label(&msg.timestamp));
        let side = width.saturating_sub(label.width()) / 2;
        lines.push(Line::from(vec![
            Span::styled("─".repeat(side), Style::default().fg(t.timestamp)),
            Span::styled(label, Style::default().fg(t.muted)),
            Span::styled("─".repeat(side), Style::default().fg(t.timestamp)),
        ]));
    } else if !grouped {
        lines.push(Line::default());
    }

    let time = msg.timestamp.with_timezone(&Local).format(&app.ui.layout.time_format).to_string();
    let body_start = lines.len();
    let own = msg.sender == app.user_id;
    let sender_color = if own { t.accent } else { t.sender_color(&msg.sender) };

    if !grouped && !is_emote(msg) {
        lines.push(Line::from(vec![
            Span::styled(format!(" {}", truncate(&msg.sender_name, width.saturating_sub(10))), Style::default().fg(sender_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {time}"), Style::default().fg(t.timestamp)),
        ]));
    }

    let pad = "   ";
    let body_w = width.saturating_sub(pad.len() + 1).max(1);
    let body = |text: &str, style: Style, lines: &mut Vec<Line<'static>>| {
        for l in wrap(text, body_w) {
            lines.push(Line::from(Span::styled(format!("{pad}{l}"), style)));
        }
    };
    match &msg.content {
        Content::Text(text) => body(text, Style::default().fg(t.text), &mut lines),
        Content::Notice(text) | Content::Other(text) => body(text, Style::default().fg(t.muted).add_modifier(Modifier::ITALIC), &mut lines),
        Content::Encrypted(_) => body(
            &format!(
                "{} encrypted message — {}",
                icon(app, "🔒", "[E]"),
                // with the backup unlocked the key really is missing; otherwise it can be recovered
                if app.security.recovery == RecoveryStatus::Enabled {
                    "key not available on this device"
                } else {
                    "press F2 to verify the session and read it"
                },
            ),
            Style::default().fg(t.timestamp).add_modifier(Modifier::ITALIC),
            &mut lines,
        ),
        Content::Emote(text) => {
            for (i, l) in wrap(&format!("{} {}", msg.sender_name, text), body_w).into_iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::styled(if i == 0 { " * " } else { pad }, Style::default().fg(sender_color)),
                    Span::styled(l, Style::default().fg(t.text).add_modifier(Modifier::ITALIC)),
                ]));
            }
            if let Some(first) = lines.get_mut(body_start) {
                first.spans.push(Span::styled(format!("  {time}"), Style::default().fg(t.timestamp)));
            }
        }
        Content::Attachment(a) => {
            let ic = match a.kind {
                AttachmentKind::Image => icon(app, "▣", "[img]"),
                AttachmentKind::Video => icon(app, "▶", "[video]"),
                AttachmentKind::Audio => icon(app, "♫", "[audio]"),
                AttachmentKind::File => icon(app, "▤", "[file]"),
            };
            lines.push(Line::from(vec![
                Span::raw(pad),
                Span::styled(format!("{ic} "), Style::default().fg(t.accent)),
                Span::styled(truncate(&a.name, body_w.saturating_sub(ic.width() + 1)), Style::default().fg(t.text).add_modifier(Modifier::UNDERLINED)),
            ]));
            let mut details = vec![a.kind.label().to_string()];
            details.extend(a.size.map(human_size));
            details.extend(a.mimetype.clone());
            let mut info = vec![Span::styled(format!("{pad}{}", details.join(" · ")), Style::default().fg(t.muted))];
            if selected {
                let hint = if app.is_insert() { "   Enter open · Esc s save" } else { "   Enter open · s save" };
                info.push(Span::styled(hint, Style::default().fg(t.accent).add_modifier(Modifier::BOLD)));
            }
            lines.push(Line::from(info));
        }
    }

    if selected {
        for line in &mut lines[body_start..] {
            let w = line.width();
            if w < width { line.spans.push(Span::raw(" ".repeat(width - w))); }
            *line = std::mem::take(line).style(Style::default().bg(t.surface));
        }
        if let Some(span) = lines.get_mut(body_start).and_then(|l| l.spans.first_mut()) {
            // side marker in place of the first space
            let rest: String = span.content.chars().skip(1).collect();
            *span = Span::styled(format!("{}{rest}", icon(app, "▌", ">")), span.style.fg(t.accent));
        }
    }
    lines
}
