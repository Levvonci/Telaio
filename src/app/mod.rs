//! Application state and how it reacts to events. No terminal dependencies.

pub mod event;
pub mod input;
pub mod model;
pub mod popup;
pub mod timeline;

use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::config::UiConfig;
use crate::matrix::address;
use crate::theme::Theme;
use event::{AppEvent, Command, Joined, Security, SyncState, VerifyStage};
use input::LineInput;
use model::{AccountEntry, HierarchyEntry, Message, Room, Space};
use popup::{
    fuzzy_score, AccountItem, LoginForm, PickAction, PickItem, Picker, PickerKind, Popup, TextKind, TextPopup, VerifyPopup,
    SEARCH_DEBOUNCE,
};
use timeline::{HistoryState, Timeline};

/// Panel receiving keys. `Input` is INSERT mode (keys go into the text);
/// the other panels are NORMAL mode (keys are commands), as in vim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus { Spaces, Rooms, Chat, Input }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Section { Invites, People, Rooms, Explore }

/// Row of the room list
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Entry {
    Section(Section),
    /// Index into `App::rooms`
    Room(usize),
    /// Index into the current space hierarchy (room not joined yet)
    Explore(usize),
}

pub struct ErrorEntry {
    pub msg: String,
    pub at: Instant,
    /// Consecutive repetitions of the same error
    pub count: u32,
}

/// Message view state, updated while rendering
#[derive(Default)]
pub struct ChatView {
    /// Lines scrolled up from the bottom
    pub scroll: usize,
    /// The render reached the oldest loaded message
    pub at_top: bool,
    pub height: usize,
}

/// Where clickable elements were drawn in the last frame
#[derive(Default)]
pub struct HitMap {
    pub spaces: Rect,
    /// Row (relative to `spaces`) of the Home entry
    pub spaces_first: u16,
    pub spaces_scroll: usize,
    pub rooms: Rect,
    /// For each room list row: index into `App::entries`
    pub room_lines: Vec<Option<usize>>,
    pub rooms_scroll: usize,
    pub chat: Rect,
    pub input: Rect,
}

const MAX_ERRORS: usize = 5;

pub struct App {
    pub ui: UiConfig,
    pub theme: Theme,
    pub user_id: String,
    pub device_id: String,
    pub security: Security,
    pub accounts: Vec<AccountEntry>,
    pub rooms: Vec<Room>,
    pub spaces: Vec<Space>,
    pub timelines: HashMap<String, Timeline>,
    pub hierarchy: HashMap<String, Vec<HierarchyEntry>>,

    /// 0 = Home, 1.. = `spaces[i - 1]`
    pub space_idx: usize,
    /// Room list of the current space, rebuilt only when needed
    pub entries: Vec<Entry>,
    pub room_cursor: usize,
    pub selected_room: Option<String>,
    pub show_spaces: bool,
    pub show_rooms: bool,
    pub focus: Focus,
    /// Panel to return to when leaving insert mode
    last_panel: Focus,
    /// Selected message (index into the timeline)
    pub selected_msg: Option<usize>,
    pub chat_view: ChatView,
    pub hit: HitMap,

    pub input: LineInput,
    pub popup: Option<Popup>,

    pub sync_state: SyncState,
    pub status: Option<(String, Instant)>,
    pub errors: VecDeque<ErrorEntry>,
    pub uploads: usize,
    /// Room or space to select as soon as it shows up in the lists
    pending_open: Option<String>,
    /// Last room opened in each space ("" = Home)
    last_room: HashMap<String, String>,

    /// Commands to run, drained by the main loop
    pub pending: Vec<Command>,
}

impl App {
    pub fn new(ui: UiConfig, user_id: String) -> Self {
        let theme = Theme::from_config(&ui);
        let (show_spaces, show_rooms) = (ui.layout.show_spaces, ui.layout.show_rooms);
        Self {
            ui,
            theme,
            user_id,
            device_id: String::new(),
            security: Security::default(),
            accounts: Vec::new(),
            rooms: Vec::new(),
            spaces: Vec::new(),
            timelines: HashMap::new(),
            hierarchy: HashMap::new(),
            space_idx: 0,
            entries: Vec::new(),
            room_cursor: 0,
            selected_room: None,
            show_spaces,
            show_rooms,
            focus: if show_rooms { Focus::Rooms } else { Focus::Chat },
            last_panel: if show_rooms { Focus::Rooms } else { Focus::Chat },
            selected_msg: None,
            chat_view: ChatView::default(),
            hit: HitMap::default(),
            input: LineInput::default(),
            popup: None,
            sync_state: SyncState::Connecting,
            status: None,
            errors: VecDeque::new(),
            uploads: 0,
            pending_open: None,
            last_room: HashMap::new(),
            pending: Vec::new(),
        }
    }

