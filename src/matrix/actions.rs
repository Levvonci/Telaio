//! Room actions: messages, attachments, join/leave, spaces, moderation.

use anyhow::{bail, Context, Result};
use matrix_sdk::{
    attachment::AttachmentConfig,
    media::{MediaFormat, MediaRequestParameters},
    ruma::{
        api::client::{receipt::create_receipt::v3::ReceiptType, room::create_room::v3::Request as CreateRoomRequest},
        assign,
        events::{
            receipt::ReceiptThread, room::message::RoomMessageEventContent, space::child::SpaceChildEventContent,
        },
        room::RoomType,
        serde::Raw,
        EventId, OwnedUserId, RoomId,
    },
    Client, Room, RoomMemberships,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{address::{parse_room_address, parse_user_id}, get_room, human_size, sync};
use crate::app::event::{Joined, ModAction};
use crate::app::model::{Attachment, Room as AppRoom, Space};

/// Sends a text message; Markdown is converted to HTML
pub async fn send_text(client: &Client, room_id: &str, body: &str) -> Result<()> {
    get_room(client, room_id)?
        .send(RoomMessageEventContent::text_markdown(body))
        .await
        .context("Cannot send the message")?;
    Ok(())
}

/// Uploads a file and sends it as an attachment (image, video, audio or
/// generic file by MIME type). Encrypted in encrypted rooms.
pub async fn send_file(client: &Client, room_id: &str, path: &Path, max_mb: u64) -> Result<()> {
    let room = get_room(client, room_id)?;
    let meta = tokio::fs::metadata(path).await.with_context(|| format!("File not found: {}", path.display()))?;
    if !meta.is_file() {
        bail!("{} is not a file", path.display());
    }
    if meta.len() > max_mb * 1024 * 1024 {
        bail!("File too large ({}, limit {max_mb} MB)", human_size(meta.len()));
    }
    let data = tokio::fs::read(path).await.with_context(|| format!("Cannot read {}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let mime = mime_guess::from_path(path).first_or_octet_stream();

    room.send_attachment(name.clone(), &mime, data, AttachmentConfig::new())
        .await
        .with_context(|| format!("Cannot send {name}"))?;
    Ok(())
}

/// Downloads an attachment into `dir` without overwriting existing files.
/// With `reuse`, a file with the same name and size counts as already
/// downloaded (reopening an attachment doesn't duplicate it).
pub async fn download(client: &Client, att: &Attachment, dir: &Path, reuse: bool) -> Result<PathBuf> {
    let name = sanitize_filename(&att.name);
    if reuse {
        let existing = dir.join(&name);
        if let Ok(meta) = tokio::fs::metadata(&existing).await {
            if meta.is_file() && att.size.is_none_or(|s| s == meta.len()) {
                return Ok(existing);
            }
        }
    }
    let request = MediaRequestParameters { source: att.source.clone(), format: MediaFormat::File };
    let data = client.media().get_media_content(&request, true).await.context("Download failed")?;
    tokio::fs::create_dir_all(dir).await.with_context(|| format!("Cannot create {}", dir.display()))?;
    let path = unique_path(dir, &name);
    tokio::fs::write(&path, data).await.with_context(|| format!("Cannot write {}", path.display()))?;
    Ok(path)
}

/// Opens a file with the given program (usually xdg-open), detached from the
/// terminal so no output reaches the UI
pub fn open_file(command: &[String], path: &Path) -> Result<()> {
    let (program, args) = command.split_first().context("No program configured to open files")?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // own process group: the program outlives telaio
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().with_context(|| format!("Cannot start {program}"))?;
    // reap the child (no zombies)
    tokio::spawn(async move { let _ = child.wait().await; });
    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim_start_matches('.').trim();
    if clean.is_empty() { "attachment".into() } else { clean.to_string() }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap()
}

pub async fn mark_read(client: &Client, room_id: &str, event_id: &str) -> Result<()> {
    let room = get_room(client, room_id)?;
    room.send_single_receipt(ReceiptType::Read, ReceiptThread::Unthreaded, EventId::parse(event_id)?).await?;
    Ok(())
}

/// Joins a room or space by address. Spaces are recognized from the preview
/// (if available) or from the room state.
pub async fn join(client: &Client, address: &str) -> Result<Joined> {
    let (id, via) = parse_room_address(address)?;
    let preview_space = client
        .get_room_preview(&id, via.clone())
        .await
        .ok()
        .map(|p| p.room_type == Some(RoomType::Space));
    let room = client
        .join_room_by_id_or_alias(&id, &via)
        .await
        .with_context(|| format!("Cannot join {id}"))?;

    if room.is_space() || preview_space == Some(true) {
        let mut space = sync::space_info(&room).await;
        if space.name.starts_with('!') { space.name = id.to_string(); }
        return Ok(Joined::Space(space));
    }
    let mut info = sync::room_info(&room).await.unwrap_or_else(|| new_room(room.room_id().as_str(), id.as_str(), false));
    if info.name.starts_with('!') { info.name = id.to_string(); }
    Ok(Joined::Room(info))
}

pub async fn leave(client: &Client, room_id: &str) -> Result<()> {
    get_room(client, room_id)?.leave().await.context("Cannot leave the room")?;
    Ok(())
}

pub async fn create_space(client: &Client, name: &str) -> Result<Space> {
    let creation = Raw::from_json(serde_json::value::to_raw_value(&serde_json::json!({ "type": "m.space" }))?);
    let request = assign!(CreateRoomRequest::new(), {
        name: Some(name.to_owned()),
        creation_content: Some(creation),
    });
    let room = client.create_room(request).await.context("Cannot create the space")?;
    Ok(Space { id: room.room_id().to_string(), name: name.to_owned(), children: HashSet::new() })
}

/// Creates a private end-to-end encrypted room (like Element). Spaces stay
/// unencrypted: they hold no messages, only the hierarchy.
pub async fn create_room(client: &Client, name: &str) -> Result<AppRoom> {
    let encryption = Raw::from_json(serde_json::value::to_raw_value(&serde_json::json!({
        "type": "m.room.encryption",
        "state_key": "",
        "content": { "algorithm": "m.megolm.v1.aes-sha2" },
    }))?);
    let request = assign!(CreateRoomRequest::new(), {
        name: Some(name.to_owned()),
        initial_state: vec![encryption],
    });
    let room = client.create_room(request).await.context("Cannot create the room")?;
    let mut info = new_room(room.room_id().as_str(), name, false);
    info.encrypted = true;
    Ok(info)
}

/// Creates a direct chat (registered in m.direct and encrypted)
pub async fn create_dm(client: &Client, user_id: &str) -> Result<AppRoom> {
    let uid: OwnedUserId = user_id.trim().parse().context("Invalid user ID (use @user:server)")?;
    let room = client.create_dm(&uid).await.context("Cannot create the chat")?;
    let mut info = new_room(room.room_id().as_str(), uid.localpart(), true);
    info.dm_with = vec![uid.to_string()];
    Ok(info)
}

fn new_room(id: &str, name: &str, is_direct: bool) -> AppRoom {
    AppRoom {
        id: id.to_owned(),
        name: name.to_owned(),
        topic: None,
        alias: None,
        unread: 0,
        highlights: 0,
        is_direct,
        dm_with: Vec::new(),
        members: 1,
        encrypted: is_direct,
        invite: None,
    }
}

pub async fn invite(client: &Client, room_id: &str, user_id: &str) -> Result<()> {
    let uid: OwnedUserId = user_id.trim().parse().context("Invalid user ID (use @user:server)")?;
    get_room(client, room_id)?.invite_user_by_id(&uid).await.context("Invite failed")?;
    Ok(())
}

/// Adds or removes a room from a space (m.space.child state event)
pub async fn set_space_child(client: &Client, space_id: &str, room_id: &str, add: bool) -> Result<()> {
    let space = get_room(client, space_id).context("Space not found")?;
    let rid = RoomId::parse(room_id).context("Invalid room ID")?;
    let result = if add {
        // "via" must hold the server name (e.g. matrix.org), not the API host
        let server = client.user_id().context("Not signed in")?.server_name().to_owned();
        space.send_state_event_for_key(&rid, SpaceChildEventContent::new(vec![server])).await.map(|_| ())
    } else {
        // empty content = room removed from the space
        space.send_state_event_raw("m.space.child", rid.as_str(), serde_json::json!({})).await.map(|_| ())
    };
    result.context("You don't have permission to edit this space")
}

// ── Moderation ─────────────────────────────────────────────────────────────

/// Kicks, bans or unbans a user. Power levels are checked beforehand to give
/// a clear message instead of a 403. Returns the target's MXID.
pub async fn moderate(client: &Client, room_id: &str, action: ModAction, target: &str, reason: Option<&str>) -> Result<String> {
    let room = get_room(client, room_id)?;
    let own = client.user_id().context("Not signed in")?.to_owned();
    let uid = resolve_member(&room, target, action).await?;
    if uid == own {
        bail!("you can't do this to yourself (use /leave to leave)");
    }

    let levels = room.power_levels().await.context("Room power levels unavailable")?;
    let allowed = match action {
        ModAction::Kick => levels.user_can_kick_user(&own, &uid),
        ModAction::Ban => levels.user_can_ban_user(&own, &uid),
        ModAction::Unban => levels.user_can_ban(&own),
    };
    if !allowed {
        bail!("you don't have permission in this room (you need a higher power level than {uid})");
    }

    let result = match action {
        ModAction::Kick => room.kick_user(&uid, reason).await,
        ModAction::Ban => room.ban_user(&uid, reason).await,
        ModAction::Unban => room.unban_user(&uid, reason).await,
    };
    result.with_context(|| format!("The server rejected the request for {uid}"))?;
    Ok(uid.to_string())
}

/// Finds a member by MXID, display name or localpart
async fn resolve_member(room: &Room, target: &str, action: ModAction) -> Result<OwnedUserId> {
    if let Some(uid) = parse_user_id(target) {
        return Ok(uid);
    }
    let memberships = match action {
        ModAction::Unban => RoomMemberships::BAN,
        _ => RoomMemberships::JOIN | RoomMemberships::INVITE,
    };
    let members = room.members(memberships).await.context("Member list unavailable")?;
    let wanted = target.trim().trim_start_matches('@').to_lowercase();
    let found: Vec<OwnedUserId> = members
        .iter()
        .filter(|m| {
            m.user_id().localpart().to_lowercase() == wanted
                || m.display_name().is_some_and(|n| n.to_lowercase() == wanted)
        })
        .map(|m| m.user_id().to_owned())
        .collect();
    match found.as_slice() {
        [] => bail!("no member matches “{target}”"),
        [uid] => Ok(uid.clone()),
        many => bail!(
            "“{target}” is ambiguous, use the MXID: {}",
            many.iter().map(|u| u.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

pub async fn set_topic(client: &Client, room_id: &str, topic: &str) -> Result<()> {
    get_room(client, room_id)?.set_room_topic(topic).await.context("You don't have permission to change the topic")?;
    Ok(())
}

pub async fn set_name(client: &Client, room_id: &str, name: &str) -> Result<()> {
    get_room(client, room_id)?.set_name(name.to_owned()).await.context("You don't have permission to rename the room")?;
    Ok(())
}

/// Redacts a message (own messages always; others' need permission)
pub async fn redact(client: &Client, room_id: &str, event_id: &str) -> Result<()> {
    let eid = EventId::parse(event_id)?;
    get_room(client, room_id)?.redact(&eid, None, None).await.context("You don't have permission to delete this message")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename(""), "attachment");
    }

    #[tokio::test]
    async fn open_file_runs_detached_program() {
        let dir = std::env::temp_dir().join(format!("telaio-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("opened");
        // "touch" instead of xdg-open: the file appears if the program ran
        open_file(&["touch".to_string()], &target).unwrap();
        for _ in 0..50 {
            if target.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(target.exists());
        assert!(open_file(&["program-that-does-not-exist".to_string()], &target).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
