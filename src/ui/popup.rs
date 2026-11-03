//! Popups.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::{clear, composer::render_line_input, icon, spread, text::truncate};
use crate::app::event::{RecoveryStatus, VerifyStage};
use crate::app::input::LineInput;
use crate::app::popup::{AccountItem, LoginForm, Picker, PickerKind, Popup, TextKind, TextPopup, VerifyPopup};
use crate::app::App;
use crate::keys::describe_path;

pub fn render(f: &mut Frame, app: &App) {
    match &app.popup {
        None => {}
        Some(Popup::Help) => help(f, app),
        Some(Popup::Text(p)) => text(f, app, p),
        Some(Popup::Picker(p)) => picker(f, app, p),
        Some(Popup::ManageSpaces { room_id, cursor }) => manage_spaces(f, app, room_id, *cursor),
        Some(Popup::Confirm { title, question, .. }) => confirm(f, app, title, question),
        Some(Popup::Account { cursor }) => account(f, app, *cursor),
        Some(Popup::Login(form)) => login(f, app, form),
        Some(Popup::Verify(v)) => verify(f, app, v),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h)
}

/// Shared frame: title top-left, hint bottom-right
fn frame(f: &mut Frame, app: &App, width: u16, height: u16, title: &str, hint: &str) -> Rect {
    let t = &app.theme;
    let area = centered(f.area(), width, height);
    clear(f, t, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(t.border_type)
        .border_style(Style::default().fg(t.border))
        .title_top(Span::styled(format!(" {title} "), Style::default().fg(t.text).add_modifier(Modifier::BOLD)))
        .title_bottom(Line::from(Span::styled(format!(" {hint} "), Style::default().fg(t.muted))).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

/// Text field with a "›" prompt; places the cursor
fn input_row(f: &mut Frame, app: &App, input: &LineInput, area: Rect, placeholder: &str) {
    let t = &app.theme;
    f.render_widget(Paragraph::new(Span::styled(format!(" {} ", icon(app, "›", ">")), Style::default().fg(t.accent))), area);
    let field = Rect { x: area.x + 3, width: area.width.saturating_sub(4), ..area };
    field_input(f, app, input, field, placeholder, true);
}

/// Field text (or placeholder); with `active`, places the cursor
fn field_input(f: &mut Frame, app: &App, input: &LineInput, field: Rect, placeholder: &str, active: bool) {
    let t = &app.theme;
    if input.is_empty() {
        f.render_widget(Paragraph::new(Span::styled(truncate(placeholder, field.width as usize), Style::default().fg(t.timestamp))), field);
    }
    let style = if active { Style::default().fg(t.text).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.muted) };
    let x = render_line_input(f, input, field, style, active);
    if active { f.set_cursor_position((x, field.y)); }
}

/// Selectable list entry: marker, text and right-aligned tag
fn item_line(app: &App, text: &str, tag: &str, selected: bool, width: usize) -> Line<'static> {
    let t = &app.theme;
    let marker = if selected { icon(app, "▌", ">") } else { " " };
    let style = if selected { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.text) };
    spread(
        vec![
            Span::styled(marker.to_string(), Style::default().fg(t.accent)),
            Span::styled(format!(" {}", truncate(text, width.saturating_sub(tag.width() + 4))), style),
        ],
        vec![Span::styled(format!("{tag} "), Style::default().fg(t.muted))],
        width,
    )
    .style(Style::default().bg(if selected { t.surface } else { t.background }))
}