    // ── Panels ─────────────────────────────────────────────────────────────

    /// Panels visible in normal mode, in Tab order
    fn panels(&self) -> Vec<Focus> {
        let mut v = Vec::with_capacity(3);
        if self.show_spaces { v.push(Focus::Spaces); }
        if self.show_rooms { v.push(Focus::Rooms); }
        v.push(Focus::Chat);
        v
    }

    pub fn is_insert(&self) -> bool {
        self.focus == Focus::Input
    }

    pub fn enter_insert(&mut self) {
        if !self.is_insert() { self.last_panel = self.focus; }
        self.focus = Focus::Input;
        self.selected_msg = None;
    }

    /// NORMAL mode: back to the last used panel, if still visible
    pub fn enter_normal(&mut self) {
        self.focus = if self.panels().contains(&self.last_panel) { self.last_panel } else { self.fallback_panel() };
    }

    fn fallback_panel(&self) -> Focus {
        if self.show_rooms { Focus::Rooms } else { Focus::Chat }
    }

    pub fn focus_panel(&mut self, panel: Focus) {
        if panel == Focus::Input { return self.enter_insert(); }
        self.focus = panel;
        self.last_panel = panel;
        if panel != Focus::Chat { self.selected_msg = None; }
    }

    pub fn cycle_focus(&mut self, forward: bool) {
        let panels = self.panels();
        let n = panels.len();
        let next = match panels.iter().position(|&p| p == self.focus) {
            Some(pos) => panels[if forward { (pos + 1) % n } else { (pos + n - 1) % n }],
            None => self.fallback_panel(),
        };
        self.focus_panel(next);
    }

    pub fn toggle_spaces(&mut self) {
        self.show_spaces = !self.show_spaces;
        if self.show_spaces { self.focus_panel(Focus::Spaces); }
        else if self.focus == Focus::Spaces { self.focus_panel(self.fallback_panel()); }
    }

    pub fn toggle_rooms(&mut self) {
        self.show_rooms = !self.show_rooms;
        if self.show_rooms { self.focus_panel(Focus::Rooms); }
        else if self.focus == Focus::Rooms { self.focus_panel(Focus::Chat); }
    }

    // ── Spaces ─────────────────────────────────────────────────────────────

    pub fn current_space(&self) -> Option<&Space> {
        self.space_idx.checked_sub(1).and_then(|i| self.spaces.get(i))
    }

    pub fn space_label(&self) -> String {
        self.current_space().map(|s| s.name.clone()).unwrap_or_else(|| "Home".into())
    }

    fn in_any_space(&self, room_id: &str) -> bool {
        self.spaces.iter().any(|s| s.children.contains(room_id))
    }

    fn room_in_view(&self, room: &Room) -> bool {
        if room.invite.is_some() {
            return self.space_idx == 0; // invites live in Home
        }
        match self.current_space() {
            Some(space) => space.children.contains(&room.id),
            None => self.ui.layout.home_all_rooms || room.is_direct || !self.in_any_space(&room.id),
        }
    }

    /// (unread, mentions) of a space bar entry
    pub fn space_unread(&self, idx: usize) -> (u64, u64) {
        let space = idx.checked_sub(1).and_then(|i| self.spaces.get(i));
        self.rooms.iter()
            .filter(|r| match space {
                Some(s) => s.children.contains(&r.id),
                None => self.ui.layout.home_all_rooms || r.is_direct || !self.in_any_space(&r.id),
            })
            // an invite counts as unread
            .fold((0, 0), |(u, h), r| (u + r.unread + u64::from(r.invite.is_some()), h + r.highlights))
    }

    pub fn select_space(&mut self, idx: usize) {
        let idx = idx.min(self.spaces.len());
        let changed = idx != self.space_idx;
        self.space_idx = idx;
        if let Some(space) = self.current_space() {
            let id = space.id.clone();
            if changed || !self.hierarchy.contains_key(&id) {
                self.pending.push(Command::LoadHierarchy { space_id: id });
            }
        }
        self.refresh_entries();
        // reopen the last room seen in this space, otherwise the first one
        let key = self.space_key();
        if let Some(id) = self.last_room.get(&key).cloned() {
            if self.rooms.iter().any(|r| r.id == id && self.room_in_view(r)) && !self.is_open(&id) {
                self.open_room(&id);
            }
        }
        if self.room_entry_pos().is_none() {
            self.room_cursor = 0;
            self.move_room_cursor(0);
        }
    }

