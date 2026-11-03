//! Login with persistent sessions and multiple accounts.
//!
//! After the first login, token and device are saved in `session.json` and
//! the client state in a SQLite store, so later starts need no password and
//! sync resumes where it stopped. Each account has its own data folder; known
//! accounts are those in account.toml plus those with a saved session.

use anyhow::{bail, Context, Result};
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    ruma::api::error::ErrorKind,
    Client,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use crate::config::{self, AccountConfig};

/// Bump when the store format changes: older stores are discarded (and the
/// old device logged out)
const STORE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct StoredSession {
    homeserver: String,
    username: String,
    #[serde(default)]
    store_version: u32,
    session: MatrixSession,
}

pub enum Restore {
    Ready(Client),
    /// A new login is needed; optional explanation to show
    NeedLogin(Option<&'static str>),
}

/// Reopens an account's saved session, without a password
pub async fn restore(username: &str) -> Result<Restore> {
    let data_dir = config::data_dir_for(username);
    let Some(stored) = read_session(&data_dir.join("session.json")) else {
        return Ok(Restore::NeedLogin(None));
    };
    if stored.store_version != STORE_VERSION {
        logout_old(&stored.homeserver, stored.session).await;
        discard(username);
        return Ok(Restore::NeedLogin(Some("The local store was upgraded: please sign in again.")));
    }

    let client = build_client(&stored.homeserver, &data_dir.join("store"), false).await?;
    client.restore_session(stored.session).await.context("Cannot restore the session")?;
    match client.whoami().await {
        Err(e) if matches!(e.client_api_error_kind(), Some(ErrorKind::UnknownToken(_))) => {
            drop(client);
            discard(username);
            Ok(Restore::NeedLogin(Some("Session expired: please sign in again.")))
        }
        // continue even offline: the sync will retry
        _ => Ok(Restore::Ready(client)),
    }
}

/// Password login. `homeserver` may be a URL or a server name; if empty it
/// is derived from the MXID (`@user:server`).
pub async fn login(homeserver: &str, username: &str, password: &str) -> Result<Client> {
    let res = try_login(homeserver, username.trim(), password).await;
    if res.is_err() {
        // no empty folders for accounts that never signed in
        discard(username.trim());
        let _ = std::fs::remove_dir(config::data_dir_for(username.trim()));
    }
    res
}

async fn try_login(homeserver: &str, username: &str, password: &str) -> Result<Client> {
    let homeserver = match homeserver.trim() {
        "" => match username.split_once(':') {
            Some((_, server)) if username.starts_with('@') && !server.is_empty() => server.to_owned(),
            _ => bail!("Enter the server or a full user ID (@user:server)"),
        },
        hs => hs.to_owned(),
    };

    // a previous session's store is no longer valid
    discard(username);
    let data_dir = config::data_dir_for(username);
    let client = build_client(&homeserver, &data_dir.join("store"), true).await
        .with_context(|| format!("Server {homeserver} unreachable"))?;
    let login = client
        .matrix_auth()
        .login_username(username, password)
        .initial_device_display_name(config::APP_NAME)
        .send()
        .await;
    if let Err(e) = login {
        if matches!(e.client_api_error_kind(), Some(ErrorKind::Forbidden)) {
            bail!("Login failed: wrong username or password");
        }
        return Err(anyhow::Error::new(e)).with_context(|| format!("Cannot sign in via {homeserver}"));
    }

    if let Some(session) = client.matrix_auth().session() {
        let stored = StoredSession {
            // resolved URL: later restores don't depend on discovery
            homeserver: client.homeserver().to_string(),
            username: username.to_owned(),
            store_version: STORE_VERSION,
            session,
        };
        config::write_private(&data_dir.join("session.json"), &serde_json::to_string_pretty(&stored)?)
            .context("Cannot save the session")?;
    }
    Ok(client)
}

/// Logs the device out and deletes local data
pub async fn logout(client: Client, username: &str) -> Result<()> {
    let res = client.matrix_auth().logout().await;
    drop(client);
    discard(username);
    res.context("Server logout failed (local data was deleted)")?;
    Ok(())
}

/// Configured accounts plus those with a saved session, without duplicates
pub fn known_accounts(configured: &[AccountConfig]) -> Vec<AccountConfig> {
    let mut out: Vec<AccountConfig> = configured.to_vec();
    let Ok(dirs) = std::fs::read_dir(config::data_root()) else { return out };
    let mut saved: Vec<AccountConfig> = dirs
        .flatten()
        .filter_map(|d| read_session(&d.path().join("session.json")))
        .map(|s| AccountConfig { homeserver: s.homeserver, username: s.username, password: None })
        .collect();
    saved.sort_by(|a, b| a.username.cmp(&b.username));
    for acc in saved {
        if !out.iter().any(|a| a.username == acc.username) { out.push(acc); }
    }
    out
}

pub fn has_session(username: &str) -> bool {
    read_session(&config::data_dir_for(username).join("session.json")).is_some()
}

/// Gives the device the current app name (after the rename). Best effort.
pub async fn rename_device(client: &Client) {
    if let Some(device_id) = client.device_id().map(|d| d.to_owned()) {
        let _ = client.rename_device(&device_id, config::APP_NAME).await;
    }
}

/// `discover`: `homeserver` may be a server name to resolve (.well-known);
/// otherwise no network request is made while building
async fn build_client(homeserver: &str, store_path: &Path, discover: bool) -> Result<Client> {
    std::fs::create_dir_all(store_path)?;
    let builder = Client::builder();
    let builder = if discover { builder.server_name_or_homeserver_url(homeserver) } else { builder.homeserver_url(homeserver) };
    builder
        .sqlite_store(store_path, None)
        .with_encryption_settings(EncryptionSettings {
            // missing keys are fetched from the backup when a message fails to
            // decrypt (requires the recovery key or a verified session)
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            ..Default::default()
        })
        .build()
        .await
        .context("Cannot create the client")
}

/// Deletes an account's session and store
fn discard(username: &str) {
    let dir = config::data_dir_for(username);
    let _ = std::fs::remove_file(dir.join("session.json"));
    let _ = std::fs::remove_dir_all(dir.join("store"));
}

/// Logs out the device of an obsolete session so it doesn't linger in the
/// account's device list. Best effort.
async fn logout_old(homeserver: &str, session: MatrixSession) {
    let attempt = async {
        let client = Client::builder().homeserver_url(homeserver).build().await.ok()?;
        client.restore_session(session).await.ok()?;
        client.matrix_auth().logout().await.ok()
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), attempt).await;
}

fn read_session(path: &Path) -> Option<StoredSession> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}
