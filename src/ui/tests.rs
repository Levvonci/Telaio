//! Rendering tests on a virtual terminal with sample data.

use chrono::Utc;
use matrix_sdk::ruma::{events::room::MediaSource, OwnedMxcUri};
use ratatui::{backend::TestBackend, Terminal};

use super::render;
use crate::app::event::SyncState;
use crate::app::event::{AppEvent, Command, RecoveryStatus, Security, VerifyStage};
use crate::app::model::{AccountEntry, Attachment, AttachmentKind, Content, HierarchyEntry, Message, Room, Space, Undecrypted};
use crate::app::popup::{LoginForm, PickerKind, Popup, TextKind, VerifyPopup};
use crate::app::{App, Focus};
use crate::config::UiConfig;

fn room(id: &str, name: &str, unread: u64, is_direct: bool) -> Room {
    Room {
        id: id.into(), name: name.into(), topic: None, alias: None, unread, highlights: 0,
        is_direct, dm_with: if is_direct { vec![format!("@{}:x", name.to_lowercase())] } else { vec![] },
        members: if is_direct { 2 } else { 12 }, encrypted: is_direct, invite: None,
    }
}

fn demo_app() -> App {
    let mut app = App::new(UiConfig::default(), "@me:example.org".into());
    app.sync_state = SyncState::Online;
    let mut thesis = room("!a:x", "Thesis — telaio", 0, false);
    thesis.topic = Some("Talking about the client".into());
    app.rooms = vec![thesis, room("!b:x", "Rust Users", 3, false), room("!c:x", "Supervisor", 1, true)];
    app.spaces = vec![Space { id: "!s:x".into(), name: "University".into(), children: ["!a:x".to_string()].into() }];
    app.hierarchy.insert("!s:x".into(), vec![HierarchyEntry {
        id: "!z:x".into(), name: "Announcements".into(), alias: Some("#announcements:x".into()), topic: Some("Course news".into()), members: 87, is_space: false,
    }]);
    let now = Utc::now();
    let mk = |id: &str, who: &str, mins: i64, content: Content| Message {
        event_id: id.into(), sender: format!("@{who}:x"), sender_name: who.into(),
        timestamp: now - chrono::Duration::minutes(mins), content,
    };
    let mut tl = crate::app::timeline::Timeline::default();
    tl.add_history(vec![
        mk("1", "alice", 3000, Content::Text("Yesterday's message".into())),
        mk("2", "bob", 60, Content::Text("I uploaded the new draft, let me know what you think when you have time.".into())),
        mk("3", "bob", 59, Content::Text("Especially chapter 3".into())),
        mk("4", "me", 30, Content::Attachment(Attachment {
            kind: AttachmentKind::File, name: "chapter3.pdf".into(), size: Some(1_234_567),
            mimetype: Some("application/pdf".into()), source: MediaSource::Plain(OwnedMxcUri::from("mxc://x/y")),
        })),
        mk("5", "alice", 3, Content::Encrypted(Undecrypted { session_id: Some("s1".into()), raw: "{\"encrypted\":1}".into() })),
        mk("6", "alice", 2, Content::Emote("approves".into())),
    ], None);
    app.timelines.insert("!a:x".into(), tl);
    app.device_id = "TELAIO01".into();
    app.accounts = vec![AccountEntry { username: "@work:example.org".into(), has_session: true }];
    app.refresh_entries();
    app.open_room_anywhere("!a:x");
    app.pending.clear();
    app
}