    fn space_key(&self) -> String {
        self.current_space().map(|s| s.id.clone()).unwrap_or_default()
    }

    pub fn move_space(&mut self, delta: isize) {
        let max = self.spaces.len() as isize;
        let new = (self.space_idx as isize + delta).clamp(0, max) as usize;
        if new != self.space_idx { self.select_space(new); }
    }

    // ── Room list ──────────────────────────────────────────────────────────

    fn last_activity(&self, room_id: &str) -> Option<DateTime<Utc>> {
        self.timelines.get(room_id).and_then(|t| t.last()).map(|m| m.timestamp)
    }

    /// Rebuilds the current space list: Invites, People, Rooms, Explore
    pub fn refresh_entries(&mut self) {
        let mut invites = Vec::new();
        let mut people = Vec::new();
        let mut rooms = Vec::new();
        for (i, room) in self.rooms.iter().enumerate() {
            if !self.room_in_view(room) { continue; }
            if room.invite.is_some() { invites.push(i) }
            else if room.is_direct { people.push(i) }
            else { rooms.push(i) }
        }
        let by_activity = |v: &mut Vec<usize>| v.sort_by(|&a, &b| {
            let (ra, rb) = (&self.rooms[a], &self.rooms[b]);
            self.last_activity(&rb.id).cmp(&self.last_activity(&ra.id))
                .then_with(|| ra.name.to_lowercase().cmp(&rb.name.to_lowercase()))
        });
        by_activity(&mut people);
        by_activity(&mut rooms);

        let mut entries = Vec::new();
        if !invites.is_empty() {
            entries.push(Entry::Section(Section::Invites));
            entries.extend(invites.into_iter().map(Entry::Room));
        }
        if !people.is_empty() {
            entries.push(Entry::Section(Section::People));
            entries.extend(people.into_iter().map(Entry::Room));
        }
        if !rooms.is_empty() {
            entries.push(Entry::Section(Section::Rooms));
            entries.extend(rooms.into_iter().map(Entry::Room));
        }
        let explore: Vec<usize> = self.explore_list().map(|(i, _)| i).collect();
        if !explore.is_empty() {
            entries.push(Entry::Section(Section::Explore));
            entries.extend(explore.into_iter().map(Entry::Explore));
        }

        let previous = self.entries.get(self.room_cursor).copied();
        self.entries = entries;
        // keep the cursor on the open room (indices may have changed)
        if let Some(pos) = self.room_entry_pos() {
            self.room_cursor = pos;
        } else if let Some(Entry::Explore(i)) = previous {
            self.room_cursor = self.entries.iter().position(|e| *e == Entry::Explore(i))
                .unwrap_or(self.room_cursor);
        }
        self.room_cursor = self.room_cursor.min(self.entries.len().saturating_sub(1));
    }

    /// Rooms of the current space hierarchy not joined yet
    fn explore_list(&self) -> impl Iterator<Item = (usize, &HierarchyEntry)> {
        let space = self.current_space();
        let list = space.and_then(|s| self.hierarchy.get(&s.id));
        list.into_iter().flatten().enumerate().filter(move |(_, h)| {
            Some(h.id.as_str()) != space.map(|s| s.id.as_str())
                && !self.rooms.iter().any(|r| r.id == h.id)
                && !self.spaces.iter().any(|s| s.id == h.id)
        })
    }

    pub fn explore_entry(&self, idx: usize) -> Option<&HierarchyEntry> {
        self.hierarchy.get(&self.current_space()?.id)?.get(idx)
    }

    /// "Explore" entry under the cursor, if any
    pub fn selected_explore(&self) -> Option<&HierarchyEntry> {
        match self.entries.get(self.room_cursor) {
            Some(Entry::Explore(i)) => self.explore_entry(*i),
            _ => None,
        }
    }

    fn room_entry_pos(&self) -> Option<usize> {
        let id = self.selected_room.as_deref()?;
        self.entries.iter().position(|e| matches!(e, Entry::Room(i) if self.rooms[*i].id == id))
    }

