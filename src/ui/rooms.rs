//! Room bar of the selected space, split into sections.

use chrono::{DateTime, Local, Utc};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::{column_block, header_line, icon, is_focused, text::truncate};
use crate::app::model::Content;
use crate::app::{App, Entry, Focus, HitMap, Section};

pub fn render(f: &mut Frame, app: &App, area: Rect, hit: &mut HitMap) {
    let t = &app.theme;
    let block = column_block(t);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let focused = is_focused(app, Focus::Rooms);

    let header = vec![header_line(t, &app.space_label(), "", focused, width), Line::default()];

    hit.rooms = area;
    if app.entries.is_empty() {
        let mut lines = header;
        let msg = if app.rooms.is_empty() && app.sync_state == crate::app::event::SyncState::Connecting {
            "Loading…"
        } else {
            "No rooms here"
        };
        lines.push(Line::from(Span::styled(format!(" {msg}"), Style::default().fg(t.muted))));
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let mut lines = Vec::new();
    // for each line, the entry it belongs to (for mouse clicks)
    let mut owners: Vec<Option<usize>> = Vec::new();
    let mut cursor_line = 0;
    for (i, entry) in app.entries.iter().enumerate() {
        let selected = i == app.room_cursor;
        if selected { cursor_line = lines.len(); }
        let start = lines.len();
        match *entry {
            Entry::Section(section) => {
                if i > 0 { lines.push(Line::default()); }
                let label = match section {
                    Section::Invites => "INVITES",
                    Section::People => "PEOPLE",
                    Section::Rooms => "ROOMS",
                    Section::Explore => "EXPLORE",
                };
                lines.push(Line::from(Span::styled(format!(" {label}"), Style::default().fg(t.muted).add_modifier(Modifier::BOLD))));
            }
            Entry::Room(idx) => lines.extend(room_lines(app, idx, selected, focused, width)),
            Entry::Explore(idx) => {
                let Some(h) = app.explore_entry(idx) else { continue };
                let tag = if h.is_space { "space".to_string() } else { format!("{}", h.members) };
                lines.push(item_line(app, "+", &h.name, &tag, selected, focused, false, width, true));
            }
        }
        let owner = (!matches!(entry, Entry::Section(_))).then_some(i);
        owners.extend(std::iter::repeat_n(owner, lines.len() - start));
    }

    // keep the cursor visible (headers stay pinned at the top)
    let height = (inner.height as usize).saturating_sub(header.len());
    let scroll = (cursor_line + 3).saturating_sub(height);
    let [head_area, list_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(header.len() as u16),
        ratatui::layout::Constraint::Min(0),
    ]).areas(inner);
    f.render_widget(Paragraph::new(header), head_area);
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), list_area);
    hit.rooms = list_area;
    hit.room_lines = owners;
    hit.rooms_scroll = scroll;
}

fn room_lines(app: &App, idx: usize, selected: bool, focused: bool, width: usize) -> Vec<Line<'static>> {
    let t = &app.theme;
    let room = &app.rooms[idx];
    let badge = match (&room.invite, room.highlights, room.unread) {
        (Some(_), _, _) => "new".to_string(),
        (None, 0, 0) => String::new(),
        (None, 0, n) => n.to_string(),
        (None, h, _) => format!("@{h}"),
    };
    let ic = match &room.invite {
        Some(i) if i.is_space => icon(app, "◆", "*"),
        _ if room.is_direct => "@",
        _ => "#",
    };
    let mut lines = vec![item_line(app, ic, &room.name, &badge, selected, focused, room.unread > 0 || room.invite.is_some(), width, false)];

    if let Some(invite) = &room.invite {
        let bg = if selected { t.surface } else { t.background };
        let text = truncate(&format!("from {}", invite.inviter), width.saturating_sub(4));
        let pad = width.saturating_sub(text.width() + 3);
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(text, Style::default().fg(t.muted)),
            Span::raw(" ".repeat(pad)),
        ]).style(Style::default().bg(bg)));
        return lines;
    }

    if app.ui.layout.show_message_preview {
        let bg = if selected { t.surface } else { t.background };
        let last = app.timelines.get(&room.id).and_then(|tl| tl.last());
        let line = match last {
            Some(m) => {
                let time = short_time(app, &m.timestamp);
                let who = match &m.content {
                    _ if m.sender == app.user_id => "you: ".to_string(),
                    Content::Emote(_) => String::new(),
                    _ if room.is_direct => String::new(),
                    _ => format!("{}: ", m.sender_name),
                };
                let preview_w = width.saturating_sub(5 + time.width());
                let preview = truncate(&format!("{who}{}", m.preview()), preview_w);
                let pad = preview_w.saturating_sub(preview.width());
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled(preview, Style::default().fg(t.muted)),
                    Span::raw(" ".repeat(pad + 1)),
                    Span::styled(time, Style::default().fg(t.timestamp)),
                    Span::raw(" "),
                ])
            }
            None => Line::from(Span::styled(format!("   {}", " ".repeat(width.saturating_sub(3))), Style::default())),
        };
        lines.push(line.style(Style::default().bg(bg)));
    }
    lines
}

/// Room line: marker, icon, name and right-aligned badge
#[allow(clippy::too_many_arguments)]
fn item_line(app: &App, ic: &str, name: &str, badge: &str, selected: bool, focused: bool, unread: bool, width: usize, faint: bool) -> Line<'static> {
    let t = &app.theme;
    let name_w = width.saturating_sub(3 + badge.width() + 2);
    let name = truncate(name, name_w);
    let pad = name_w.saturating_sub(name.width());
    let name_style = match (selected, focused) {
        (true, true) => Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        (true, false) => Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        _ if unread => Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        _ if faint => Style::default().fg(t.muted),
        _ => Style::default().fg(t.text),
    };
    let badge_style = if faint { Style::default().fg(t.timestamp) } else { Style::default().fg(t.unread).add_modifier(Modifier::BOLD) };
    Line::from(vec![
        Span::styled(if selected { icon(app, "▌", ">") } else { " " }.to_string(), Style::default().fg(t.accent)),
        Span::styled(format!("{ic} "), Style::default().fg(if unread { t.unread } else { t.muted })),
        Span::styled(name, name_style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(badge.to_string(), badge_style),
        Span::raw(" "),
    ])
    .style(Style::default().bg(if selected { t.surface } else { t.background }))
}

fn short_time(app: &App, ts: &DateTime<Utc>) -> String {
    let local = ts.with_timezone(&Local);
    if local.date_naive() == Local::now().date_naive() {
        local.format(&app.ui.layout.time_format).to_string()
    } else {
        local.format("%d/%m").to_string()
    }
}