fn draw(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app)).unwrap();
    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn renders_at_any_size() {
    let mut app = demo_app();
    app.push_error("Cannot send photo.png (413 file too large)");
    for (w, h) in [(140, 40), (80, 24), (40, 12), (10, 4), (1, 1), (0, 0)] {
        for (spaces, rooms) in [(true, true), (false, true), (true, false), (false, false)] {
            app.show_spaces = spaces;
            app.show_rooms = rooms;
            for focus in [Focus::Spaces, Focus::Rooms, Focus::Chat, Focus::Input] {
                app.focus = focus;
                draw(&mut app, w, h);
            }
        }
        let emojis: Vec<(String, String)> = ["🐶", "☁️", "🔑", "🚀", "⚓", "🎸", "📌"].iter()
            .map(|e| (e.to_string(), "Spanner".to_string())).collect();
        let verify = |stage| Popup::Verify(VerifyPopup { user_id: "@me:example.org".into(), flow_id: "f".into(), stage });
        for popup in [
            Popup::Help,
            Popup::ManageSpaces { room_id: "!a:x".into(), cursor: 0 },
            Popup::Account { cursor: 3 },
            Popup::Login(LoginForm::new("@me:example.org", "", Some("Session expired".into()))),
            verify(VerifyStage::Incoming { device: "ABCDEF".into() }),
            verify(VerifyStage::Waiting("Waiting".into())),
            verify(VerifyStage::Emojis(emojis.clone())),
            verify(VerifyStage::Done),
            verify(VerifyStage::Cancelled("timed out".into())),
        ] {
            app.popup = Some(popup);
            draw(&mut app, w, h);
        }
        for kind in [PickerKind::Join, PickerKind::Dm, PickerKind::Switch] {
            app.open_picker(kind);
            draw(&mut app, w, h);
        }
        app.open_text(TextKind::Attach);
        draw(&mut app, w, h);
        app.open_text(TextKind::RecoveryKey);
        draw(&mut app, w, h);
        app.popup = None;
        app.select_space(1);
        draw(&mut app, w, h);
        app.select_space(0);
    }
}

#[test]
fn explore_and_space_switch() {
    let mut app = demo_app();
    app.select_space(1);
    // inside the space: the child room and the entry to explore
    assert!(app.entries.iter().any(|e| matches!(e, crate::app::Entry::Explore(_))));
    app.move_room_cursor(10);
    assert_eq!(app.selected_explore().map(|h| h.name.as_str()), Some("Announcements"));
    assert!(app.selected_room.is_none());
    app.select_space(0);
    // back in Home, the last room seen there is reopened
    assert_eq!(app.selected_room.as_deref(), Some("!a:x"));
}

#[test]
fn switcher_and_dm_picker() {
    let mut app = demo_app();
    app.open_picker(PickerKind::Switch);
    if let Some(Popup::Picker(p)) = app.popup.as_mut() { p.input.set("rust"); }
    app.picker_changed();
    let Some(Popup::Picker(p)) = &app.popup else { panic!() };
    assert_eq!(p.items()[0].title, "# Rust Users");
    let item = p.selected().unwrap();
    app.pick(item);
    assert_eq!(app.current_room().unwrap().name, "Rust Users");

    // an existing DM is reopened instead of creating a new one
    app.open_picker(PickerKind::Dm);
    if let Some(Popup::Picker(p)) = app.popup.as_mut() { p.input.set("@supervisor:x"); }
    app.picker_changed();
    let Some(Popup::Picker(p)) = &app.popup else { panic!() };
    let item = p.selected().unwrap();
    app.pick(item);
    assert_eq!(app.current_room().unwrap().name, "Supervisor");
    assert!(app.pending.iter().all(|c| !matches!(c, crate::app::event::Command::OpenDm { .. })));
}

#[test]
#[ignore = "prints the UI: cargo test preview -- --ignored --nocapture"]
fn preview() {
    let mut app = demo_app();
    app.focus = Focus::Chat;
    app.selected_msg = Some(3);
    app.set_status("Attachment sent");
    app.push_error("Join failed: Cannot join #missing:matrix.org (M_NOT_FOUND)");
    println!("{}", draw(&mut app, 120, 30));

    app.errors.clear();
    app.select_space(1);
    app.focus = Focus::Rooms;
    app.move_room_cursor(10);
    println!("{}", draw(&mut app, 120, 22));

    app.show_spaces = false;
    app.select_space(0);
    app.focus = Focus::Input;
    app.input.insert_str("draft almost ready");
    println!("{}", draw(&mut app, 90, 20));

    app.show_rooms = false;
    app.open_picker(PickerKind::Switch);
    println!("{}", draw(&mut app, 90, 22));

    app.popup = None;
    app.show_rooms = true;
    let mut invite = room("!inv:x", "Study group", 0, false);
    invite.invite = Some(crate::app::model::InviteInfo { inviter: "Carla".into(), is_space: false });
    app.rooms.push(invite);
    app.open_room_anywhere("!inv:x");
    app.focus = Focus::Rooms;
    println!("{}", draw(&mut app, 90, 16));
    app.security = Security { verified: Some(false), recovery: RecoveryStatus::Incomplete };
    app.popup = Some(Popup::Account { cursor: 0 });
    println!("{}", draw(&mut app, 100, 24));
    let emojis = [("🐶", "Dog"), ("🔑", "Key"), ("🚀", "Rocket"), ("⚓", "Anchor"), ("🎸", "Guitar"), ("🔧", "Spanner"), ("📌", "Pin")];
    app.popup = Some(Popup::Verify(VerifyPopup {
        user_id: app.user_id.clone(), flow_id: "f".into(),
        stage: VerifyStage::Emojis(emojis.iter().map(|(e, n)| (e.to_string(), n.to_string())).collect()),
    }));
    println!("{}", draw(&mut app, 100, 16));
    app.popup = Some(Popup::Login(LoginForm::new("@me:example.org", "", Some("Session expired: please sign in again.".into()))));
    println!("{}", draw(&mut app, 100, 16));
}

