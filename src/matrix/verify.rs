//! Session verification: recovery key, interactive emoji verification (SAS)
//! and retried decryption when room keys arrive.
//!
//! Each request is driven by one task (`drive_request`) that follows the SDK
//! states and turns them into `AppEvent::Verification`; user choices arrive
//! as separate commands that look the verification up by (user, flow_id),
//! with no shared state.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use matrix_sdk::{
    encryption::{
        recovery::{RecoveryError, RecoveryState},
        secret_storage::SecretStorageError,
        verification::{CancelInfo, SasState, SasVerification, Verification, VerificationRequest, VerificationRequestState},
        VerificationState,
    },
    ruma::{
        events::{
            key::verification::{cancel::CancelCode, request::ToDeviceKeyVerificationRequestEvent, VerificationMethod},
            room::encrypted::OriginalSyncRoomEncryptedEvent,
        },
        serde::Raw,
        UserId,
    },
    Client,
};

use super::{convert, get_room, Tx};
use crate::app::event::{AppEvent, RecoveryStatus, Security, VerifyStage};
use crate::app::model::{Content, Message};

// ── Session state ──────────────────────────────────────────────────────────

/// Follows device verification and key backup state
pub async fn watch_security(client: Client, tx: Tx) {
    let encryption = client.encryption();
    let mut verified = encryption.verification_state();
    let mut recovery = encryption.recovery().state_stream();
    let mut current = Security::default();
    loop {
        tokio::select! {
            Some(v) = verified.next() => current.verified = match v {
                VerificationState::Verified => Some(true),
                VerificationState::Unverified => Some(false),
                VerificationState::Unknown => None,
            },
            Some(r) = recovery.next() => current.recovery = match r {
                RecoveryState::Enabled => RecoveryStatus::Enabled,
                RecoveryState::Incomplete => RecoveryStatus::Incomplete,
                RecoveryState::Disabled => RecoveryStatus::Disabled,
                RecoveryState::Unknown => RecoveryStatus::Unknown,
            },
            else => return,
        }
        if tx.send(AppEvent::Security(current.clone())).is_err() { return; }
    }
}

/// Reports newly received room keys (from backup, verification or other
/// devices) so messages still encrypted get retried
pub async fn watch_room_keys(client: Client, tx: Tx) {
    let Some(mut stream) = client.encryption().room_keys_received_stream().await else { return };
    while let Some(batch) = stream.next().await {
        let keys = batch.ok().map(|keys| keys.into_iter().map(|k| (k.room_id.to_string(), k.session_id)).collect());
        if tx.send(AppEvent::RoomKeys(keys)).is_err() { return; }
    }
}

/// Retries decrypting stored events. Returns only the successful ones.
pub async fn redecrypt(client: &Client, room_id: &str, events: &[String]) -> Result<Vec<(String, Option<Message>)>> {
    let room = get_room(client, room_id)?;
    let mut out = Vec::new();
    for json in events {
        let Ok(raw) = Raw::<OriginalSyncRoomEncryptedEvent>::from_json_string(json.clone()) else { continue };
        let Ok(Some(event_id)) = raw.get_field::<String>("event_id") else { continue };
        // if the key is still missing, the SDK asks the backup in the background
        let Ok(ev) = room.decrypt_event(&raw, None).await else { continue };
        let message = convert::from_timeline_event(&room, ev.raw()).await;
        let still_encrypted = message.as_ref().is_some_and(|m| matches!(m.content, Content::Encrypted(_)));
        if !still_encrypted {
            out.push((event_id, message));
        }
    }
    Ok(out)
}

// ── Recovery key ───────────────────────────────────────────────────────────

/// Imports cross-signing and backup keys from server-side secret storage:
/// the device becomes verified and old messages can be decrypted
pub async fn recover(client: &Client, key: &str) -> Result<()> {
    let recovery = client.encryption().recovery();
    if recovery.state() == RecoveryState::Disabled {
        bail!("The account has no key backup: set one up in Element (Settings → Security)");
    }
    match recovery.recover(key.trim()).await {
        Ok(()) => Ok(()),
        Err(RecoveryError::SecretStorage(SecretStorageError::SecretStorageKey(_))) => bail!("Wrong recovery key"),
        Err(RecoveryError::SecretStorage(SecretStorageError::MissingKeyInfo { .. })) => {
            bail!("The account has no secret storage: set it up in Element")
        }
        Err(e) => Err(anyhow::Error::new(e)).context("Recovery failed"),
    }
}

// ── Emoji verification ─────────────────────────────────────────────────────

pub fn register_handlers(client: &Client, tx: &Tx) {
    let tx = tx.clone();
    client.add_event_handler(move |ev: ToDeviceKeyVerificationRequestEvent, client: Client| {
        let tx = tx.clone();
        async move {
            let request = client.encryption().get_verification_request(&ev.sender, &ev.content.transaction_id).await;
            let Some(request) = request else { return };
            send(&tx, &request, VerifyStage::Incoming { device: ev.content.from_device.to_string() });
            tokio::spawn(drive_request(request, tx));
        }
    });
}

/// Asks the account's other devices to verify this session
pub async fn start(client: &Client, tx: &Tx) -> Result<()> {
    let encryption = client.encryption();
    let user_id = client.user_id().context("Unknown user")?;
    let identity = match encryption.get_user_identity(user_id).await? {
        Some(identity) => Some(identity),
        None => encryption.request_user_identity(user_id).await?,
    };
    let Some(identity) = identity else {
        bail!("The account has no cross-signing keys yet: set them up in Element (Settings → Security)");
    };
    if !encryption.has_devices_to_verify_against().await? {
        bail!("No other verified device: use the recovery key");
    }
    let request = identity
        .request_verification_with_methods(vec![VerificationMethod::SasV1])
        .await
        .context("Verification request not sent")?;
    send(tx, &request, VerifyStage::Waiting("Accept the request on another device (e.g. Element)".into()));
    tokio::spawn(drive_request(request, tx.clone()));
    Ok(())
}