fn text(f: &mut Frame, app: &App, p: &TextPopup) {
    let t = &app.theme;
    let room = app.current_room().map(|r| r.name.clone()).unwrap_or_default();
    let (title, placeholder) = match p.kind {
        TextKind::CreateSpace => ("New space".to_string(), "space name".to_string()),
        TextKind::CreateRoom => match app.current_space() {
            Some(s) => (format!("New room in {}", s.name), "room name".into()),
            None => ("New room".into(), "room name".into()),
        },
        TextKind::Invite => (format!("Invite to {room}"), "@user:server".into()),
        TextKind::Attach => (format!("Attach to {room}"), "file path — Tab completes".into()),
        TextKind::RecoveryKey => ("Recovery key".into(), "paste the recovery key or passphrase".into()),
    };

    let mut extra: Vec<Line> = Vec::new();
    if p.kind == TextKind::Attach {
        if let Some((desc, ok)) = describe_path(p.input.as_str()) {
            extra.push(Line::from(Span::styled(format!("   {desc}"), Style::default().fg(if ok { t.success } else { t.muted }))));
        }
        const MAX: usize = 8;
        for c in p.completions.iter().take(MAX) {
            let color = if c.ends_with('/') { t.accent } else { t.text };
            extra.push(Line::from(Span::styled(format!("   {c}"), Style::default().fg(color))));
        }
        if p.completions.len() > MAX {
            extra.push(Line::from(Span::styled(format!("   … {} more", p.completions.len() - MAX), Style::default().fg(t.muted))));
        }
    }

    if p.kind == TextKind::RecoveryKey {
        extra.push(Line::from(Span::styled("   The one saved when you set up key backup (e.g. in Element)", Style::default().fg(t.muted))));
    }

    let inner = frame(f, app, 64, 4 + extra.len() as u16, &title, "Enter confirm · Esc cancel");
    let [row, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    let shown = if p.kind.masked() { p.input.masked() } else { p.input.clone() };
    input_row(f, app, &shown, row, &placeholder);
    f.render_widget(Paragraph::new(extra), Rect { y: row.y + 1, height: rest.height + 1, ..rest });
}

fn picker(f: &mut Frame, app: &App, p: &Picker) {
    let t = &app.theme;
    let (title, placeholder, empty) = match p.kind {
        PickerKind::Join => (
            "Join a room or space",
            "#alias:server, !id:server, matrix.to link or directory search",
            "Type an address or at least 2 letters to search",
        ),
        PickerKind::Dm => ("New chat", "@user:server or search by name", "Type an MXID or at least 2 letters to search"),
        PickerKind::Switch => ("Go to…", "room name", "No matching room"),
    };
    let items = p.items();
    let height = 16u16;
    let inner = frame(f, app, 72, height, title, "↑↓ choose · Enter open · Esc close");
    let [row, sep, list] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    input_row(f, app, &p.input, row, placeholder);
    f.render_widget(Paragraph::new(Span::styled("─".repeat(sep.width as usize), Style::default().fg(t.timestamp))), sep);

    let width = list.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    if items.is_empty() {
        let msg = if p.searching || p.edited_at.is_some() { "searching…" } else if p.input.is_empty() || !p.wants_remote() { empty } else { "no results" };
        lines.push(Line::from(Span::styled(format!("  {msg}"), Style::default().fg(t.muted))));
    }
    // each result takes two lines: title and detail
    let visible = (list.height as usize / 2).max(1);
    let first = (p.cursor + 1).saturating_sub(visible);
    for (i, item) in items.iter().enumerate().skip(first).take(visible) {
        let selected = i == p.cursor;
        let bg = if selected { t.surface } else { t.background };
        let marker = if selected { icon(app, "▌", ">") } else { " " };
        let title_style = if selected { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.text) };
        let tag_w = item.tag.width();
        lines.push(spread(
            vec![
                Span::styled(marker.to_string(), Style::default().fg(t.accent)),
                Span::styled(format!(" {}", truncate(&item.title, width.saturating_sub(tag_w + 4))), title_style),
            ],
            vec![Span::styled(format!("{} ", item.tag), Style::default().fg(t.muted))],
            width,
        ).style(Style::default().bg(bg)));
        let sub = truncate(&item.subtitle, width.saturating_sub(3));
        let pad = width.saturating_sub(sub.width() + 2);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(sub, Style::default().fg(t.timestamp)),
            Span::raw(" ".repeat(pad)),
        ]).style(Style::default().bg(bg)));
    }
    if !items.is_empty() && p.searching {
        lines.push(Line::from(Span::styled("  searching…", Style::default().fg(t.muted))));
    }
    f.render_widget(Paragraph::new(lines), list);
}