    /// Moves the cursor skipping headers; rooms open immediately
    pub fn move_room_cursor(&mut self, delta: isize) {
        let selectable: Vec<usize> = self.entries.iter().enumerate()
            .filter(|(_, e)| !matches!(e, Entry::Section(_)))
            .map(|(i, _)| i)
            .collect();
        if selectable.is_empty() {
            self.selected_room = None;
            return;
        }
        let cur = selectable.iter().position(|&i| i >= self.room_cursor).unwrap_or(0) as isize;
        let next = selectable[(cur + delta).clamp(0, selectable.len() as isize - 1) as usize];
        self.room_cursor = next;
        match self.entries[next] {
            Entry::Room(i) => {
                let id = self.rooms[i].id.clone();
                if self.selected_room.as_deref() != Some(&id) { self.open_room(&id); }
            }
            Entry::Explore(_) => { self.selected_room = None; self.selected_msg = None; }
            Entry::Section(_) => {}
        }
    }

    pub fn current_room(&self) -> Option<&Room> {
        let id = self.selected_room.as_deref()?;
        self.rooms.iter().find(|r| r.id == id)
    }

    pub fn current_timeline(&self) -> Option<&Timeline> {
        self.timelines.get(self.selected_room.as_deref()?)
    }

    /// Selects a room: resets scroll, marks it read, loads history
    pub fn open_room(&mut self, room_id: &str) {
        self.selected_room = Some(room_id.to_string());
        self.chat_view.scroll = 0;
        self.selected_msg = None;
        // an invite's messages cannot be read before joining
        if !self.current_room().is_some_and(|r| r.invite.is_some()) {
            self.mark_read(room_id);
            self.ensure_history(room_id);
        }
        if let Some(pos) = self.room_entry_pos() { self.room_cursor = pos; }
        self.last_room.insert(self.space_key(), room_id.to_string());
    }

    /// Opens a room wherever it is: if not in the current space, switches to Home
    pub fn open_room_anywhere(&mut self, room_id: &str) {
        let Some(room) = self.rooms.iter().find(|r| r.id == room_id) else { return };
        if !self.room_in_view(room) {
            self.space_idx = 0;
            let room = room.clone();
            if !self.room_in_view(&room) {
                // Home without "all rooms": go to the space containing it
                if let Some(i) = self.spaces.iter().position(|s| s.children.contains(room_id)) {
                    self.space_idx = i + 1;
                }
            }
        }
        self.refresh_entries();
        self.open_room(room_id);
    }

    fn mark_read(&mut self, room_id: &str) {
        let Some(room) = self.rooms.iter_mut().find(|r| r.id == room_id) else { return };
        if room.unread == 0 { return; }
        room.unread = 0;
        room.highlights = 0;
        if let Some(last) = self.timelines.get(room_id).and_then(|t| t.last()) {
            self.pending.push(Command::MarkRead { room_id: room_id.into(), event_id: last.event_id.clone() });
        }
    }

    fn ensure_history(&mut self, room_id: &str) {
        let tl = self.timelines.entry(room_id.to_string()).or_default();
        if tl.history == HistoryState::NotLoaded {
            tl.history = HistoryState::Loading;
            self.pending.push(Command::LoadHistory { room_id: room_id.into(), from: None });
        }
    }

    /// Older messages for the open room (scrolling past the start)
    pub fn load_older(&mut self) {
        let Some(id) = self.selected_room.clone() else { return };
        let Some(tl) = self.timelines.get_mut(&id) else { return };
        if tl.history == HistoryState::Partial {
            tl.history = HistoryState::Loading;
            self.pending.push(Command::LoadHistory { room_id: id, from: tl.prev_token.clone() });
        }
    }

    /// Scrolls the chat by `lines` (positive = towards older messages);
    /// past the loaded start, requests the previous page
    pub fn scroll_chat(&mut self, lines: isize) {
        if lines > 0 && self.chat_view.at_top {
            self.load_older();
        } else if lines > 0 {
            self.chat_view.scroll += lines as usize;
        } else {
            self.chat_view.scroll = self.chat_view.scroll.saturating_sub(lines.unsigned_abs());
        }
    }

    fn is_open(&self, room_id: &str) -> bool {
        self.selected_room.as_deref() == Some(room_id)
    }

    // ── Events ─────────────────────────────────────────────────────────────

