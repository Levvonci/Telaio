//! Keyboard, paste and mouse → state changes and commands.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::path::PathBuf;

use crate::app::event::{Command, ModAction, VerifyStage};
use crate::app::input::LineInput;
use crate::app::popup::{ConfirmAction, LoginForm, PickerKind, Popup, TextKind};
use crate::app::{App, Entry, Focus};
use crate::config::expand_tilde;

/// Handles a terminal event. Returns true if the app should quit.
pub fn handle_event(event: Event, app: &mut App) -> bool {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => handle_key(key, app),
        Event::Paste(text) => {
            paste(&text, app);
            false
        }
        Event::Mouse(mouse) => {
            handle_mouse(mouse, app);
            false
        }
        _ => false,
    }
}

fn handle_key(key: KeyEvent, app: &mut App) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl {
        match key.code {
            KeyCode::Char('c') | KeyCode::Char('q') => return true,
            KeyCode::Char('e') => { app.errors.clear(); return false; }
            _ => {}
        }
    }

    if app.popup.is_some() {
        handle_popup(key, app);
        return false;
    }

    // shortcuts valid in every panel
    match key.code {
        KeyCode::F(1) => { app.popup = Some(Popup::Help); return false; }
        KeyCode::F(2) => { app.popup = Some(Popup::Account { cursor: 0 }); return false; }
        KeyCode::Char('s') if ctrl => { app.toggle_spaces(); return false; }
        KeyCode::Char('r') if ctrl => { app.toggle_rooms(); return false; }
        KeyCode::Char('k') if ctrl => { app.open_picker(PickerKind::Switch); return false; }
        KeyCode::Char('o') if ctrl => { open_attach(app); return false; }
        _ => {}
    }

    if app.is_insert() {
        handle_input(key, app);
        return false;
    }

    // NORMAL mode: keys shared by all panels
    match key.code {
        KeyCode::Tab => { app.cycle_focus(true); return false; }
        KeyCode::BackTab => { app.cycle_focus(false); return false; }
        KeyCode::Char('?') => { app.popup = Some(Popup::Help); return false; }
        KeyCode::Char('i') if !ctrl => { app.enter_insert(); return false; }
        // like vim's command line: enter insert mode with "/" already typed
        KeyCode::Char('/') | KeyCode::Char(':') => {
            app.enter_insert();
            if app.input.is_empty() { app.input.insert('/'); }
            return false;
        }
        _ => {}
    }

    match app.focus {
        Focus::Spaces => handle_spaces(key, app),
        Focus::Rooms => handle_rooms(key, app),
        Focus::Chat => handle_chat(key, app),
        Focus::Input => unreachable!("handled above"),
    }
    false
}

fn paste(text: &str, app: &mut App) {
    let text = text.replace(['\r', '\n'], " ");
    match app.popup.as_mut() {
        Some(Popup::Text(p)) => p.input.insert_str(text.trim()),
        Some(Popup::Login(form)) if !form.busy => form.input().insert_str(text.trim()),
        Some(Popup::Picker(p)) => {
            p.input.insert_str(text.trim());
            app.picker_changed();
        }
        Some(_) => {}
        None => {
            app.enter_insert();
            app.input.insert_str(&text);
        }
    }
}

/// Editing keys shared by all text fields. Returns true if handled.
fn edit_line(input: &mut LineInput, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('w') if ctrl => input.delete_word(),
        KeyCode::Char('u') if ctrl => input.clear(),
        KeyCode::Char('a') if ctrl => input.home(),
        KeyCode::Char(c) if !ctrl => input.insert(c),
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        KeyCode::Left => input.left(),
        KeyCode::Right => input.right(),
        KeyCode::Home => input.home(),
        KeyCode::End => input.end(),
        _ => return false,
    }
    true
}

fn no_room(app: &mut App) {
    app.push_error("Select a room first");
}