fn manage_spaces(f: &mut Frame, app: &App, room_id: &str, cursor: usize) {
    let t = &app.theme;
    let room = app.rooms.iter().find(|r| r.id == room_id).map(|r| r.name.clone()).unwrap_or_default();
    let height = (app.spaces.len() as u16 + 4).min(20);
    let inner = frame(f, app, 56, height, &format!("Spaces of {room}"), "Space/Enter add or remove · Esc close");
    let width = inner.width as usize;
    let mut lines = vec![Line::from(Span::styled(" The room appears in the checked spaces", Style::default().fg(t.muted))), Line::default()];
    for (i, space) in app.spaces.iter().enumerate() {
        let selected = i == cursor;
        let member = space.children.contains(room_id);
        let check = match (member, app.ui.layout.icons) {
            (true, true) => "◉",
            (false, true) => "○",
            (true, false) => "[x]",
            (false, false) => "[ ]",
        };
        let style = if selected { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.text) };
        let line = format!(" {check} {}", truncate(&space.name, width.saturating_sub(6)));
        let pad = width.saturating_sub(line.width());
        lines.push(Line::from(vec![
            Span::styled(line, style),
            Span::raw(" ".repeat(pad)),
        ]).style(Style::default().bg(if selected { t.surface } else { t.background })));
    }
    let scroll = (cursor + 3).saturating_sub(inner.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}

fn account(f: &mut Frame, app: &App, cursor: usize) {
    let t = &app.theme;
    let sec = &app.security;
    let (mark, state, color) = match sec.verified {
        Some(true) => (icon(app, "✓", "v"), "session verified", t.success),
        Some(false) => (icon(app, "✗", "x"), "session not verified", t.error),
        None => (icon(app, "…", "."), "checking session…", t.muted),
    };
    let backup = match sec.recovery {
        RecoveryStatus::Enabled => "enabled",
        RecoveryStatus::Incomplete => "unlock it with the recovery key",
        RecoveryStatus::Disabled => "not set up on this account",
        RecoveryStatus::Unknown => "…",
    };
    let items = app.account_items();
    let width = 66u16;
    let w = width as usize - 2;

    let mut lines = vec![
        Line::from(Span::styled(format!(" {}", truncate(&app.user_id, w - 2)), Style::default().fg(t.text).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled(format!(" {mark} {state}"), Style::default().fg(color)),
            Span::styled(if app.device_id.is_empty() { String::new() } else { format!(" · device {}", app.device_id) }, Style::default().fg(t.muted)),
        ]),
        Line::from(Span::styled(format!(" key backup: {backup}"), Style::default().fg(t.muted))),
        Line::default(),
    ];
    let section = |name: &str| Line::from(Span::styled(format!(" {name}"), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)));
    lines.push(section("Verification"));
    let mut selected_line = 0;
    for (i, item) in items.iter().enumerate() {
        if i == 2 {
            lines.push(Line::default());
            lines.push(section("Account"));
        }
        let (text, tag) = match item {
            AccountItem::VerifyDevice => (
                "Verify with another device (emoji)".to_string(),
                if sec.verified == Some(true) { "done".into() } else { String::new() },
            ),
            AccountItem::RecoveryKey => (
                "Enter the recovery key".to_string(),
                if sec.recovery == RecoveryStatus::Enabled { "done".into() } else { String::new() },
            ),
            AccountItem::Switch(name) => {
                let saved = app.accounts.iter().any(|a| &a.username == name && a.has_session);
                (format!("Switch to {name}"), if saved { String::new() } else { "password".into() })
            }
            AccountItem::AddAccount => ("Add an account".to_string(), String::new()),
            AccountItem::Logout => ("Log out of this account".to_string(), String::new()),
        };
        if i == cursor { selected_line = lines.len(); }
        lines.push(item_line(app, &text, &tag, i == cursor, w));
    }

    let height = (lines.len() as u16 + 2).min(f.area().height.saturating_sub(2));
    let inner = frame(f, app, width, height, "Account", "↑↓ choose · Enter · Esc close");
    let scroll = (selected_line + 1).saturating_sub(inner.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}

