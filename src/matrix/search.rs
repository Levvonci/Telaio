//! Server searches: user directory, public room directory, space hierarchy.

use anyhow::{Context, Result};
use matrix_sdk::{
    ruma::{
        api::client::{
            directory::get_public_rooms_filtered::v3::Request as PublicRoomsRequest,
            space::get_hierarchy::v1::Request as HierarchyRequest,
        },
        assign,
        directory::Filter,
        room::RoomType,
        RoomId, UInt,
    },
    Client,
};

use crate::app::model::HierarchyEntry;
use crate::app::popup::{PickAction, PickItem};

const RESULTS: u32 = 20;

/// Homeserver user directory (display name or MXID)
pub async fn users(client: &Client, query: &str) -> Result<Vec<PickItem>> {
    let resp = client.search_users(query, RESULTS.into()).await.context("User search failed")?;
    Ok(resp
        .results
        .into_iter()
        .filter(|u| Some(u.user_id.as_ref()) != client.user_id())
        .map(|u| PickItem {
            title: u.display_name.clone().unwrap_or_else(|| u.user_id.localpart().to_string()),
            subtitle: u.user_id.to_string(),
            tag: String::new(),
            action: PickAction::Dm(u.user_id.to_string()),
        })
        .collect())
}

/// Homeserver public directory of rooms and spaces
pub async fn rooms(client: &Client, query: &str) -> Result<Vec<PickItem>> {
    let request = assign!(PublicRoomsRequest::new(), {
        limit: Some(UInt::from(RESULTS)),
        filter: assign!(Filter::new(), { generic_search_term: Some(query.to_owned()) }),
    });
    let resp = client.public_rooms_filtered(request).await.context("Directory search failed")?;
    Ok(resp
        .chunk
        .into_iter()
        .map(|r| {
            let address = r.canonical_alias.as_ref().map(|a| a.to_string()).unwrap_or_else(|| r.room_id.to_string());
            let is_space = r.room_type == Some(RoomType::Space);
            PickItem {
                title: r.name.clone().unwrap_or_else(|| address.clone()),
                subtitle: address.clone(),
                tag: if is_space { "space".into() } else { format!("{} members", r.num_joined_members) },
                action: PickAction::Join(address),
            }
        })
        .collect())
}

/// Rooms and sub-spaces of a space (first level), joined or not
pub async fn hierarchy(client: &Client, space_id: &str) -> Result<Vec<HierarchyEntry>> {
    let rid = RoomId::parse(space_id).context("Invalid space ID")?;
    let request = assign!(HierarchyRequest::new(rid), {
        max_depth: Some(UInt::from(1u32)),
        limit: Some(UInt::from(100u32)),
    });
    let resp = client.send(request).await.context("Cannot read the space")?;
    Ok(resp
        .rooms
        .into_iter()
        .map(|chunk| {
            let s = chunk.summary;
            HierarchyEntry {
                id: s.room_id.to_string(),
                name: s.name.clone()
                    .or_else(|| s.canonical_alias.as_ref().map(|a| a.to_string()))
                    .unwrap_or_else(|| s.room_id.to_string()),
                alias: s.canonical_alias.map(|a| a.to_string()),
                topic: s.topic,
                members: s.num_joined_members.into(),
                is_space: s.room_type == Some(RoomType::Space),
            }
        })
        .collect())
}