fn open_attach(app: &mut App) {
    if app.selected_room.is_some() { app.open_text(TextKind::Attach); } else { no_room(app); }
}

fn ask_leave(app: &mut App, room_id: String, name: String, is_space: bool) {
    let what = if is_space { "space" } else { "room" };
    app.popup = Some(Popup::Confirm {
        title: "Leave".into(),
        question: format!("Leave the {what} “{name}”?"),
        action: ConfirmAction::Leave { room_id },
    });
}

// ── Popup ──────────────────────────────────────────────────────────────────

fn handle_popup(key: KeyEvent, app: &mut App) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // its items depend on app state: handled before borrowing the popup
    if let Some(Popup::Account { cursor }) = app.popup {
        return handle_account(key, app, cursor);
    }
    let Some(popup) = app.popup.as_mut() else { return };

    if key.code == KeyCode::Esc {
        // closing a running verification cancels it on the other side too
        if let Popup::Verify(v) = popup {
            if !v.flow_id.is_empty() && !matches!(v.stage, VerifyStage::Done | VerifyStage::Cancelled(_)) {
                app.pending.push(Command::VerifyCancel { user_id: v.user_id.clone(), flow_id: v.flow_id.clone() });
            }
        }
        app.popup = None;
        return;
    }
    match popup {
        Popup::Help => app.popup = None,

        Popup::Account { .. } => unreachable!("handled above"),

        Popup::Verify(v) => {
            let (user_id, flow_id) = (v.user_id.clone(), v.flow_id.clone());
            let yes = matches!(key.code, KeyCode::Enter | KeyCode::Char('y'));
            let no = key.code == KeyCode::Char('n');
            match v.stage {
                VerifyStage::Incoming { .. } if yes => {
                    app.pending.push(Command::VerifyAccept { user_id, flow_id });
                    v.stage = VerifyStage::Waiting("Accepted, waiting for the other device…".into());
                }
                VerifyStage::Incoming { .. } if no => {
                    app.pending.push(Command::VerifyCancel { user_id, flow_id });
                    app.popup = None;
                }
                VerifyStage::Emojis(_) if yes => {
                    app.pending.push(Command::VerifyConfirm { user_id, flow_id });
                    v.stage = VerifyStage::Waiting("Waiting for confirmation on the other device".into());
                }
                VerifyStage::Emojis(_) if no => {
                    app.pending.push(Command::VerifyMismatch { user_id, flow_id });
                    v.stage = VerifyStage::Waiting("Cancelling…".into());
                }
                VerifyStage::Done | VerifyStage::Cancelled(_) => app.popup = None,
                _ => {}
            }
        }

        Popup::Login(form) => {
            if form.busy { return; }
            let n = LoginForm::FIELDS;
            match key.code {
                KeyCode::Tab | KeyCode::Down => form.field = (form.field + 1) % n,
                KeyCode::BackTab | KeyCode::Up => form.field = (form.field + n - 1) % n,
                KeyCode::Enter if form.field + 1 < n => form.field += 1,
                KeyCode::Enter => {
                    let username = form.user.as_str().trim().to_string();
                    let password = form.password.as_str().to_string();
                    if username.is_empty() {
                        form.error = Some("Enter the user (@user:server)".into());
                        form.field = 0;
                    } else if password.is_empty() {
                        form.error = Some("Enter the password".into());
                    } else {
                        form.busy = true;
                        form.error = None;
                        let homeserver = form.server.as_str().trim().to_string();
                        app.pending.push(Command::Login { homeserver, username, password });
                    }
                }
                _ => {
                    if edit_line(form.input(), key) { form.error = None; }
                }
            }
        }

        Popup::Confirm { action, .. } => match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                let cmd = action.clone().into_command();
                app.pending.push(cmd);
                app.popup = None;
            }
            KeyCode::Char('n') => app.popup = None,
            _ => {}
        },

        Popup::ManageSpaces { room_id, cursor } => match key.code {
            KeyCode::Up | KeyCode::Char('k') => *cursor = cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => *cursor = (*cursor + 1).min(app.spaces.len().saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(space) = app.spaces.get(*cursor) {
                    let add = !space.children.contains(room_id.as_str());
                    app.pending.push(Command::SetSpaceChild { space_id: space.id.clone(), room_id: room_id.clone(), add });
                }
            }
            _ => {}
        },

        Popup::Picker(picker) => match key.code {
            KeyCode::Enter => {
                if let Some(item) = picker.selected() { app.pick(item); }
            }
            KeyCode::Up => picker.move_cursor(-1),
            KeyCode::Down => picker.move_cursor(1),
            KeyCode::Char('p') if ctrl => picker.move_cursor(-1),
            KeyCode::Char('n') if ctrl => picker.move_cursor(1),
            _ => {
                if edit_line(&mut picker.input, key) { app.picker_changed(); }
            }
        },

        Popup::Text(p) => match key.code {
            KeyCode::Enter => {
                let (kind, value) = (p.kind, p.input.as_str().trim().to_string());
                app.popup = None;
                submit_text(kind, value, app);
            }
            KeyCode::Tab if p.kind == TextKind::Attach => {
                let (completed, candidates) = complete_path(p.input.as_str());
                p.input.set(&completed);
                p.completions = candidates;
            }
            _ => {
                if edit_line(&mut p.input, key) { p.completions.clear(); }
            }
        },
    }
}

