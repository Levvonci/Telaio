//! Space bar: Home and the joined spaces.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::{column_block, header_line, icon, is_focused, text::truncate};
use crate::app::{App, Focus, HitMap};

pub fn render(f: &mut Frame, app: &App, area: Rect, hit: &mut HitMap) {
    let t = &app.theme;
    let block = column_block(t);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let focused = is_focused(app, Focus::Spaces);

    let mut lines = vec![header_line(t, "Spaces", "", focused, width), Line::default()];
    let first = lines.len() as u16;

    let names = std::iter::once((icon(app, "⌂", "~"), "Home".to_string()))
        .chain(app.spaces.iter().map(|s| (icon(app, "◆", "*"), s.name.clone())));
    for (idx, (ic, name)) in names.enumerate() {
        let selected = idx == app.space_idx;
        let (unread, highlights) = app.space_unread(idx);
        let badge = match (highlights, unread) {
            (0, 0) => String::new(),
            (0, _) => icon(app, "●", "*").to_string(),
            (h, _) => format!("@{h}"),
        };
        let name_w = width.saturating_sub(4 + badge.width() + 1);
        let name = truncate(&name, name_w);
        let pad = name_w.saturating_sub(name.width());

        let bg = if selected { t.surface } else { t.background };
        let marker = if selected { icon(app, "▌", ">") } else { " " };
        let name_style = match (selected, focused) {
            (true, true) => Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            (true, false) => Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            _ => Style::default().fg(t.text),
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(format!("{ic} "), Style::default().fg(t.muted)),
            Span::styled(name, name_style),
            Span::raw(" ".repeat(pad + 1)),
            Span::styled(badge, Style::default().fg(t.unread).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
        ]).style(Style::default().bg(bg)));
    }

    // keep the selected entry visible even with many spaces
    let height = inner.height as usize;
    let scroll = (app.space_idx + 3).saturating_sub(height);
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
    hit.spaces = area;
    hit.spaces_first = first;
    hit.spaces_scroll = scroll;
}
