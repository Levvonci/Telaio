//! Messages exchanged between the UI and background tasks.

use std::path::PathBuf;

use super::model::{Attachment, HierarchyEntry, Message, Room, Space};
use super::popup::PickItem;

/// Events produced by Matrix tasks and applied to the state by the main loop
#[derive(Debug)]
pub enum AppEvent {
    Message { room_id: String, message: Message },
    /// Full list of rooms and spaces (replaces the current one)
    RoomsUpdated { rooms: Vec<Room>, spaces: Vec<Space> },
    /// Only the rooms touched by the last sync
    RoomsPatched(Vec<Room>),
    /// History page, in chronological order
    History { room_id: String, messages: Vec<Message>, prev_token: Option<String> },
    HistoryFailed { room_id: String, error: String },
    /// Joined (or just created) a room or space
    Joined { joined: Joined, space_id: Option<String> },
    Left { room_id: String },
    SpaceChild { space_id: String, room_id: String, added: bool },
    Hierarchy { space_id: String, entries: Vec<HierarchyEntry> },
    Redacted { room_id: String, event_id: String },
    SearchResults { query: String, items: Vec<PickItem> },
    SyncState(SyncState),
    /// Session verification and key backup state
    Security(Security),
    /// Progress of an interactive (emoji) verification
    Verification { user_id: String, flow_id: String, stage: VerifyStage },
    Recovered,
    /// New room keys: (room, session). `None` = some were missed, so every
    /// encrypted message is retried
    RoomKeys(Option<Vec<(String, String)>>),
    /// Result of a retried decryption: `None` = the event is not shown
    /// (e.g. it was an edit)
    Decrypted { room_id: String, messages: Vec<(String, Option<Message>)> },
    /// The chosen account has no valid session: a password is needed
    NeedLogin { homeserver: String, username: String, note: Option<String> },
    LoginFailed(String),
    UploadFinished { ok: bool },
    Info(String),
    Error(String),
}

#[derive(Debug)]
pub enum Joined {
    Room(Room),
    Space(Space),
}

/// Operations requested by the UI, run in the background
#[derive(Debug, Clone)]
pub enum Command {
    SendText { room_id: String, body: String },
    SendFile { room_id: String, path: PathBuf },
    /// Saves the attachment to the download folder
    Download { attachment: Attachment },
    /// Downloads (if needed) and opens the attachment with the default program
    Open { attachment: Attachment },
    Join { address: String },
    Leave { room_id: String },
    CreateSpace { name: String },
    CreateRoom { name: String, space_id: Option<String> },
    OpenDm { user_id: String },
    Invite { room_id: String, user_id: String },
    SetSpaceChild { space_id: String, room_id: String, add: bool },
    /// Kicks/bans/unbans a user (MXID or display name)
    Moderate { room_id: String, action: ModAction, target: String, reason: Option<String> },
    SetTopic { room_id: String, topic: String },
    SetName { room_id: String, name: String },
    Redact { room_id: String, event_id: String },
    LoadHistory { room_id: String, from: Option<String> },
    LoadHierarchy { space_id: String },
    MarkRead { room_id: String, event_id: String },
    SearchUsers { query: String },
    SearchRooms { query: String },
    /// Asks the other devices to verify this session
    StartVerification,
    VerifyAccept { user_id: String, flow_id: String },
    /// The emojis match
    VerifyConfirm { user_id: String, flow_id: String },
    VerifyMismatch { user_id: String, flow_id: String },
    VerifyCancel { user_id: String, flow_id: String },
    /// Unlocks cross-signing and backup with the recovery key (or passphrase)
    Recover { key: String },
    /// Retries decrypting a room's events (original JSON)
    Redecrypt { room_id: String, events: Vec<String> },
    /// Switches to another account with a saved session
    SwitchAccount { username: String },
    Login { homeserver: String, username: String, password: String },
    /// Logs out of the current account, deleting local data
    Logout,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModAction { Kick, Ban, Unban }

impl ModAction {
    /// Completed action, for status messages
    pub fn done(self) -> &'static str {
        match self {
            ModAction::Kick => "was kicked",
            ModAction::Ban => "was banned",
            ModAction::Unban => "was unbanned",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Security {
    /// `None` until known
    pub verified: Option<bool>,
    pub recovery: RecoveryStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum RecoveryStatus {
    #[default]
    Unknown,
    /// Backup and cross-signing keys available on this device
    Enabled,
    /// Backup set up on the account but not unlocked here yet
    Incomplete,
    /// The account has no key backup
    Disabled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerifyStage {
    /// Request received from another device, to be accepted
    Incoming { device: String },
    /// Waiting for the other device (text to show)
    Waiting(String),
    /// Emojis to compare: (symbol, name)
    Emojis(Vec<(String, String)>),
    Done,
    Cancelled(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncState {
    Connecting,
    Online,
    Offline(String),
}