fn submit_text(kind: TextKind, value: String, app: &mut App) {
    if value.is_empty() { return; }
    let room_id = app.selected_room.clone();
    let cmd = match kind {
        TextKind::CreateSpace => Command::CreateSpace { name: value },
        TextKind::CreateRoom => Command::CreateRoom { name: value, space_id: app.current_space().map(|s| s.id.clone()) },
        TextKind::Invite => match room_id {
            Some(room_id) => Command::Invite { room_id, user_id: value },
            None => return no_room(app),
        },
        TextKind::Attach => match room_id {
            Some(room_id) => {
                app.uploads += 1;
                Command::SendFile { room_id, path: clean_path(&value) }
            }
            None => return no_room(app),
        },
        TextKind::RecoveryKey => {
            app.set_status("Checking the recovery key…");
            Command::Recover { key: value }
        }
    };
    app.pending.push(cmd);
}

/// Account popup: ↑↓ to choose, Enter to run
fn handle_account(key: KeyEvent, app: &mut App, cursor: usize) {
    let items = app.account_items();
    let cursor = match key.code {
        KeyCode::Esc => { app.popup = None; return; }
        KeyCode::Up | KeyCode::Char('k') => cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => (cursor + 1).min(items.len().saturating_sub(1)),
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') => {
            if let Some(item) = items.get(cursor).cloned() { app.account_action(item); }
            return;
        }
        _ => cursor,
    };
    app.popup = Some(Popup::Account { cursor });
}

// ── Space bar ──────────────────────────────────────────────────────────────

fn handle_spaces(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.move_space(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_space(1),
        KeyCode::Home | KeyCode::Char('g') => app.select_space(0),
        KeyCode::End | KeyCode::Char('G') => app.select_space(app.spaces.len()),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            app.show_rooms = true;
            app.focus_panel(Focus::Rooms);
        }
        KeyCode::Char('n') => app.open_text(TextKind::CreateSpace),
        KeyCode::Char('a') => app.open_picker(PickerKind::Join),
        KeyCode::Char('L') => {
            if let Some(s) = app.current_space() {
                let (id, name) = (s.id.clone(), s.name.clone());
                ask_leave(app, id, name, true);
            }
        }
        _ => {}
    }
}

// ── Room list ──────────────────────────────────────────────────────────────

