//! Rendering with ratatui.
//!
//! Layout: [space bar] [room bar] [chat], the two left bars can be toggled.
//! No top or bottom bar: status lives in the input line, errors in a bar
//! that appears only when needed.

mod chat;
mod composer;
mod popup;
mod rooms;
mod spaces;
pub mod text;

#[cfg(test)]
mod tests;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Focus, HitMap};
use crate::theme::Theme;
use text::truncate;

/// Errors shown at once in the bar
const MAX_VISIBLE_ERRORS: usize = 3;

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(app.theme.background).fg(app.theme.text)), area);

    let l = &app.ui.layout;
    let mut constraints = Vec::new();
    if app.show_spaces { constraints.push(Constraint::Length(l.spaces_width.min(area.width / 3))); }
    if app.show_rooms { constraints.push(Constraint::Length(l.rooms_width.min(area.width / 2))); }
    constraints.push(Constraint::Min(10));
    let cols = Layout::horizontal(constraints).split(area);

    // clickable areas, used by the mouse until the next frame
    let mut hit = HitMap::default();
    let mut col = 0;
    if app.show_spaces {
        spaces::render(f, app, cols[col], &mut hit);
        col += 1;
    }
    if app.show_rooms {
        rooms::render(f, app, cols[col], &mut hit);
        col += 1;
    }
    render_main(f, app, cols[col], &mut hit);
    app.hit = hit;
    popup::render(f, app);
}

/// Login screen, before a session exists: just the login popup
pub fn render_login(f: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(t.background).fg(t.text)), area);
    if area.height > 2 {
        let footer = Rect { y: area.y + area.height - 1, height: 1, ..area };
        let text = truncate(&format!(" {} — Matrix client for the terminal · Ctrl+C quit", crate::config::APP_NAME), area.width as usize);
        f.render_widget(Paragraph::new(Span::styled(text, Style::default().fg(t.timestamp))), footer);
    }
    popup::render(f, app);
}

fn render_main(f: &mut Frame, app: &mut App, area: Rect, hit: &mut HitMap) {
    let [messages, errors, input] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(error_bar_height(app)),
        Constraint::Length(2),
    ])
    .areas(area);
    hit.chat = messages;
    hit.input = input;
    chat::render(f, app, messages);
    render_error_bar(f, app, errors);
    composer::render(f, app, input);
}

// ── Shared pieces ──────────────────────────────────────────────────────────

pub fn icon<'a>(app: &App, unicode: &'a str, ascii: &'a str) -> &'a str {
    if app.ui.layout.icons { unicode } else { ascii }
}

/// Column with a separator on the right
fn column_block(theme: &Theme) -> Block<'static> {
    Block::default().borders(Borders::RIGHT).border_style(Style::default().fg(theme.border))
}

/// Column header, highlighted when the panel has focus
fn header_line(theme: &Theme, title: &str, extra: &str, focused: bool, width: usize) -> Line<'static> {
    let style = if focused {
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
    };
    let title = truncate(title, width.saturating_sub(extra.width() + 3));
    Line::from(vec![
        Span::styled(format!(" {title}"), style),
        Span::styled(format!(" {extra}"), Style::default().fg(theme.muted)),
    ])
}

fn is_focused(app: &App, panel: Focus) -> bool {
    app.focus == panel && app.popup.is_none()
}

/// Clears an area keeping the theme background (for popups)
fn clear(f: &mut Frame, theme: &Theme, area: Rect) {
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(Style::default().bg(theme.background).fg(theme.text)), area);
}

/// "left … right" line padded to `width`
fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let lw: usize = left.iter().map(|s| s.content.width()).sum();
    let rw: usize = right.iter().map(|s| s.content.width()).sum();
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(width.saturating_sub(lw + rw))));
    spans.extend(right);
    Line::from(spans)
}

// ── Error bar ──────────────────────────────────────────────────────────────

/// 0 without errors, so the bar only appears when needed
fn error_bar_height(app: &App) -> u16 {
    match app.errors.len() {
        0 => 0,
        n => n.min(MAX_VISIBLE_ERRORS) as u16 + 2,
    }
}

fn render_error_bar(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 { return; }
    let t = &app.theme;
    let n = app.errors.len();
    let title = if n == 1 { " Error ".to_string() } else { format!(" Errors ({n}) ") };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(Style::default().fg(t.error))
        .title_top(Span::styled(title, Style::default().fg(t.error).add_modifier(Modifier::BOLD)))
        .title_top(Line::from(Span::styled(" Ctrl+E dismiss ", Style::default().fg(t.muted))).right_aligned());
    let width = block.inner(area).width as usize;
    let lines: Vec<Line> = app.errors.iter().skip(n.saturating_sub(MAX_VISIBLE_ERRORS)).map(|e| {
        let count = if e.count > 1 { format!(" ×{}", e.count) } else { String::new() };
        Line::from(vec![
            Span::styled(format!(" {} ", icon(app, "✗", "x")), Style::default().fg(t.error)),
            Span::styled(truncate(&e.msg, width.saturating_sub(4 + count.width())), Style::default().fg(t.text)),
            Span::styled(count, Style::default().fg(t.muted)),
        ])
    }).collect();
    f.render_widget(Paragraph::new(lines).block(block), area);
}
