//! Paginated history loading.

use anyhow::{Context, Result};
use matrix_sdk::{
    room::MessagesOptions,
    ruma::{api::client::filter::RoomEventFilter, assign, UInt},
    Client,
};

use super::{convert, get_room};
use crate::app::model::Message;

/// Loads a history page going back from `from` (or from the end). Returns the
/// messages in chronological order and the token for the next page (`None`
/// at the start of the room).
pub async fn load(client: &Client, room_id: &str, from: Option<&str>, limit: u64) -> Result<(Vec<Message>, Option<String>)> {
    let room = get_room(client, room_id)?;
    let mut options = MessagesOptions::backward().from(from);
    options.limit = UInt::new(limit.clamp(1, 500)).unwrap_or(UInt::MIN);
    // messages only (including encrypted): no pages full of state events
    options.filter = assign!(RoomEventFilter::default(), {
        types: Some(vec!["m.room.message".to_owned(), "m.room.encrypted".to_owned()]),
    });

    let page = room.messages(options).await.context("History request failed")?;
    let mut messages = Vec::with_capacity(page.chunk.len());
    for ev in page.chunk.iter().rev() {
        if let Some(m) = convert::from_timeline_event(&room, ev.raw()).await {
            messages.push(m);
        }
    }
    let next = if page.chunk.is_empty() { None } else { page.end };
    Ok((messages, next))
}