    pub fn apply(&mut self, event: AppEvent) {
        match event {
            AppEvent::Message { room_id, message } => self.on_message(room_id, message),
            AppEvent::RoomsUpdated { rooms, spaces } => {
                let space_id = self.current_space().map(|s| s.id.clone());
                self.rooms = rooms;
                self.spaces = spaces;
                if let Some(id) = self.selected_room.clone() {
                    if let Some(r) = self.rooms.iter_mut().find(|r| r.id == id) { r.unread = 0; r.highlights = 0; }
                }
                // keep the same space selected even if the order changes
                self.space_idx = space_id
                    .and_then(|id| self.spaces.iter().position(|s| s.id == id))
                    .map_or(0, |i| i + 1);
                // preload history (and so previews) of every room
                let ids: Vec<String> = self.rooms.iter().filter(|r| r.invite.is_none()).map(|r| r.id.clone()).collect();
                for id in ids { self.ensure_history(&id); }
                self.refresh_entries();
                if self.selected_room.is_none() && self.selected_explore().is_none() {
                    self.move_room_cursor(0);
                }
                self.resolve_pending_open();
            }
            AppEvent::RoomsPatched(updated) => {
                for room in updated {
                    let open = self.is_open(&room.id);
                    match self.rooms.iter_mut().find(|r| r.id == room.id) {
                        Some(existing) => {
                            *existing = room;
                            if open { existing.unread = 0; existing.highlights = 0; }
                        }
                        None => self.rooms.push(room),
                    }
                }
                self.refresh_entries();
                self.resolve_pending_open();
            }
            AppEvent::History { room_id, messages, prev_token } => {
                let added = self.timelines.entry(room_id.clone()).or_default().add_history(messages, prev_token);
                // keep the selection on the same message
                if self.is_open(&room_id) {
                    if let Some(sel) = self.selected_msg.as_mut() { *sel += added; }
                }
                self.refresh_entries();
            }
            AppEvent::HistoryFailed { room_id, error } => {
                if let Some(tl) = self.timelines.get_mut(&room_id) {
                    tl.history = if tl.prev_token.is_some() { HistoryState::Partial } else { HistoryState::NotLoaded };
                }
                if self.is_open(&room_id) {
                    self.push_error(format!("History unavailable: {error}"));
                }
            }
            AppEvent::Joined { joined, space_id } => self.on_joined(joined, space_id),
            AppEvent::Left { room_id } => {
                let name = self.rooms.iter().find(|r| r.id == room_id).map(|r| r.name.clone())
                    .or_else(|| self.spaces.iter().find(|s| s.id == room_id).map(|s| s.name.clone()))
                    .unwrap_or_default();
                let space_id = self.current_space().map(|s| s.id.clone());
                self.rooms.retain(|r| r.id != room_id);
                self.spaces.retain(|s| s.id != room_id);
                self.timelines.remove(&room_id);
                self.space_idx = space_id
                    .and_then(|id| self.spaces.iter().position(|s| s.id == id))
                    .map_or(0, |i| i + 1);
                if self.is_open(&room_id) { self.selected_room = None; }
                self.refresh_entries();
                if self.selected_room.is_none() { self.move_room_cursor(0); }
                self.set_status(format!("Left {name}"));
            }
            AppEvent::SpaceChild { space_id, room_id, added } => {
                if let Some(space) = self.spaces.iter_mut().find(|s| s.id == space_id) {
                    if added { space.children.insert(room_id.clone()); } else { space.children.remove(&room_id); }
                    let space_name = space.name.clone();
                    let room = self.rooms.iter().find(|r| r.id == room_id).map(|r| r.name.clone()).unwrap_or_default();
                    self.set_status(if added { format!("{room} added to {space_name}") } else { format!("{room} removed from {space_name}") });
                }
                self.refresh_entries();
            }
            AppEvent::Redacted { room_id, event_id } => {
                let Some(pos) = self.timelines.get_mut(&room_id).and_then(|tl| tl.remove(&event_id)) else { return };
                if self.is_open(&room_id) {
                    // keep the selection on the neighbouring message
                    if let Some(sel) = self.selected_msg.as_mut() {
                        if *sel > pos { *sel -= 1; }
                    }
                }
                self.refresh_entries();
            }
            AppEvent::Hierarchy { space_id, entries } => {
                self.hierarchy.insert(space_id, entries);
                self.refresh_entries();
            }
            AppEvent::SearchResults { query, items } => {
                if let Some(Popup::Picker(p)) = self.popup.as_mut() {
                    if p.remote_query == query {
                        p.remote = items;
                        p.searching = false;
                    }
                }
            }
            AppEvent::SyncState(s) => {
                match (&self.sync_state, &s) {
                    (old, SyncState::Offline(e)) if old != &s => self.push_error(format!("Connection lost: {e}")),
                    (SyncState::Offline(_), SyncState::Online) => self.set_status("Connection restored"),
                    _ => {}
                }
                self.sync_state = s;
            }
            AppEvent::Security(s) => self.security = s,
            AppEvent::Verification { user_id, flow_id, stage } => self.on_verification(user_id, flow_id, stage),
            AppEvent::Recovered => {
                self.set_status("Recovery key accepted");
                self.redecrypt(None);
            }
            AppEvent::RoomKeys(keys) => self.redecrypt(keys),
            AppEvent::Decrypted { room_id, messages } => {
                let open = self.is_open(&room_id);
                let Some(tl) = self.timelines.get_mut(&room_id) else { return };
                for (event_id, msg) in messages {
                    let removed = msg.is_none();
                    let Some(pos) = tl.replace(&event_id, msg) else { continue };
                    if removed && open {
                        if let Some(sel) = self.selected_msg.as_mut() {
                            if *sel > pos { *sel -= 1; }
                        }
                    }
                }
                self.refresh_entries();
            }
            AppEvent::NeedLogin { homeserver, username, note } => {
                self.popup = Some(Popup::Login(LoginForm::new(&username, &homeserver, note)));
            }
            AppEvent::LoginFailed(msg) => match self.popup.as_mut() {
                Some(Popup::Login(form)) => {
                    form.busy = false;
                    form.error = Some(msg);
                }
                _ => self.push_error(msg),
            },
            AppEvent::UploadFinished { ok } => {
                self.uploads = self.uploads.saturating_sub(1);
                if ok { self.set_status("Attachment sent"); }
            }
            AppEvent::Info(msg) => self.set_status(msg),
            AppEvent::Error(msg) => self.push_error(msg),
        }
    }