fn handle_rooms(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.move_room_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_room_cursor(1),
        KeyCode::Home | KeyCode::Char('g') => app.move_room_cursor(-(app.entries.len() as isize)),
        KeyCode::End | KeyCode::Char('G') => app.move_room_cursor(app.entries.len() as isize),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => match app.entries.get(app.room_cursor) {
            Some(Entry::Explore(_)) => {
                if let Some(h) = app.selected_explore() {
                    let address = h.address();
                    app.set_status(format!("Joining {}…", h.name));
                    app.pending.push(Command::Join { address });
                }
            }
            Some(Entry::Room(i)) => {
                let room = &app.rooms[*i];
                if room.invite.is_some() {
                    let address = room.id.clone();
                    app.set_status(format!("Accepting invite to {}…", room.name));
                    app.pending.push(Command::Join { address });
                } else {
                    app.enter_insert();
                }
            }
            _ => {}
        },
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') if app.show_spaces => app.focus_panel(Focus::Spaces),
        KeyCode::PageUp => scroll_up(app),
        KeyCode::PageDown => scroll_down(app),
        KeyCode::Char('a') => app.open_picker(PickerKind::Join),
        KeyCode::Char('d') => app.open_picker(PickerKind::Dm),
        KeyCode::Char('n') => app.open_text(TextKind::CreateRoom),
        KeyCode::Char('f') => open_attach(app),
        KeyCode::Char('I') => {
            if app.selected_room.is_some() { app.open_text(TextKind::Invite); } else { no_room(app); }
        }
        KeyCode::Char('m') => match &app.selected_room {
            Some(_) if app.spaces.is_empty() => app.push_error("You are not in any space: create one from the space bar (n)"),
            Some(id) => app.popup = Some(Popup::ManageSpaces { room_id: id.clone(), cursor: 0 }),
            None => no_room(app),
        },
        KeyCode::Char('L') => {
            if let Some(r) = app.current_room() {
                let (id, name, invite) = (r.id.clone(), r.name.clone(), r.invite.is_some());
                if invite {
                    app.popup = Some(Popup::Confirm {
                        title: "Decline invite".into(),
                        question: format!("Decline the invite to “{name}”?"),
                        action: ConfirmAction::Leave { room_id: id },
                    });
                } else {
                    ask_leave(app, id, name, false);
                }
            }
        }
        _ => {}
    }
}

// ── Messages ───────────────────────────────────────────────────────────────

fn page(app: &App) -> isize { (app.chat_view.height / 2).max(3) as isize }

fn scroll_up(app: &mut App) { app.scroll_chat(page(app)); }

fn scroll_down(app: &mut App) { app.scroll_chat(-page(app)); }

/// Selects the previous message; past the first loaded one, requests older history
fn select_older(app: &mut App) {
    let count = app.current_timeline().map(|t| t.messages.len()).unwrap_or(0);
    match app.selected_msg {
        _ if count == 0 => {}
        None => app.selected_msg = Some(count - 1),
        Some(0) => app.load_older(),
        Some(i) => app.selected_msg = Some(i - 1),
    }
}

/// Selects the next message; past the last one, returns to the live view
fn select_newer(app: &mut App) {
    let count = app.current_timeline().map(|t| t.messages.len()).unwrap_or(0);
    if let Some(i) = app.selected_msg {
        app.selected_msg = (i + 1 < count).then_some(i + 1);
        if app.selected_msg.is_none() { app.chat_view.scroll = 0; }
    }
}

fn handle_chat(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => { app.selected_msg = None; }
        KeyCode::Up | KeyCode::Char('k') => select_older(app),
        KeyCode::Down | KeyCode::Char('j') => select_newer(app),
        KeyCode::PageUp => scroll_up(app),
        KeyCode::PageDown => scroll_down(app),
        KeyCode::End | KeyCode::Char('G') => { app.selected_msg = None; app.chat_view.scroll = 0; }
        KeyCode::Enter | KeyCode::Char('o') => attachment_action(app, true),
        KeyCode::Char('s') => attachment_action(app, false),
        KeyCode::Char('x') | KeyCode::Delete => ask_redact(app),
        KeyCode::Char('f') => open_attach(app),
        _ => {}
    }
}

