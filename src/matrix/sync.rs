//! Continuous sync and construction of the room and space lists.

use futures::future::join_all;
use matrix_sdk::{
    config::SyncSettings,
    ruma::{
        api::{
            client::filter::{FilterDefinition, LazyLoadOptions, RoomEventFilter, RoomFilter},
            error::ErrorKind,
        },
        assign,
        events::{
            room::{
                encrypted::OriginalSyncRoomEncryptedEvent, message::OriginalSyncRoomMessageEvent,
                redaction::OriginalSyncRoomRedactionEvent,
            },
            space::child::SpaceChildEventContent,
        },
        RoomId,
    },
    event_handler::RawEvent,
    Client, LoopCtrl, Room, RoomDisplayName, RoomState,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{convert, short_error, verify, Tx};
use crate::app::event::{AppEvent, SyncState};
use crate::app::model::{InviteInfo, Room as AppRoom, Space};

/// Sync plus the session's background watchers (verification state, incoming
/// keys). They all stop together when the task is aborted.
pub async fn run(client: Client, tx: Tx) {
    tokio::join!(
        sync_loop(client.clone(), tx.clone()),
        verify::watch_security(client.clone(), tx.clone()),
        verify::watch_room_keys(client, tx),
    );
}

/// Continuous sync with exponential backoff on network errors
async fn sync_loop(client: Client, tx: Tx) {
    register_handlers(&client, &tx);

    // with a persistent store the rooms are already known: show them right away
    if !client.rooms().is_empty() {
        let (rooms, spaces) = collect(&client).await;
        let _ = tx.send(AppEvent::RoomsUpdated { rooms, spaces });
    }

    let filter = assign!(FilterDefinition::default(), {
        room: assign!(RoomFilter::default(), {
            state: assign!(RoomEventFilter::default(), {
                lazy_load_options: LazyLoadOptions::Enabled { include_redundant_members: false },
            }),
        }),
    });
    let known: Arc<Mutex<HashSet<String>>> = Arc::default();
    let mut backoff = Duration::from_secs(2);

    loop {
        let settings = SyncSettings::default().timeout(Duration::from_secs(30)).filter(filter.clone().into());
        let result = client
            .sync_with_callback(settings, |response| {
                let client = client.clone();
                let tx = tx.clone();
                let known = known.clone();
                async move {
                    let _ = tx.send(AppEvent::SyncState(SyncState::Online));
                    let rooms = &response.rooms;
                    let joined: Vec<String> = rooms.joined.keys().map(|k| k.to_string()).collect();

                    // full rebuild only if the set of rooms changed; otherwise
                    // update just the rooms touched by this sync
                    let full = {
                        let known = known.lock().unwrap();
                        known.is_empty()
                            || !rooms.left.is_empty()
                            || !rooms.invited.is_empty()
                            || joined.iter().any(|id| !known.contains(id) || is_space(&client, id))
                    };
                    if full {
                        let (rooms, spaces) = collect(&client).await;
                        let mut known = known.lock().unwrap();
                        known.clear();
                        known.extend(rooms.iter().map(|r| r.id.clone()).chain(spaces.iter().map(|s| s.id.clone())));
                        let _ = tx.send(AppEvent::RoomsUpdated { rooms, spaces });
                    } else if !joined.is_empty() {
                        let rooms = joined.iter().filter_map(|id| client.get_room(&RoomId::parse(id).ok()?));
                        let updated = join_all(rooms.map(|r| async move { room_info(&r).await })).await;
                        let _ = tx.send(AppEvent::RoomsPatched(updated.into_iter().flatten().collect()));
                    }
                    LoopCtrl::Continue
                }
            })
            .await;

        let Err(e) = result else { return };
        if matches!(e.client_api_error_kind(), Some(ErrorKind::UnknownToken(_))) {
            let _ = tx.send(AppEvent::Error("Session no longer valid: sign in again from F2 or restart telaio".into()));
            return;
        }
        let _ = tx.send(AppEvent::SyncState(SyncState::Offline(short_error(&e))));
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

fn register_handlers(client: &Client, tx: &Tx) {
    let tx_msg = tx.clone();
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
        let tx = tx_msg.clone();
        async move {
            if let Some(message) = convert::from_message_event(&room, ev).await {
                let _ = tx.send(AppEvent::Message { room_id: room.room_id().to_string(), message });
            }
        }
    });

    let tx_red = tx.clone();
    client.add_event_handler(move |ev: OriginalSyncRoomRedactionEvent, room: Room| {
        let tx = tx_red.clone();
        async move {
            // since room version 11 the field lives in the content
            if let Some(target) = ev.redacts.or(ev.content.redacts) {
                let _ = tx.send(AppEvent::Redacted { room_id: room.room_id().to_string(), event_id: target.to_string() });
            }
        }
    });

    // events the SDK could not decrypt
    let tx_utd = tx.clone();
    client.add_event_handler(move |ev: OriginalSyncRoomEncryptedEvent, room: Room, raw: RawEvent| {
        let tx = tx_utd.clone();
        async move {
            // the original JSON is kept to retry once the key arrives
            let message = convert::from_encrypted_event(&room, &ev, raw.0.get().to_owned()).await;
            let _ = tx.send(AppEvent::Message { room_id: room.room_id().to_string(), message });
        }
    });

    verify::register_handlers(client, tx);
}

fn is_space(client: &Client, id: &str) -> bool {
    RoomId::parse(id).ok().and_then(|r| client.get_room(&r)).is_some_and(|r| r.is_space())
}

/// Full list of joined rooms and spaces, plus invites
pub async fn collect(client: &Client) -> (Vec<AppRoom>, Vec<Space>) {
    let all = client.rooms();
    let invited: Vec<&Room> = all.iter().filter(|r| r.state() == RoomState::Invited).collect();
    let (spaces, rooms): (Vec<&Room>, Vec<&Room>) = all
        .iter()
        .filter(|r| r.state() == RoomState::Joined)
        .partition(|r| r.is_space());
    let mut rooms: Vec<AppRoom> = join_all(rooms.into_iter().map(room_info)).await.into_iter().flatten().collect();
    rooms.extend(join_all(invited.into_iter().map(invite_info)).await);
    let mut spaces: Vec<Space> = join_all(spaces.into_iter().map(space_info)).await;
    spaces.sort_by_key(|s| s.name.to_lowercase());
    (rooms, spaces)
}

/// Display name: never "Empty Room" (falls back to alias or ID)
pub async fn display_name(room: &Room) -> String {
    match room.display_name().await {
        Ok(RoomDisplayName::Named(n) | RoomDisplayName::Aliased(n) | RoomDisplayName::Calculated(n) | RoomDisplayName::EmptyWas(n)) => n,
        _ => room
            .canonical_alias()
            .map(|a| a.to_string())
            .unwrap_or_else(|| room.room_id().to_string()),
    }
}

pub async fn room_info(room: &Room) -> Option<AppRoom> {
    if room.state() != RoomState::Joined || room.is_space() {
        return None;
    }
    let counts = room.unread_notification_counts();
    Some(AppRoom {
        id: room.room_id().to_string(),
        name: display_name(room).await,
        topic: room.topic().filter(|t| !t.trim().is_empty()),
        alias: room.canonical_alias().map(|a| a.to_string()),
        unread: counts.notification_count,
        highlights: counts.highlight_count,
        is_direct: room.is_direct().await.unwrap_or(false),
        dm_with: room.direct_targets().iter().filter_map(|t| t.as_user_id()).map(|u| u.to_string()).collect(),
        members: room.joined_members_count(),
        encrypted: room.encryption_state().is_encrypted(),
        invite: None,
    })
}

async fn invite_info(room: &Room) -> AppRoom {
    let inviter = match room.invite_details().await {
        Ok(details) => details
            .inviter
            .and_then(|m| m.display_name().map(str::to_owned))
            .unwrap_or_else(|| details.inviter_id.to_string()),
        Err(_) => String::new(),
    };
    AppRoom {
        id: room.room_id().to_string(),
        name: display_name(room).await,
        topic: room.topic().filter(|t| !t.trim().is_empty()),
        alias: room.canonical_alias().map(|a| a.to_string()),
        unread: 0,
        highlights: 0,
        is_direct: room.is_direct().await.unwrap_or(false),
        dm_with: Vec::new(),
        members: room.joined_members_count(),
        encrypted: room.encryption_state().is_encrypted(),
        invite: Some(InviteInfo { inviter, is_space: room.is_space() }),
    }
}

pub async fn space_info(room: &Room) -> Space {
    let children = room
        .get_state_events_static::<SpaceChildEventContent>()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|ev| ev.deserialize().ok())
        // an m.space.child without "via" means the room was removed
        .filter(|ev| ev.as_sync().and_then(|s| s.as_original()).is_some_and(|o| !o.content.via.is_empty()))
        .map(|ev| ev.state_key().to_string())
        .collect();
    Space {
        id: room.room_id().to_string(),
        name: display_name(room).await,
        children,
    }
}