#[test]
fn mouse_wheel_scrolls_chat_and_click_selects() {
    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    let mut app = demo_app();
    let selected = app.selected_room.clone();
    draw(&mut app, 120, 12); // short window: not all messages fit

    let at = |kind, column, row| Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
    let chat = app.hit.chat;
    crate::keys::handle_event(at(MouseEventKind::ScrollUp, chat.x + 5, chat.y + 2), &mut app);
    assert_eq!(app.chat_view.scroll, 3, "the wheel must scroll the chat");
    assert_eq!(app.selected_room, selected, "the wheel must not change room");

    // the wheel over the room list changes nothing
    let rooms = app.hit.rooms;
    crate::keys::handle_event(at(MouseEventKind::ScrollDown, rooms.x + 2, rooms.y + 1), &mut app);
    assert_eq!(app.selected_room, selected);

    // clicking a list row opens that room
    let line = app.hit.room_lines.iter().position(|e| {
        matches!(e.and_then(|i| app.entries.get(i).copied()), Some(crate::app::Entry::Room(r)) if app.rooms[r].name == "Supervisor")
    }).unwrap();
    let row = rooms.y + (line - app.hit.rooms_scroll) as u16;
    crate::keys::handle_event(at(MouseEventKind::Down(MouseButton::Left), rooms.x + 3, row), &mut app);
    assert_eq!(app.current_room().unwrap().name, "Supervisor");
    assert_eq!(app.focus, Focus::Rooms);

    // clicking the space bar selects the space
    draw(&mut app, 120, 12);
    let spaces = app.hit.spaces;
    crate::keys::handle_event(at(MouseEventKind::Down(MouseButton::Left), spaces.x + 2, spaces.y + app.hit.spaces_first + 1), &mut app);
    assert_eq!(app.space_idx, 1);
}

#[test]
fn redaction_removes_message() {
    let mut app = demo_app();
    app.selected_msg = Some(5);
    app.apply(crate::app::event::AppEvent::Redacted { room_id: "!a:x".into(), event_id: "2".into() });
    let tl = app.current_timeline().unwrap();
    assert!(tl.messages.iter().all(|m| m.event_id != "2"));
    assert_eq!(app.selected_msg, Some(4), "the selection stays on the same message");
}

#[test]
fn invites_are_listed_and_accepted() {
    use crate::app::model::InviteInfo;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut app = demo_app();
    let mut invite = room("!inv:x", "Study group", 0, false);
    invite.invite = Some(InviteInfo { inviter: "Carla".into(), is_space: false });
    app.rooms.push(invite);
    app.refresh_entries();
    assert_eq!(app.entries.first(), Some(&crate::app::Entry::Section(crate::app::Section::Invites)));
    assert_eq!(app.space_unread(0).0, 5, "3 + 1 unread + 1 invite");

    app.open_room_anywhere("!inv:x");
    app.pending.clear();
    app.focus = Focus::Rooms;
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    crate::keys::handle_event(enter, &mut app);
    assert!(app.pending.iter().any(|c| matches!(c, crate::app::event::Command::Join { address } if address == "!inv:x")));
    assert!(draw(&mut app, 100, 20).contains("Carla invited you"));
}