fn ask_redact(app: &mut App) {
    let (Some(room_id), Some(i)) = (app.selected_room.clone(), app.selected_msg) else {
        return app.set_status("Select a message with ↑↓");
    };
    let Some(msg) = app.current_timeline().and_then(|t| t.messages.get(i)) else { return };
    let preview: String = msg.preview().chars().take(60).collect();
    let question = if msg.sender == app.user_id {
        format!("Delete your message “{preview}”?")
    } else {
        format!("Delete the message from {} “{preview}”?", msg.sender_name)
    };
    let event_id = msg.event_id.clone();
    app.popup = Some(Popup::Confirm {
        title: "Delete message".into(),
        question,
        action: ConfirmAction::Redact { room_id, event_id },
    });
}

/// Opens (`open`) or saves the attachment of the selected message
fn attachment_action(app: &mut App, open: bool) {
    let att = app.selected_msg
        .and_then(|i| app.current_timeline()?.messages.get(i))
        .and_then(|m| m.attachment().cloned());
    match att {
        Some(attachment) if open => {
            app.set_status(format!("Opening {}…", attachment.name));
            app.pending.push(Command::Open { attachment });
        }
        Some(attachment) => {
            app.set_status(format!("Downloading {}…", attachment.name));
            app.pending.push(Command::Download { attachment });
        }
        None if app.selected_msg.is_some() => app.set_status("The selected message has no attachment"),
        None => app.set_status("Select a message with ↑↓"),
    }
}

// ── Input line ─────────────────────────────────────────────────────────────

/// INSERT mode: everything goes to the text field, Esc returns to normal mode
fn handle_input(key: KeyEvent, app: &mut App) {
    match key.code {
        // with a message selected, move to Messages keeping the selection
        // (s saves, x deletes); otherwise back to the last panel
        KeyCode::Esc if app.selected_msg.is_some() => app.focus_panel(Focus::Chat),
        KeyCode::Esc => app.enter_normal(),
        KeyCode::Tab | KeyCode::BackTab => {}
        KeyCode::PageUp => scroll_up(app),
        KeyCode::PageDown => scroll_down(app),
        // single-line input: arrows select messages like in the Messages panel
        KeyCode::Up => select_older(app),
        KeyCode::Down => select_newer(app),
        // with an empty field, Enter opens the selected message's attachment
        KeyCode::Enter if app.input.is_empty() && app.selected_msg.is_some() => attachment_action(app, true),
        KeyCode::Enter => {
            app.selected_msg = None;
            submit_input(app);
        }
        _ => { edit_line(&mut app.input, key); }
    }
}