    fn on_message(&mut self, room_id: String, message: Message) {
        let own = message.sender == self.user_id;
        let event_id = message.event_id.clone();
        let open = self.is_open(&room_id);
        if !self.timelines.entry(room_id.clone()).or_default().push(message) { return; }

        if let Some(room) = self.rooms.iter_mut().find(|r| r.id == room_id) {
            if open { room.unread = 0; } else if !own { room.unread = room.unread.saturating_add(1); }
        }
        if open && !own {
            self.pending.push(Command::MarkRead { room_id, event_id });
        }
        self.refresh_entries();
    }

    fn on_joined(&mut self, joined: Joined, space_id: Option<String>) {
        match joined {
            Joined::Room(room) => {
                let id = room.id.clone();
                self.set_status(format!("Joined {}", room.name));
                if let Some(space) = space_id.and_then(|sid| self.spaces.iter_mut().find(|s| s.id == sid)) {
                    space.children.insert(id.clone());
                }
                // an accepted invite becomes a normal room
                self.rooms.retain(|r| r.id != id);
                self.rooms.push(room);
                self.ensure_history(&id);
                self.open_room_anywhere(&id);
                // the sync may rebuild the list before it knows about the room
                self.pending_open = Some(id);
            }
            Joined::Space(space) => {
                self.set_status(format!("Joined space {}", space.name));
                let id = space.id.clone();
                self.rooms.retain(|r| r.id != id);
                if !self.spaces.iter().any(|s| s.id == id) { self.spaces.push(space); }
                if let Some(i) = self.spaces.iter().position(|s| s.id == id) {
                    self.show_spaces = true;
                    self.select_space(i + 1);
                    self.show_rooms = true;
                    self.focus_panel(Focus::Rooms);
                }
            }
        }
    }

    // ── Session and verification ───────────────────────────────────────────

    /// Account popup entries, in display order
    pub fn account_items(&self) -> Vec<AccountItem> {
        let mut items = vec![AccountItem::VerifyDevice, AccountItem::RecoveryKey];
        items.extend(self.accounts.iter().filter(|a| a.username != self.user_id).map(|a| AccountItem::Switch(a.username.clone())));
        items.push(AccountItem::AddAccount);
        items.push(AccountItem::Logout);
        items
    }

    pub fn account_action(&mut self, item: AccountItem) {
        self.popup = None;
        match item {
            AccountItem::VerifyDevice => self.start_verification(),
            AccountItem::RecoveryKey => self.open_text(TextKind::RecoveryKey),
            AccountItem::Switch(username) => {
                self.set_status(format!("Switching to {username}…"));
                self.pending.push(Command::SwitchAccount { username });
            }
            AccountItem::AddAccount => self.popup = Some(Popup::Login(LoginForm::default())),
            AccountItem::Logout => {
                self.popup = Some(Popup::Confirm {
                    title: "Log out".into(),
                    question: format!("Log out of {}? Local data on this device will be deleted", self.user_id),
                    action: popup::ConfirmAction::Logout,
                });
            }
        }
    }

    pub fn start_verification(&mut self) {
        self.popup = Some(Popup::Verify(VerifyPopup {
            user_id: self.user_id.clone(),
            flow_id: String::new(),
            stage: VerifyStage::Waiting("Sending request…".into()),
        }));
        self.pending.push(Command::StartVerification);
    }