#[test]
fn kick_command_asks_confirmation() {
    use crate::app::popup::ConfirmAction;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut app = demo_app();
    app.focus = Focus::Input;
    app.input.insert_str("/kick @bob:x repeated spam");
    let key = |c| Event::Key(KeyEvent::new(c, KeyModifiers::NONE));
    crate::keys::handle_event(key(KeyCode::Enter), &mut app);
    let Some(Popup::Confirm { action: ConfirmAction::Moderate { target, reason, .. }, .. }) = &app.popup else {
        panic!("a confirmation is required");
    };
    assert_eq!(target, "@bob:x");
    assert_eq!(reason.as_deref(), Some("repeated spam"));
    crate::keys::handle_event(key(KeyCode::Enter), &mut app);
    assert!(app.pending.iter().any(|c| matches!(c, crate::app::event::Command::Moderate { .. })));
}

#[test]
fn vim_like_modes() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let key = |c| Event::Key(KeyEvent::new(c, KeyModifiers::NONE));
    let mut app = demo_app();
    app.focus_panel(Focus::Rooms);
    assert!(!app.is_insert());
    assert!(draw(&mut app, 100, 20).contains("NORMAL"));

    // in normal mode letters are commands: "j" changes room, doesn't type
    crate::keys::handle_event(key(KeyCode::Char('j')), &mut app);
    assert!(app.input.is_empty());

    // Tab never enters the text field
    for _ in 0..5 {
        crate::keys::handle_event(key(KeyCode::Tab), &mut app);
        assert!(!app.is_insert());
    }

    // "i" enters insert mode: letters now go into the text
    app.focus_panel(Focus::Rooms);
    crate::keys::handle_event(key(KeyCode::Char('i')), &mut app);
    assert!(app.is_insert());
    for c in "jak".chars() { crate::keys::handle_event(key(KeyCode::Char(c)), &mut app); }
    assert_eq!(app.input.as_str(), "jak");
    assert!(draw(&mut app, 100, 20).contains("INSERT"));

    // Esc goes back to the previous panel, the text stays
    crate::keys::handle_event(key(KeyCode::Esc), &mut app);
    assert_eq!(app.focus, Focus::Rooms);
    assert_eq!(app.input.as_str(), "jak");

    // ":" opens the "command line"
    app.input.clear();
    crate::keys::handle_event(key(KeyCode::Char(':')), &mut app);
    assert!(app.is_insert());
    assert_eq!(app.input.as_str(), "/");

    // with both bars hidden Esc still reaches normal mode
    app.show_spaces = false;
    app.show_rooms = false;
    crate::keys::handle_event(key(KeyCode::Esc), &mut app);
    assert_eq!(app.focus, Focus::Chat);
}

fn press(app: &mut App, code: crossterm::event::KeyCode) {
    use crossterm::event::{Event, KeyEvent, KeyModifiers};
    crate::keys::handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), app);
}

#[test]
fn arrows_select_messages_in_insert_mode() {
    use crossterm::event::KeyCode;
    let mut app = demo_app();
    app.enter_insert();
    let room = app.selected_room.clone();
    let count = app.current_timeline().unwrap().messages.len();
    press(&mut app, KeyCode::Up);
    assert_eq!(app.selected_msg, Some(count - 1), "↑ selects the last message");
    assert_eq!(app.focus, Focus::Input);
    assert_eq!(app.selected_room, room, "arrows don't change room");

    // up to chapter3.pdf: the selection is visible in insert mode too
    while app.selected_msg != Some(3) { press(&mut app, KeyCode::Up); }
    assert!(draw(&mut app, 120, 30).contains("Enter open · Esc s save"));
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.pending.last(), Some(Command::Open { attachment }) if attachment.name == "chapter3.pdf"));

    // with text in the field, Enter sends and returns to the latest messages
    app.input.insert_str("hi");
    press(&mut app, KeyCode::Enter);
    assert!(app.selected_msg.is_none() && app.input.is_empty());

    // Esc with a selection moves to Messages keeping it (s saves)
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::Chat);
    assert_eq!(app.selected_msg, Some(count - 1));

    // ↓ past the last message clears the selection
    app.enter_insert();
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Down);
    assert!(app.selected_msg.is_none());
}

