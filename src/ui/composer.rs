//! Input line: mode, user, text with cursor, and status (connection,
//! uploads, notices).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{icon, is_focused, text::truncate};
use crate::app::event::SyncState;
use crate::app::input::LineInput;
use crate::app::{App, Focus};
use crate::theme::blend;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let focused = is_focused(app, Focus::Input);
    let block = Block::default().borders(Borders::TOP).border_style(Style::default().fg(t.border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 4 { return; }

    // right side: transient notices, uploads in progress, connection state
    let mut right: Vec<Span> = Vec::new();
    if let Some((msg, _)) = &app.status {
        let color = blend(t.success, t.background, app.status_opacity());
        right.push(Span::styled(format!("{} ", truncate(msg, inner.width as usize / 4)), Style::default().fg(color)));
    }
    if app.uploads > 0 {
        right.push(Span::styled(format!("{} {} ", icon(app, "⬆", "^"), app.uploads), Style::default().fg(t.accent)));
    }
    // until the session is verified, old encrypted messages stay unreadable
    if app.security.verified == Some(false) {
        right.push(Span::styled(format!("{} unverified (F2) ", icon(app, "⚠", "!")), Style::default().fg(t.error)));
    }
    match &app.sync_state {
        SyncState::Online => {}
        SyncState::Connecting => right.push(Span::styled("connecting… ", Style::default().fg(t.muted))),
        SyncState::Offline(_) => right.push(Span::styled("offline ", Style::default().fg(t.error))),
    }
    let right_w = right.iter().map(|s| s.content.width()).sum::<usize>() as u16;

    // left side: mode (like vim's modeline), user and prompt
    let mode = if app.is_insert() {
        Span::styled(" INSERT ", Style::default().fg(t.background).bg(t.accent).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" NORMAL ", Style::default().fg(t.text).bg(t.surface).add_modifier(Modifier::BOLD))
    };
    let user_style = if focused {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.muted)
    };
    let max_user = (inner.width as usize / 5).max(6);
    let prompt = vec![
        mode,
        Span::styled(format!(" {}", short_user(&app.user_id, max_user)), user_style),
        Span::styled(format!(" {} ", icon(app, "›", ">")), Style::default().fg(t.muted)),
    ];
    let prompt_w = prompt.iter().map(|s| s.content.width()).sum::<usize>() as u16;

    let text_area = Rect {
        x: inner.x + prompt_w,
        width: inner.width.saturating_sub(prompt_w + right_w + 1),
        ..inner
    };
    f.render_widget(Paragraph::new(Line::from(prompt)), inner);
    f.render_widget(
        Paragraph::new(Line::from(right)).right_aligned(),
        Rect { x: inner.x + inner.width - right_w.min(inner.width), width: right_w.min(inner.width), ..inner },
    );

    if app.input.is_empty() {
        let placeholder = match (app.current_room(), app.is_insert()) {
            (None, _) => "no room open".to_string(),
            (Some(r), true) => format!("Message {}…  (Esc for commands)", r.name),
            (Some(_), false) => "press i to write · F1 help".to_string(),
        };
        let placeholder = truncate(&placeholder, text_area.width as usize);
        f.render_widget(Paragraph::new(Span::styled(placeholder, Style::default().fg(t.timestamp))), text_area);
    }
    // in normal mode pending text is shown from its start
    let style = if app.is_insert() { Style::default().fg(t.text) } else { Style::default().fg(t.muted) };
    let x = render_line_input(f, &app.input, text_area, style, app.is_insert());
    if focused { f.set_cursor_position((x, text_area.y)); }
}

/// Full MXID if it fits, otherwise just the localpart ("@me")
fn short_user(user_id: &str, max: usize) -> String {
    if user_id.width() <= max { return user_id.to_string(); }
    let local = user_id.split(':').next().unwrap_or(user_id);
    truncate(local, max)
}

/// Horizontally scrolling text field; returns the cursor column
pub fn render_line_input(f: &mut Frame, input: &LineInput, area: Rect, style: Style, follow_cursor: bool) -> u16 {
    if area.width < 2 { return area.x; }
    let avail = area.width as usize - 1; // 1 column for the cursor at line end
    let chars: Vec<char> = input.as_str().chars().collect();
    let cursor = if follow_cursor { input.cursor().min(chars.len()) } else { 0 };
    let width_of = |a: usize, b: usize| chars[a..b].iter().map(|c| c.width().unwrap_or(0)).sum::<usize>();

    // first visible char: the cursor must stay inside the area
    let mut start = 0;
    while start < cursor && width_of(start, cursor) > avail { start += 1; }
    let mut shown = String::new();
    let mut w = 0;
    for &c in &chars[start..] {
        let cw = c.width().unwrap_or(0);
        if w + cw > avail { break; }
        shown.push(c);
        w += cw;
    }
    if !input.is_empty() {
        f.render_widget(Paragraph::new(Span::styled(shown, style)), area);
    }
    area.x + width_of(start, cursor) as u16
}