    fn on_verification(&mut self, user_id: String, flow_id: String, stage: VerifyStage) {
        if stage == VerifyStage::Done {
            if user_id == self.user_id { self.set_status("Session verified"); }
            // after verification, keys and secrets arrive from the other devices
            self.redecrypt(None);
        }
        match self.popup.as_mut() {
            // a request sent from here now gets its ID
            Some(Popup::Verify(v)) if v.flow_id == flow_id || (v.flow_id.is_empty() && v.user_id == user_id) => {
                v.flow_id = flow_id;
                v.stage = stage;
            }
            // a new request takes over any other popup; updates about
            // already closed verifications are ignored
            _ if matches!(stage, VerifyStage::Incoming { .. } | VerifyStage::Emojis(_)) => {
                self.popup = Some(Popup::Verify(VerifyPopup { user_id, flow_id, stage }));
            }
            _ => {}
        }
    }

    /// Retries decryption: only for the given (room, session) keys, or everything
    fn redecrypt(&mut self, keys: Option<Vec<(String, String)>>) {
        let mut by_room: HashMap<String, HashSet<String>> = HashMap::new();
        for (room, session) in keys.iter().flatten() {
            by_room.entry(room.clone()).or_default().insert(session.clone());
        }
        for (room_id, tl) in &self.timelines {
            let events = match &keys {
                None => tl.undecrypted(None),
                Some(_) => match by_room.get(room_id) {
                    Some(sessions) => tl.undecrypted(Some(&sessions.iter().map(String::as_str).collect())),
                    None => continue,
                },
            };
            if !events.is_empty() {
                self.pending.push(Command::Redecrypt { room_id: room_id.clone(), events });
            }
        }
    }

    fn resolve_pending_open(&mut self) {
        let Some(id) = self.pending_open.clone() else { return };
        if self.rooms.iter().any(|r| r.id == id) {
            if !self.is_open(&id) { self.open_room_anywhere(&id); }
            self.pending_open = None;
        }
    }

    // ── Popups and searches ────────────────────────────────────────────────

    pub fn open_text(&mut self, kind: TextKind) {
        self.popup = Some(Popup::Text(TextPopup { kind, input: LineInput::default(), completions: Vec::new() }));
    }

    pub fn open_picker(&mut self, kind: PickerKind) {
        self.popup = Some(Popup::Picker(Picker::new(kind)));
        self.picker_changed();
    }

    /// Recomputes local picker results after the text changes
    pub fn picker_changed(&mut self) {
        let Some(Popup::Picker(picker)) = self.popup.as_ref() else { return };
        let query = picker.input.as_str().trim().to_string();
        let kind = picker.kind;
        let local = match kind {
            PickerKind::Join => self.join_candidates(&query),
            PickerKind::Dm => self.dm_candidates(&query),
            PickerKind::Switch => self.switch_candidates(&query),
        };
        if let Some(Popup::Picker(p)) = self.popup.as_mut() {
            p.local = local;
            p.cursor = 0;
            if p.wants_remote() {
                p.edited_at = Some(Instant::now());
            } else {
                p.remote.clear();
                p.remote_query.clear();
                p.searching = false;
                p.edited_at = None;
            }
        }
    }

    fn join_candidates(&self, query: &str) -> Vec<PickItem> {
        let mut items = Vec::new();
        if let Ok((id, _)) = address::parse_room_address(query) {
            items.push(PickItem {
                title: format!("Join {id}"),
                subtitle: "room or space address".into(),
                tag: String::new(),
                action: PickAction::Join(query.to_string()),
            });
        }
        if !query.is_empty() {
            // rooms of the current space not joined yet
            for (_, h) in self.explore_list() {
                if fuzzy_score(&h.name, query).is_some() {
                    items.push(PickItem {
                        title: h.name.clone(),
                        subtitle: h.alias.clone().unwrap_or_default(),
                        tag: if h.is_space { "space".into() } else { format!("{} members", h.members) },
                        action: PickAction::Join(h.address()),
                    });
                }
            }
        }
        items
    }

