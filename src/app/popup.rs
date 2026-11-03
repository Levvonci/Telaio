//! Popups: text fields, searches with results, confirmations.

use std::time::{Duration, Instant};

use super::event::{Command, ModAction, VerifyStage};
use super::input::LineInput;

/// Delay after the last keystroke before querying the server
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);

pub enum Popup {
    Text(TextPopup),
    Picker(Picker),
    /// Adds/removes the room from spaces (cursor over the space list)
    ManageSpaces { room_id: String, cursor: usize },
    Confirm { title: String, question: String, action: ConfirmAction },
    /// Session and accounts: verification, switching, logout
    Account { cursor: usize },
    Login(LoginForm),
    Verify(VerifyPopup),
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextKind { CreateSpace, CreateRoom, Invite, Attach, RecoveryKey }

impl TextKind {
    /// The text is a secret: shown as dots
    pub fn masked(self) -> bool { self == TextKind::RecoveryKey }
}

/// Selectable entries of the account popup
#[derive(Debug, Clone, PartialEq)]
pub enum AccountItem {
    VerifyDevice,
    RecoveryKey,
    Switch(String),
    AddAccount,
    Logout,
}

/// Login form shown inside the TUI
#[derive(Default)]
pub struct LoginForm {
    pub user: LineInput,
    pub server: LineInput,
    pub password: LineInput,
    /// Active field: 0 user, 1 server, 2 password
    pub field: usize,
    /// Login in progress
    pub busy: bool,
    pub error: Option<String>,
}

impl LoginForm {
    pub const FIELDS: usize = 3;

    pub fn new(username: &str, homeserver: &str, note: Option<String>) -> Self {
        let mut form = Self { error: note, ..Self::default() };
        form.user.set(username);
        form.server.set(homeserver);
        // known user: start from the password
        form.field = if username.is_empty() { 0 } else { 2 };
        form
    }

    pub fn input(&mut self) -> &mut LineInput {
        match self.field {
            0 => &mut self.user,
            1 => &mut self.server,
            _ => &mut self.password,
        }
    }
}

/// Interactive verification in progress (or just finished)
pub struct VerifyPopup {
    pub user_id: String,
    /// Empty until a request sent from here gets an ID
    pub flow_id: String,
    pub stage: VerifyStage,
}

pub struct TextPopup {
    pub kind: TextKind,
    pub input: LineInput,
    /// Path completions (attachments only)
    pub completions: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    Leave { room_id: String },
    Moderate { room_id: String, action: ModAction, target: String, reason: Option<String> },
    Redact { room_id: String, event_id: String },
    Logout,
}

impl ConfirmAction {
    pub fn into_command(self) -> Command {
        match self {
            ConfirmAction::Leave { room_id } => Command::Leave { room_id },
            ConfirmAction::Moderate { room_id, action, target, reason } => Command::Moderate { room_id, action, target, reason },
            ConfirmAction::Redact { room_id, event_id } => Command::Redact { room_id, event_id },
            ConfirmAction::Logout => Command::Logout,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickerKind {
    /// Join a room or space: address or directory search
    Join,
    /// Direct chat: user directory search
    Dm,
    /// Quick switch between joined rooms
    Switch,
}

#[derive(Debug, Clone)]
pub enum PickAction {
    Join(String),
    Open(String),
    Dm(String),
}

#[derive(Debug, Clone)]
pub struct PickItem {
    pub title: String,
    pub subtitle: String,
    /// Short right-aligned label (e.g. "space", "123 members")
    pub tag: String,
    pub action: PickAction,
}

pub struct Picker {
    pub kind: PickerKind,
    pub input: LineInput,
    /// Local results, computed on every keystroke
    pub local: Vec<PickItem>,
    /// Server results for `remote_query`
    pub remote: Vec<PickItem>,
    pub remote_query: String,
    pub cursor: usize,
    pub searching: bool,
    /// Last text change not yet sent to the server
    pub edited_at: Option<Instant>,
}

impl Picker {
    pub fn new(kind: PickerKind) -> Self {
        Self {
            kind,
            input: LineInput::default(),
            local: Vec::new(),
            remote: Vec::new(),
            remote_query: String::new(),
            cursor: 0,
            searching: false,
            edited_at: None,
        }
    }

    /// Local results first, then server results without duplicates
    pub fn items(&self) -> Vec<&PickItem> {
        let mut out: Vec<&PickItem> = self.local.iter().collect();
        for r in &self.remote {
            let dup = self.local.iter().any(|l| same_target(&l.action, &r.action));
            if !dup { out.push(r); }
        }
        out
    }

    pub fn selected(&self) -> Option<PickItem> {
        let items = self.items();
        items.get(self.cursor.min(items.len().saturating_sub(1))).map(|i| (*i).clone())
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.items().len();
        if n == 0 { self.cursor = 0; return; }
        self.cursor = (self.cursor as isize + delta).clamp(0, n as isize - 1) as usize;
    }

    /// Whether a server search makes sense for the current text
    pub fn wants_remote(&self) -> bool {
        self.kind != PickerKind::Switch && self.input.as_str().trim().chars().count() >= 2
    }
}

fn same_target(a: &PickAction, b: &PickAction) -> bool {
    match (a, b) {
        (PickAction::Join(x), PickAction::Join(y)) | (PickAction::Dm(x), PickAction::Dm(y)) => x == y,
        (PickAction::Open(x), PickAction::Join(y)) | (PickAction::Join(y), PickAction::Open(x)) => x == y,
        _ => false,
    }
}

/// Simple fuzzy match: every letter of the query, in order
pub fn fuzzy_score(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    if n.is_empty() { return Some(0); }
    if let Some(pos) = h.find(&n) {
        return Some(pos); // substring: best score
    }
    let mut chars = h.chars();
    let mut gaps = 100;
    for c in n.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(x) if x == c => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(gaps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy() {
        assert_eq!(fuzzy_score("Rust Italia", "ital"), Some(5));
        assert!(fuzzy_score("Rust Italia", "rsi").is_some());
        assert!(fuzzy_score("Rust", "xyz").is_none());
    }
}