#[test]
fn header_shows_encryption_state() {
    let mut app = demo_app();
    // "Thesis — telaio" (!a:x) is not encrypted
    let screen = draw(&mut app, 120, 20);
    assert!(screen.contains("Thesis — telaio  ✗ not encrypted"), "{screen}");

    app.rooms.iter_mut().find(|r| r.id == "!a:x").unwrap().encrypted = true;
    let screen = draw(&mut app, 120, 20);
    assert!(screen.contains("Thesis — telaio  ✓ encrypted"), "{screen}");
    assert!(!screen.contains("not encrypted"));
}

#[test]
fn unverified_session_is_flagged() {
    let mut app = demo_app();
    app.apply(AppEvent::Security(Security { verified: Some(false), recovery: RecoveryStatus::Incomplete }));
    let screen = draw(&mut app, 140, 30);
    assert!(screen.contains("unverified (F2)"));
    assert!(screen.contains("press F2 to verify"), "the encrypted message hints how to unlock it:\n{screen}");

    app.apply(AppEvent::Security(Security { verified: Some(true), recovery: RecoveryStatus::Enabled }));
    let screen = draw(&mut app, 140, 30);
    assert!(!screen.contains("unverified"));
    assert!(!screen.contains("press F2 to verify"));
}

#[test]
fn account_popup_actions() {
    use crossterm::event::KeyCode;
    let mut app = demo_app();
    press(&mut app, KeyCode::F(2));
    let screen = draw(&mut app, 120, 30);
    assert!(screen.contains("device TELAIO01"));
    assert!(screen.contains("Switch to @work:example.org"));

    // ↓↓ reaches "Switch to …"
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert!(app.popup.is_none());
    assert!(app.pending.iter().any(|c| matches!(c, Command::SwitchAccount { username } if username == "@work:example.org")));

    // logging out asks for confirmation
    app.pending.clear();
    app.popup = Some(Popup::Account { cursor: 0 });
    for _ in 0..10 { press(&mut app, KeyCode::Down); }
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.popup, Some(Popup::Confirm { .. })));
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.iter().any(|c| matches!(c, Command::Logout)));
}

#[test]
fn recovery_key_is_masked_and_sent() {
    use crossterm::event::{Event, KeyCode};
    let mut app = demo_app();
    app.focus = Focus::Input;
    app.input.insert_str("/recovery");
    press(&mut app, KeyCode::Enter);
    crate::keys::handle_event(Event::Paste("EsTc abcd EFGH".into()), &mut app);
    let screen = draw(&mut app, 100, 24);
    assert!(!screen.contains("abcd"), "the key must not appear on screen");
    assert!(screen.contains("••••"));
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.iter().any(|c| matches!(c, Command::Recover { key } if key == "EsTc abcd EFGH")));
}

#[test]
fn login_form_flow() {
    use crossterm::event::KeyCode;
    let mut app = demo_app();
    app.popup = Some(Popup::Login(LoginForm::default()));
    for c in "@new:b.org".chars() { press(&mut app, KeyCode::Char(c)); }
    press(&mut app, KeyCode::Enter); // → server (empty: from the MXID)
    press(&mut app, KeyCode::Enter); // → password
    press(&mut app, KeyCode::Enter); // empty password: error, no command
    assert!(app.pending.is_empty());
    for c in "secret".chars() { press(&mut app, KeyCode::Char(c)); }
    assert!(!draw(&mut app, 100, 24).contains("secret"));
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.iter().any(|c| matches!(c,
        Command::Login { homeserver, username, password } if homeserver.is_empty() && username == "@new:b.org" && password == "secret")));
    let Some(Popup::Login(form)) = &app.popup else { panic!() };
    assert!(form.busy);

    // on error the popup stays open with the message
    app.apply(AppEvent::LoginFailed("Login failed: wrong username or password".into()));
    assert!(draw(&mut app, 100, 24).contains("wrong username"));

    // an account without a valid session opens the login prefilled, on the password
    app.apply(AppEvent::NeedLogin { homeserver: "https://b.org".into(), username: "@x:b.org".into(), note: None });
    let Some(Popup::Login(form)) = &app.popup else { panic!() };
    assert_eq!((form.user.as_str(), form.server.as_str(), form.field), ("@x:b.org", "https://b.org", 2));
}