/// Sends the typed text, interpreting commands starting with "/"
fn submit_input(app: &mut App) {
    let text = app.input.as_str().trim().to_string();
    if text.is_empty() { return; }
    let (cmd, arg) = text.split_once(' ').map(|(c, a)| (c, a.trim())).unwrap_or((text.as_str(), ""));
    let room_id = app.selected_room.clone();

    match cmd {
        "/file" => match (room_id, arg) {
            (None, _) => return no_room(app),
            (Some(_), "") => app.open_text(TextKind::Attach),
            (Some(room_id), path) => {
                app.uploads += 1;
                app.pending.push(Command::SendFile { room_id, path: clean_path(path) });
            }
        },
        "/join" => {
            app.open_picker(PickerKind::Join);
            if let Some(Popup::Picker(p)) = app.popup.as_mut() { p.input.set(arg); }
            app.picker_changed();
        }
        "/dm" => {
            app.open_picker(PickerKind::Dm);
            if let Some(Popup::Picker(p)) = app.popup.as_mut() { p.input.set(arg); }
            app.picker_changed();
        }
        "/invite" => match (room_id, arg) {
            (None, _) => return no_room(app),
            (Some(_), "") => return app.push_error("Usage: /invite @user:server"),
            (Some(room_id), user) => app.pending.push(Command::Invite { room_id, user_id: user.to_string() }),
        },
        "/leave" => match app.current_room() {
            Some(r) => {
                let (id, name) = (r.id.clone(), r.name.clone());
                ask_leave(app, id, name, false);
            }
            None => return no_room(app),
        },
        "/kick" | "/ban" | "/unban" => {
            let Some(room_id) = room_id else { return no_room(app) };
            let action = match cmd {
                "/ban" => ModAction::Ban,
                "/unban" => ModAction::Unban,
                _ => ModAction::Kick,
            };
            let (target, reason) = arg.split_once(' ').map(|(t, r)| (t, Some(r.trim().to_string()))).unwrap_or((arg, None));
            if target.is_empty() {
                return app.push_error(format!("Usage: {cmd} @user:server [reason]"));
            }
            let room = app.current_room().map(|r| r.name.clone()).unwrap_or_default();
            let (title, verb, prep) = match action {
                ModAction::Kick => ("Kick", "kick", "from"),
                ModAction::Ban => ("Ban", "ban", "from"),
                ModAction::Unban => ("Unban", "unban", "in"),
            };
            let because = reason.as_deref().map(|r| format!(" (reason: {r})")).unwrap_or_default();
            app.popup = Some(Popup::Confirm {
                title: title.into(),
                question: format!("{verb} {target} {prep} “{room}”?{because}", verb = capitalize(verb)),
                action: ConfirmAction::Moderate { room_id, action, target: target.to_string(), reason: reason.filter(|r| !r.is_empty()) },
            });
        }
        "/topic" => match room_id {
            None => return no_room(app),
            Some(room_id) => app.pending.push(Command::SetTopic { room_id, topic: arg.to_string() }),
        },
        "/name" => match (room_id, arg) {
            (None, _) => return no_room(app),
            (Some(_), "") => return app.push_error("Usage: /name new name"),
            (Some(room_id), name) => app.pending.push(Command::SetName { room_id, name: name.to_string() }),
        },
        "/help" => app.popup = Some(Popup::Help),
        "/account" => app.popup = Some(Popup::Account { cursor: 0 }),
        "/verify" => app.start_verification(),
        "/recovery" => app.open_text(TextKind::RecoveryKey),
        _ => {
            let Some(room_id) = room_id else { return no_room(app) };
            if app.current_room().is_some_and(|r| r.invite.is_some()) {
                return app.push_error("Accept the invite first (Enter on the room in the list)");
            }
            // "//text" sends "/text" literally
            let body = match text.strip_prefix("//") {
                Some(rest) => format!("/{rest}"),
                None if text.starts_with('/') => {
                    return app.push_error(format!("Unknown command: {cmd} (use // to send text starting with /)"));
                }
                None => text.clone(),
            };
            app.pending.push(Command::SendText { room_id, body });
            app.chat_view.scroll = 0;
            app.selected_msg = None;
        }
    }
    app.input.clear();
}

// ── Mouse ──────────────────────────────────────────────────────────────────

/// Lines scrolled per wheel notch
const WHEEL_LINES: isize = 3;

fn contains(area: Rect, col: u16, row: u16) -> bool {
    area.contains(Position { x: col, y: row })
}

fn handle_mouse(m: MouseEvent, app: &mut App) {
    let (col, row) = (m.column, m.row);
    match m.kind {
        // the wheel always scrolls messages, except over the two side bars
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            if app.popup.is_some() || contains(app.hit.spaces, col, row) || contains(app.hit.rooms, col, row) {
                return;
            }
            let up = m.kind == MouseEventKind::ScrollUp;
            app.scroll_chat(if up { WHEEL_LINES } else { -WHEEL_LINES });
        }
        MouseEventKind::Down(MouseButton::Left) if app.popup.is_none() => click(app, col, row),
        _ => {}
    }
}