fn login(f: &mut Frame, app: &App, form: &LoginForm) {
    let t = &app.theme;
    let inner = frame(f, app, 64, 8, "Sign in", "Tab field · Enter next/sign in · Esc cancel");
    let masked = form.password.masked();
    let rows: [(&str, &LineInput, &str); 3] = [
        ("User", &form.user, "@user:server"),
        ("Server", &form.server, "optional: derived from the user"),
        ("Password", &masked, ""),
    ];
    const LABEL: u16 = 11;
    for (i, (label, input, placeholder)) in rows.into_iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.y + inner.height { break; }
        let active = i == form.field && !form.busy;
        let style = if active { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.muted) };
        f.render_widget(Paragraph::new(Span::styled(format!(" {label}"), style)), Rect { y, height: 1, ..inner });
        let field = Rect { x: inner.x + LABEL, y, width: inner.width.saturating_sub(LABEL + 1), height: 1 };
        field_input(f, app, input, field, placeholder, active);
    }
    let (msg, color) = match (&form.error, form.busy) {
        (_, true) => ("Signing in…".to_string(), t.muted),
        (Some(e), _) => (e.clone(), t.error),
        (None, _) => (String::new(), t.muted),
    };
    if inner.height > 4 {
        let row = Rect { y: inner.y + 4, height: inner.height - 4, ..inner };
        f.render_widget(Paragraph::new(Span::styled(format!(" {}", truncate(&msg, inner.width as usize - 2)), Style::default().fg(color))), row);
    }
}

fn verify(f: &mut Frame, app: &App, v: &VerifyPopup) {
    let t = &app.theme;
    let own = v.user_id == app.user_id;
    let text = |s: String, color| Line::from(Span::styled(format!(" {s}"), Style::default().fg(color)));
    let (lines, hint): (Vec<Line>, &str) = match &v.stage {
        VerifyStage::Incoming { device } => (
            vec![
                Line::default(),
                text(if own {
                    format!("Another of your devices ({device}) wants to verify this session.")
                } else {
                    format!("{} ({device}) wants to verify your identity.", v.user_id)
                }, t.text),
            ],
            "Enter accept · n decline",
        ),
        VerifyStage::Waiting(msg) => (vec![Line::default(), text(msg.clone(), t.muted)], "Esc cancel"),
        VerifyStage::Emojis(list) => return emojis(f, app, list),
        VerifyStage::Done => (
            vec![Line::default(), text(format!("{} {}", icon(app, "✓", "v"), if own { "Session verified".into() } else { format!("{} verified", v.user_id) }), t.success)],
            "any key to close",
        ),
        VerifyStage::Cancelled(reason) => (
            vec![Line::default(), text(format!("{} Verification failed: {reason}", icon(app, "✗", "x")), t.error)],
            "any key to close",
        ),
    };
    let inner = frame(f, app, 72, 5, "Session verification", hint);
    let wrapped: Vec<Line> = lines.into_iter().map(|l| {
        let s: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
        let style = l.spans.first().map(|s| s.style).unwrap_or_default();
        Line::from(Span::styled(truncate(&s, inner.width as usize), style))
    }).collect();
    f.render_widget(Paragraph::new(wrapped), inner);
}

/// The seven emojis to compare: a grid if there is room, otherwise a column
fn emojis(f: &mut Frame, app: &App, list: &[(String, String)]) {
    let t = &app.theme;
    let hint = "Enter they match · n they don't · Esc cancel";
    let question = " Does the other device show the same emojis, in the same order?";
    const COL: u16 = 12;
    let grid_w = COL * list.len().max(1) as u16 + 2;
    if f.area().width >= grid_w + 2 {
        let inner = frame(f, app, grid_w.max(question.width() as u16 + 3), 8, "Compare emojis", hint);
        f.render_widget(Paragraph::new(Span::styled(truncate(question, inner.width as usize), Style::default().fg(t.text))), inner);
        let x0 = inner.x + inner.width.saturating_sub(COL * list.len() as u16) / 2;
        for (i, (symbol, name)) in list.iter().enumerate() {
            let x = x0 + COL * i as u16;
            // one area per column: an emoji of unexpected width doesn't shift the others
            for (dy, s, style) in [
                (2, symbol.as_str(), Style::default().fg(t.text)),
                (3, name.as_str(), Style::default().fg(t.muted)),
            ] {
                let area = Rect { x, y: inner.y + dy, width: COL, height: 1 }.intersection(inner);
                f.render_widget(Paragraph::new(Span::styled(truncate(s, COL as usize - 1), style)).centered(), area);
            }
        }
        return;
    }
    let inner = frame(f, app, 48, list.len() as u16 + 4, "Compare emojis", hint);
    let mut lines = vec![Line::from(Span::styled(truncate(question, inner.width as usize), Style::default().fg(t.text))), Line::default()];
    lines.extend(list.iter().map(|(symbol, name)| Line::from(vec![
        Span::styled(format!("   {symbol}  "), Style::default().fg(t.text)),
        Span::styled(name.clone(), Style::default().fg(t.muted)),
    ])));
    f.render_widget(Paragraph::new(lines), inner);
}