#[test]
fn emoji_verification_flow() {
    use crossterm::event::KeyCode;
    let mut app = demo_app();
    let ev = |flow: &str, stage| AppEvent::Verification { user_id: "@me:example.org".into(), flow_id: flow.into(), stage };

    // request sent from here: the popup gets its ID from the first event
    app.start_verification();
    assert!(app.pending.iter().any(|c| matches!(c, Command::StartVerification)));
    app.apply(ev("f1", VerifyStage::Waiting("Accept the request".into())));
    let emojis = vec![("🐶".to_string(), "Dog".to_string()); 7];
    app.apply(ev("f1", VerifyStage::Emojis(emojis)));
    let screen = draw(&mut app, 120, 30);
    assert!(screen.contains("Dog") && screen.contains("same emojis"));

    app.pending.clear();
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.iter().any(|c| matches!(c, Command::VerifyConfirm { flow_id, .. } if flow_id == "f1")));
    app.apply(ev("f1", VerifyStage::Done));
    assert!(draw(&mut app, 120, 30).contains("Session verified"));
    // once verified, encrypted messages are retried
    assert!(app.pending.iter().any(|c| matches!(c, Command::Redecrypt { room_id, .. } if room_id == "!a:x")));
    press(&mut app, KeyCode::Char('x'));
    assert!(app.popup.is_none());

    // an incoming request takes over another open popup
    app.popup = Some(Popup::Help);
    app.apply(ev("f2", VerifyStage::Incoming { device: "ELEMENT".into() }));
    assert!(draw(&mut app, 120, 30).contains("ELEMENT"));
    app.pending.clear();
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.iter().any(|c| matches!(c, Command::VerifyAccept { flow_id, .. } if flow_id == "f2")));
    // Esc while waiting cancels on the other side too
    press(&mut app, KeyCode::Esc);
    assert!(app.pending.iter().any(|c| matches!(c, Command::VerifyCancel { flow_id, .. } if flow_id == "f2")));
    // updates about a closed verification don't reopen the popup
    app.apply(ev("f2", VerifyStage::Cancelled("cancelled".into())));
    assert!(app.popup.is_none());
}

#[test]
fn new_room_keys_redecrypt_messages() {
    let mut app = demo_app();
    // key for another session: nothing to do
    app.apply(AppEvent::RoomKeys(Some(vec![("!a:x".into(), "other".into())])));
    assert!(app.pending.is_empty());
    app.apply(AppEvent::RoomKeys(Some(vec![("!a:x".into(), "s1".into())])));
    assert!(app.pending.iter().any(|c| matches!(c, Command::Redecrypt { events, .. } if events.len() == 1)));

    // the decrypted message replaces the placeholder
    let mut msg = app.current_timeline().unwrap().messages[4].clone();
    msg.content = Content::Text("now readable".into());
    app.apply(AppEvent::Decrypted { room_id: "!a:x".into(), messages: vec![("5".into(), Some(msg))] });
    assert!(draw(&mut app, 140, 30).contains("now readable"));
    assert!(app.current_timeline().unwrap().undecrypted(None).is_empty());
}

#[test]
fn login_screen_renders() {
    let mut app = App::new(UiConfig::default(), String::new());
    app.popup = Some(Popup::Login(LoginForm::default()));
    for (w, h) in [(120, 30), (60, 12), (20, 5), (1, 1), (0, 0)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| super::render_login(f, &app)).unwrap();
    }
    let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
    term.draw(|f| super::render_login(f, &app)).unwrap();
    let screen: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(screen.contains("Sign in") && screen.contains("Ctrl+C quit"));
    // Esc closes the popup: the login loop treats it as giving up
    press(&mut app, crossterm::event::KeyCode::Esc);
    assert!(app.popup.is_none());
}

#[test]
fn attachment_open_or_save() {
    use crossterm::event::KeyCode;
    let mut app = demo_app();
    app.focus_panel(Focus::Chat);
    app.selected_msg = Some(3); // chapter3.pdf
    assert!(draw(&mut app, 120, 30).contains("Enter open · s save"));
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.pending.last(), Some(Command::Open { attachment }) if attachment.name == "chapter3.pdf"));
    press(&mut app, KeyCode::Char('s'));
    assert!(matches!(app.pending.last(), Some(Command::Download { .. })));
    // message without attachment: no command
    app.pending.clear();
    app.selected_msg = Some(1);
    press(&mut app, KeyCode::Enter);
    assert!(app.pending.is_empty());
}