/// Click: selects a space or room, or focuses the panel
fn click(app: &mut App, col: u16, row: u16) {
    let hit = &app.hit;
    if contains(hit.spaces, col, row) {
        let line = (row - hit.spaces.y) as usize + hit.spaces_scroll;
        let idx = line.checked_sub(hit.spaces_first as usize);
        app.focus_panel(Focus::Spaces);
        if let Some(idx) = idx.filter(|&i| i <= app.spaces.len()) {
            app.select_space(idx);
        }
    } else if contains(hit.rooms, col, row) {
        let line = (row - hit.rooms.y) as usize + hit.rooms_scroll;
        let entry = hit.room_lines.get(line).copied().flatten();
        app.focus_panel(Focus::Rooms);
        if let Some(entry) = entry {
            app.room_cursor = entry;
            app.move_room_cursor(0);
        }
    } else if contains(hit.input, col, row) {
        app.enter_insert();
    } else if contains(hit.chat, col, row) {
        app.focus_panel(Focus::Chat);
    }
}

// ── File paths ─────────────────────────────────────────────────────────────

/// User-typed path: expands ~ and strips the quotes some terminals add when
/// a file is dragged into the window
pub fn clean_path(raw: &str) -> PathBuf {
    let s = raw.trim();
    let s = s.strip_prefix("file://").unwrap_or(s);
    let s = s
        .strip_prefix('\'').and_then(|x| x.strip_suffix('\''))
        .or_else(|| s.strip_prefix('"').and_then(|x| x.strip_suffix('"')))
        .unwrap_or(s);
    expand_tilde(&s.replace("\\ ", " "))
}

/// Shell-style completion: text completed up to the common prefix, plus the
/// candidates if there is more than one
fn complete_path(input: &str) -> (String, Vec<String>) {
    let (dir_part, file_part) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };
    let dir = if dir_part.is_empty() { PathBuf::from(".") } else { expand_tilde(dir_part) };
    let Ok(entries) = std::fs::read_dir(&dir) else { return (input.to_string(), vec![]) };

    let show_hidden = file_part.starts_with('.');
    let mut matches: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(file_part) || (!show_hidden && name.starts_with('.')) { return None; }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            Some(if is_dir { format!("{name}/") } else { name })
        })
        .collect();
    matches.sort();

    match matches.len() {
        0 => (input.to_string(), vec![]),
        1 => (format!("{dir_part}{}", matches[0]), vec![]),
        _ => (format!("{dir_part}{}", common_prefix(&matches)), matches),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn common_prefix(items: &[String]) -> String {
    let first = &items[0];
    let mut end = items[1..].iter()
        .map(|s| first.bytes().zip(s.bytes()).take_while(|(a, b)| a == b).count())
        .min()
        .unwrap_or(first.len());
    while !first.is_char_boundary(end) { end -= 1; }
    first[..end].to_string()
}

/// Description of the file typed in the attach popup: (text, exists)
pub fn describe_path(raw: &str) -> Option<(String, bool)> {
    if raw.trim().is_empty() { return None; }
    let path = clean_path(raw);
    Some(match std::fs::metadata(&path) {
        Ok(m) if m.is_file() => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (format!("{} · {mime}", crate::matrix::human_size(m.len())), true)
        }
        Ok(m) if m.is_dir() => ("folder — Tab to browse".into(), false),
        _ => ("file not found".into(), false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_paths() {
        assert_eq!(clean_path("'/tmp/a b.png'"), PathBuf::from("/tmp/a b.png"));
        assert_eq!(clean_path("file:///tmp/x"), PathBuf::from("/tmp/x"));
        assert_eq!(clean_path("/tmp/a\\ b"), PathBuf::from("/tmp/a b"));
    }

    #[test]
    fn prefix() {
        let v = vec!["documents.pdf".to_string(), "documented/".to_string()];
        assert_eq!(common_prefix(&v), "document");
    }
}