fn confirm(f: &mut Frame, app: &App, title: &str, question: &str) {
    let t = &app.theme;
    let inner = frame(f, app, 56, 5, title, "Enter confirm · Esc cancel");
    f.render_widget(
        Paragraph::new(vec![Line::default(), Line::from(Span::styled(format!(" {}", truncate(question, inner.width as usize - 2)), Style::default().fg(t.text)))]),
        inner,
    );
}

fn help(f: &mut Frame, app: &App) {
    let t = &app.theme;
    let section = |name: &'static str| Line::from(Span::styled(format!(" {name}"), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)));
    let key = |k: &'static str, d: &'static str| Line::from(vec![
        Span::styled(format!("   {k:<18}"), Style::default().fg(t.text).add_modifier(Modifier::BOLD)),
        Span::styled(d, Style::default().fg(t.muted)),
    ]);
    let lines = vec![
        section("Modes (like vim)"),
        key("i  Enter", "INSERT: keys go into the text field"),
        key("Esc", "NORMAL: keys are commands"),
        key("/  :", "type a command (/kick, /file, …)"),
        Line::default(),
        section("General (normal mode)"),
        key("Tab / Shift+Tab", "next / previous panel"),
        key("Ctrl+S / Ctrl+R", "toggle space bar / room bar"),
        key("Ctrl+K", "go to a room"),
        key("Ctrl+O", "attach a file"),
        key("Ctrl+E", "dismiss errors"),
        key("wheel / click", "scroll messages / select"),
        key("Shift+drag", "select text in the terminal"),
        key("F1  ?", "this help"),
        key("F2", "account: verification, switching"),
        key("Ctrl+C", "quit"),
        Line::default(),
        section("Spaces"),
        key("↑↓  j k", "choose a space"),
        key("n", "create a space"),
        key("a", "join a room or space"),
        key("L", "leave the space"),
        Line::default(),
        section("Rooms"),
        key("↑↓  j k", "open a room"),
        key("Enter", "write / join (rooms to explore)"),
        key("a  d", "join by address / new chat"),
        key("n", "create a room (in the current space)"),
        key("m", "add/remove the room from spaces"),
        key("I  L", "invite / leave the room"),
        Line::default(),
        section("Messages and writing"),
        key("↑↓  Enter", "select a message / open attachment"),
        key("s", "save the attachment to Matrix_Download"),
        key("x", "delete the selected message"),
        key("PgUp PgDn End", "scroll / back to latest"),
        key("↑↓ in insert", "select a message (Enter opens)"),
        key("/file /join /dm", "attach, join, new chat"),
        key("/invite /leave", "invite, leave"),
        key("/kick /ban /unban", "@user [reason] — with permission"),
        key("/topic /name", "change room topic / name"),
        key("/verify /recovery", "emoji verification / recovery key"),
        key("Ctrl+W  Ctrl+U", "delete word / line"),
    ];
    // if it doesn't fit vertically and there is room, split into two columns at a section break
    let screen = f.area();
    let two_columns = lines.len() as u16 + 2 > screen.height && screen.width >= 130;
    if !two_columns {
        let inner = frame(f, app, 66, lines.len() as u16 + 2, "Keys", "any key to close");
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }
    let half = lines.len() / 2;
    let split = (half..lines.len()).find(|&i| lines[i].spans.is_empty()).unwrap_or(half);
    let (left, right) = lines.split_at(split);
    let right: Vec<Line> = right.iter().skip(1).cloned().collect();
    let height = left.len().max(right.len()) as u16 + 2;
    let inner = frame(f, app, 128, height, "Keys", "any key to close");
    let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);
    f.render_widget(Paragraph::new(left.to_vec()), l);
    f.render_widget(Paragraph::new(right), r);
}