    fn dm_candidates(&self, query: &str) -> Vec<PickItem> {
        let mut items: Vec<(usize, PickItem)> = self.rooms.iter()
            .filter(|r| r.is_direct)
            .filter_map(|r| {
                let hay = format!("{} {}", r.name, r.dm_with.join(" "));
                let score = fuzzy_score(&hay, query)?;
                Some((score, PickItem {
                    title: r.name.clone(),
                    subtitle: r.dm_with.join(", "),
                    tag: "existing chat".into(),
                    action: PickAction::Open(r.id.clone()),
                }))
            })
            .collect();
        items.sort_by_key(|(s, _)| *s);
        let mut out: Vec<PickItem> = items.into_iter().map(|(_, i)| i).take(5).collect();
        if address::parse_user_id(query).is_some() && !out.iter().any(|i| i.subtitle == query) {
            out.insert(0, PickItem {
                title: format!("New chat with {query}"),
                subtitle: String::new(),
                tag: String::new(),
                action: PickAction::Dm(query.to_string()),
            });
        }
        out
    }

    fn switch_candidates(&self, query: &str) -> Vec<PickItem> {
        let mut scored: Vec<(usize, Option<DateTime<Utc>>, PickItem)> = self.rooms.iter()
            .filter_map(|r| {
                let hay = format!("{} {}", r.name, r.alias.as_deref().unwrap_or(""));
                let score = fuzzy_score(&hay, query)?;
                let tag = if r.unread > 0 { format!("{} unread", r.unread) } else { String::new() };
                Some((score, self.last_activity(&r.id), PickItem {
                    title: format!("{} {}", if r.is_direct { "@" } else { "#" }, r.name),
                    subtitle: r.alias.clone().or_else(|| r.dm_with.first().cloned()).unwrap_or_default(),
                    tag,
                    action: PickAction::Open(r.id.clone()),
                }))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        scored.into_iter().map(|(_, _, i)| i).take(50).collect()
    }

    pub fn pick(&mut self, item: PickItem) {
        self.popup = None;
        match item.action {
            PickAction::Open(id) => {
                self.open_room_anywhere(&id);
                self.enter_insert();
            }
            PickAction::Join(address) => {
                self.set_status(format!("Joining {address}…"));
                self.pending.push(Command::Join { address });
            }
            PickAction::Dm(user_id) => {
                let existing = self.rooms.iter().find(|r| r.is_direct && r.dm_with.contains(&user_id));
                match existing {
                    Some(r) => {
                        let id = r.id.clone();
                        self.open_room_anywhere(&id);
                        self.enter_insert();
                    }
                    None => {
                        self.set_status(format!("Opening chat with {user_id}…"));
                        self.pending.push(Command::OpenDm { user_id });
                    }
                }
            }
        }
    }

    // ── Status, errors and timers ──────────────────────────────────────────

    pub fn set_status(&mut self, msg: impl Into<String>) {
        let secs = self.ui.behavior.status_timeout_secs;
        self.status = Some((msg.into(), Instant::now() + Duration::from_secs(secs)));
    }

    pub fn push_error(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if let Some(last) = self.errors.back_mut() {
            if last.msg == msg {
                last.count += 1;
                last.at = Instant::now();
                return;
            }
        }
        self.errors.push_back(ErrorEntry { msg, at: Instant::now(), count: 1 });
        while self.errors.len() > MAX_ERRORS { self.errors.pop_front(); }
    }

    /// Status message opacity (1.0 → 0.0 while fading)
    pub fn status_opacity(&self) -> f32 {
        let Some((_, expires)) = &self.status else { return 0.0 };
        let remaining = expires.saturating_duration_since(Instant::now()).as_secs_f32();
        (remaining / self.ui.behavior.status_fade_secs.max(0.01)).min(1.0)
    }

    /// Advances timers, fades and debounced searches. True if a redraw is needed.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let mut dirty = false;

        if let Some((_, expires)) = self.status {
            if now >= expires {
                self.status = None;
                dirty = true;
            } else {
                dirty |= expires.duration_since(now).as_secs_f32() < self.ui.behavior.status_fade_secs;
            }
        }

        let timeout = self.ui.behavior.error_timeout_secs;
        if timeout > 0 {
            let before = self.errors.len();
            self.errors.retain(|e| now.duration_since(e.at) < Duration::from_secs(timeout));
            dirty |= self.errors.len() != before;
        }

        // server search after a short pause in typing
        if let Some(Popup::Picker(p)) = self.popup.as_mut() {
            if p.edited_at.is_some_and(|t| now.duration_since(t) >= SEARCH_DEBOUNCE) {
                p.edited_at = None;
                let query = p.input.as_str().trim().to_string();
                if query != p.remote_query {
                    p.remote_query = query.clone();
                    p.searching = true;
                    self.pending.push(match p.kind {
                        PickerKind::Dm => Command::SearchUsers { query },
                        _ => Command::SearchRooms { query },
                    });
                    dirty = true;
                }
            }
        }
        dirty
    }
}