pub async fn accept(client: &Client, user_id: &str, flow_id: &str) -> Result<()> {
    let user_id = UserId::parse(user_id)?;
    let request = client.encryption().get_verification_request(&user_id, flow_id).await
        .context("The request is no longer valid")?;
    request.accept_with_methods(vec![VerificationMethod::SasV1]).await?;
    Ok(())
}

pub async fn confirm(client: &Client, user_id: &str, flow_id: &str, matches: bool) -> Result<()> {
    let sas = find_sas(client, user_id, flow_id).await?;
    if matches { sas.confirm().await? } else { sas.mismatch().await? }
    Ok(())
}

pub async fn cancel(client: &Client, user_id: &str, flow_id: &str) -> Result<()> {
    let user_id = UserId::parse(user_id)?;
    let encryption = client.encryption();
    if let Some(Verification::SasV1(sas)) = encryption.get_verification(&user_id, flow_id).await {
        sas.cancel().await?;
    } else if let Some(request) = encryption.get_verification_request(&user_id, flow_id).await {
        request.cancel().await?;
    }
    Ok(())
}

async fn find_sas(client: &Client, user_id: &str, flow_id: &str) -> Result<SasVerification> {
    let user_id = UserId::parse(user_id)?;
    match client.encryption().get_verification(&user_id, flow_id).await {
        Some(Verification::SasV1(sas)) => Ok(sas),
        _ => bail!("The verification is no longer active"),
    }
}

fn send(tx: &Tx, request: &VerificationRequest, stage: VerifyStage) {
    let _ = tx.send(AppEvent::Verification {
        user_id: request.other_user_id().to_string(),
        flow_id: request.flow_id().to_string(),
        stage,
    });
}

/// Follows a request to the end: starts or accepts the emoji comparison as
/// soon as both sides are ready
async fn drive_request(request: VerificationRequest, tx: Tx) {
    let mut changes = request.changes();
    let mut state = request.state();
    let mut sas_started = false;
    loop {
        match state {
            // whoever requested the verification starts the comparison
            VerificationRequestState::Ready { .. } if request.we_started() => {
                if !sas_started {
                    sas_started = true;
                    if let Err(e) = request.start_sas().await {
                        send(&tx, &request, VerifyStage::Cancelled(format!("could not start ({e})")));
                        return;
                    }
                }
            }
            VerificationRequestState::Ready { .. } => {
                send(&tx, &request, VerifyStage::Waiting("Choose emoji verification on the other device".into()));
            }
            VerificationRequestState::Transitioned { verification, .. } => {
                match verification {
                    Verification::SasV1(sas) => drive_sas(sas, &request, &tx).await,
                    #[allow(unreachable_patterns)]
                    _ => send(&tx, &request, VerifyStage::Cancelled("unsupported method: use emojis".into())),
                }
                return;
            }
            VerificationRequestState::Done => return send(&tx, &request, VerifyStage::Done),
            VerificationRequestState::Cancelled(info) => return send(&tx, &request, VerifyStage::Cancelled(cancel_reason(&info))),
            VerificationRequestState::Created { .. } | VerificationRequestState::Requested { .. } => {}
        }
        match changes.next().await {
            Some(s) => state = s,
            None => return,
        }
    }
}

async fn drive_sas(sas: SasVerification, request: &VerificationRequest, tx: &Tx) {
    let mut changes = sas.changes();
    if !sas.we_started() {
        if let Err(e) = sas.accept().await {
            return send(tx, request, VerifyStage::Cancelled(format!("could not accept ({e})")));
        }
    }
    let mut state = sas.state();
    loop {
        match state {
            SasState::KeysExchanged { emojis, decimals } => {
                let list = match emojis {
                    Some(e) => e.emojis.iter()
                        .map(|emoji| (emoji.symbol.to_string(), emoji.description.to_string()))
                        .collect(),
                    // the other device doesn't support emojis: three numbers
                    None => [decimals.0, decimals.1, decimals.2].iter().map(|d| (d.to_string(), String::new())).collect(),
                };
                send(tx, request, VerifyStage::Emojis(list));
            }
            SasState::Confirmed => send(tx, request, VerifyStage::Waiting("Waiting for confirmation on the other device".into())),
            SasState::Done { .. } => return send(tx, request, VerifyStage::Done),
            SasState::Cancelled(info) => return send(tx, request, VerifyStage::Cancelled(cancel_reason(&info))),
            SasState::Created { .. } | SasState::Started { .. } | SasState::Accepted { .. } => {}
        }
        match changes.next().await {
            Some(s) => state = s,
            None => return,
        }
    }
}

fn cancel_reason(info: &CancelInfo) -> String {
    let reason = match info.cancel_code() {
        CancelCode::User => "cancelled",
        CancelCode::Timeout => "timed out",
        CancelCode::MismatchedSas => "the emojis don't match",
        CancelCode::Accepted => "accepted on another device",
        CancelCode::UnknownMethod | CancelCode::UnknownTransaction => "unsupported method",
        CancelCode::KeyMismatch | CancelCode::MismatchedCommitment => "the keys don't match",
        _ => return info.reason().to_string(),
    };
    if info.cancelled_by_us() { format!("{reason} (by this device)") } else { reason.to_string() }
}
