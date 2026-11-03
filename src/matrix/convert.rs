//! Matrix events → internal model.

use chrono::{DateTime, Utc};
use matrix_sdk::{
    ruma::{
        events::{
            room::{
                encrypted::{EncryptedEventScheme, OriginalSyncRoomEncryptedEvent},
                message::{MessageType, OriginalSyncRoomMessageEvent, Relation},
            },
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
        },
        serde::Raw,
        MilliSecondsSinceUnixEpoch, UInt, UserId,
    },
    Room,
};

use crate::app::model::{Attachment, AttachmentKind, Content, Message, Undecrypted};

pub fn to_datetime(ts: MilliSecondsSinceUnixEpoch) -> DateTime<Utc> {
    let ms: u64 = ts.get().into();
    DateTime::from_timestamp_millis(ms as i64).unwrap_or_else(Utc::now)
}

/// Member display name (from the local store, no network request)
pub async fn member_name(room: &Room, user: &UserId) -> String {
    match room.get_member_no_sync(user).await {
        Ok(Some(member)) => member.display_name().unwrap_or(user.localpart()).to_string(),
        _ => user.localpart().to_string(),
    }
}

/// Message from an m.room.message event; `None` for edits and ignored types
pub async fn from_message_event(room: &Room, ev: OriginalSyncRoomMessageEvent) -> Option<Message> {
    // edits ("* corrected text") are not messages of their own
    if matches!(ev.content.relates_to, Some(Relation::Replacement(_))) {
        return None;
    }
    let content = convert_content(ev.content.msgtype)?;
    Some(Message {
        event_id: ev.event_id.to_string(),
        sender_name: member_name(room, &ev.sender).await,
        sender: ev.sender.to_string(),
        timestamp: to_datetime(ev.origin_server_ts),
        content,
    })
}

/// Message from a generic timeline event (history and retried decryption)
pub async fn from_timeline_event(room: &Room, raw: &Raw<AnySyncTimelineEvent>) -> Option<Message> {
    let AnySyncTimelineEvent::MessageLike(ev) = raw.deserialize().ok()? else { return None };
    match ev {
        AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(m)) => from_message_event(room, m).await,
        // still encrypted: the key is not available on this device
        AnySyncMessageLikeEvent::RoomEncrypted(SyncMessageLikeEvent::Original(e)) => {
            Some(from_encrypted_event(room, &e, raw.json().get().to_owned()).await)
        }
        _ => None,
    }
}

/// Placeholder for an undecrypted event; `raw` is its original JSON
pub async fn from_encrypted_event(room: &Room, ev: &OriginalSyncRoomEncryptedEvent, raw: String) -> Message {
    let session_id = match &ev.content.scheme {
        EncryptedEventScheme::MegolmV1AesSha2(c) => Some(c.session_id.clone()),
        _ => None,
    };
    Message {
        event_id: ev.event_id.to_string(),
        sender_name: member_name(room, &ev.sender).await,
        sender: ev.sender.to_string(),
        timestamp: to_datetime(ev.origin_server_ts),
        content: Content::Encrypted(Undecrypted { session_id, raw }),
    }
}

fn convert_content(msgtype: MessageType) -> Option<Content> {
    let size = |s: Option<UInt>| s.map(u64::from);
    let attachment = |kind, name, size, mimetype, source| Content::Attachment(Attachment { kind, name, size, mimetype, source });
    Some(match msgtype {
        MessageType::Text(t) => Content::Text(t.body),
        MessageType::Notice(n) => Content::Notice(n.body),
        MessageType::Emote(e) => Content::Emote(e.body),
        MessageType::Image(c) => {
            let (s, m) = c.info.map(|i| (size(i.size), i.mimetype)).unwrap_or_default();
            attachment(AttachmentKind::Image, c.filename.unwrap_or(c.body), s, m, c.source)
        }
        MessageType::Video(c) => {
            let (s, m) = c.info.map(|i| (size(i.size), i.mimetype)).unwrap_or_default();
            attachment(AttachmentKind::Video, c.filename.unwrap_or(c.body), s, m, c.source)
        }
        MessageType::Audio(c) => {
            let (s, m) = c.info.map(|i| (size(i.size), i.mimetype)).unwrap_or_default();
            attachment(AttachmentKind::Audio, c.filename.unwrap_or(c.body), s, m, c.source)
        }
        MessageType::File(c) => {
            let (s, m) = c.info.map(|i| (size(i.size), i.mimetype)).unwrap_or_default();
            attachment(AttachmentKind::File, c.filename.unwrap_or(c.body), s, m, c.source)
        }
        MessageType::VerificationRequest(_) => return None,
        other => Content::Other(other.body().to_string()),
    })
}
