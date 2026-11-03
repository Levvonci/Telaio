//! Data model independent of matrix-sdk (apart from the media source).

use chrono::{DateTime, Utc};
use matrix_sdk::ruma::events::room::MediaSource;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Message {
    pub event_id: String,
    /// Sender MXID
    pub sender: String,
    pub sender_name: String,
    pub timestamp: DateTime<Utc>,
    pub content: Content,
}

#[derive(Debug, Clone)]
pub enum Content {
    Text(String),
    Notice(String),
    Emote(String),
    Attachment(Attachment),
    /// Encrypted message that could not be decrypted
    Encrypted(Undecrypted),
    /// Unsupported types (location, polls…), shown as text
    Other(String),
}

/// Known account (configured or with a saved session)
#[derive(Debug, Clone)]
pub struct AccountEntry {
    pub username: String,
    /// Opens without a password
    pub has_session: bool,
}

/// Encrypted event kept to retry once its key arrives
#[derive(Debug, Clone, Default)]
pub struct Undecrypted {
    /// Megolm session: identifies the key needed
    pub session_id: Option<String>,
    /// Original event JSON
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttachmentKind { Image, Video, Audio, File }

#[derive(Debug, Clone)]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub name: String,
    pub size: Option<u64>,
    pub mimetype: Option<String>,
    pub source: MediaSource,
}

impl Message {
    /// Short preview for the room list
    pub fn preview(&self) -> String {
        match &self.content {
            Content::Text(t) | Content::Notice(t) | Content::Other(t) => t.lines().next().unwrap_or("").to_string(),
            Content::Emote(t) => format!("* {} {}", self.sender_name, t),
            Content::Attachment(a) => format!("[{}] {}", a.kind.label(), a.name),
            Content::Encrypted(_) => "encrypted message".into(),
        }
    }

    pub fn attachment(&self) -> Option<&Attachment> {
        match &self.content {
            Content::Attachment(a) => Some(a),
            _ => None,
        }
    }
}

impl AttachmentKind {
    pub fn label(self) -> &'static str {
        match self {
            AttachmentKind::Image => "image",
            AttachmentKind::Video => "video",
            AttachmentKind::Audio => "audio",
            AttachmentKind::File => "file",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub topic: Option<String>,
    pub alias: Option<String>,
    pub unread: u64,
    /// Mentions/highlights among the unread
    pub highlights: u64,
    pub is_direct: bool,
    /// For DMs: the other participants
    pub dm_with: Vec<String>,
    pub members: u64,
    pub encrypted: bool,
    /// Set if the user was invited but has not joined yet
    pub invite: Option<InviteInfo>,
}

#[derive(Debug, Clone)]
pub struct InviteInfo {
    /// Name (or MXID) of the inviter
    pub inviter: String,
    pub is_space: bool,
}

#[derive(Debug, Clone)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub children: HashSet<String>,
}

/// Room or sub-space listed in a space hierarchy
#[derive(Debug, Clone)]
pub struct HierarchyEntry {
    pub id: String,
    pub name: String,
    pub alias: Option<String>,
    pub topic: Option<String>,
    pub members: u64,
    pub is_space: bool,
}

impl HierarchyEntry {
    /// Address used to join (the alias if present, it is more robust)
    pub fn address(&self) -> String {
        self.alias.clone().unwrap_or_else(|| self.id.clone())
    }
}
