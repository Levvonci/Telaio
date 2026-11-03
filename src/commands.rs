//! Runs UI commands in background tasks.
//!
//! Every outcome goes back to the main loop as an `AppEvent`: no task writes
//! to the terminal, errors end up in the error bar.

use matrix_sdk::Client;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, Semaphore};

use crate::app::event::{AppEvent, Command, Joined, VerifyStage};
use crate::matrix::session::{self, Restore};
use crate::matrix::{actions, history, search, short_error, verify, Tx};

/// Session change, carried out by the main loop
pub enum SessionChange {
    Switch { client: Client, username: String },
    LoggedOut,
}

/// Parallel history loads
const HISTORY_CONCURRENCY: usize = 4;

pub struct Executor {
    client: Client,
    tx: Tx,
    session_tx: UnboundedSender<SessionChange>,
    /// Current account (data folder name)
    username: String,
    history: Arc<Semaphore>,
    history_limit: u64,
    max_upload_mb: u64,
    download_dir: PathBuf,
    open_command: Vec<String>,
}

impl Executor {
    pub fn new(client: Client, username: String, tx: Tx, session_tx: UnboundedSender<SessionChange>, ui: &crate::config::UiConfig) -> Self {
        Self {
            client,
            tx,
            session_tx,
            username,
            history: Arc::new(Semaphore::new(HISTORY_CONCURRENCY)),
            history_limit: ui.behavior.history_limit,
            max_upload_mb: ui.behavior.max_upload_mb,
            download_dir: ui.behavior.download_dir(),
            open_command: ui.behavior.open_command(),
        }
    }

    /// Spawns a task producing at most one event; an error becomes
    /// `AppEvent::Error` prefixed by `context`
    fn spawn<F>(&self, context: &'static str, fut: impl FnOnce(Client) -> F + Send + 'static)
    where
        F: Future<Output = anyhow::Result<Option<AppEvent>>> + Send + 'static,
    {
        let (client, tx) = (self.client.clone(), self.tx.clone());
        tokio::spawn(async move {
            let event = match fut(client).await {
                Ok(Some(ev)) => ev,
                Ok(None) => return,
                Err(e) => AppEvent::Error(format!("{context}: {}", format_error(&e))),
            };
            let _ = tx.send(event);
        });
    }

    pub fn run(&self, cmd: Command) {
        match cmd {
            Command::SendText { room_id, body } => self.spawn("Message not sent", move |c| async move {
                actions::send_text(&c, &room_id, &body).await?;
                Ok(None)
            }),
            Command::SendFile { room_id, path } => {
                let (max, tx) = (self.max_upload_mb, self.tx.clone());
                self.spawn("Attachment not sent", move |c| async move {
                    let res = actions::send_file(&c, &room_id, &path, max).await;
                    let _ = tx.send(AppEvent::UploadFinished { ok: res.is_ok() });
                    res.map(|_| None)
                })
            }
            Command::Download { attachment } => {
                let dir = self.download_dir.clone();
                self.spawn("Download failed", move |c| async move {
                    let path = actions::download(&c, &attachment, &dir, false).await?;
                    Ok(Some(AppEvent::Info(format!("Saved to {}", path.display()))))
                })
            }
            Command::Open { attachment } => {
                let (dir, open) = (self.download_dir.clone(), self.open_command.clone());
                self.spawn("Cannot open", move |c| async move {
                    let path = actions::download(&c, &attachment, &dir, true).await?;
                    actions::open_file(&open, &path)?;
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    Ok(Some(AppEvent::Info(format!("Opened {name}"))))
                })
            }
            Command::Join { address } => self.spawn("Join failed", move |c| async move {
                let joined = actions::join(&c, &address).await?;
                Ok(Some(AppEvent::Joined { joined, space_id: None }))
            }),
            Command::Leave { room_id } => self.spawn("Leave failed", move |c| async move {
                actions::leave(&c, &room_id).await?;
                Ok(Some(AppEvent::Left { room_id }))
            }),
            Command::CreateSpace { name } => self.spawn("Create space", move |c| async move {
                let space = actions::create_space(&c, &name).await?;
                Ok(Some(AppEvent::Joined { joined: Joined::Space(space), space_id: None }))
            }),
            Command::CreateRoom { name, space_id } => {
                let tx = self.tx.clone();
                self.spawn("Create room", move |c| async move {
                    let room = actions::create_room(&c, &name).await?;
                    let space_id = match space_id {
                        Some(sid) => add_to_space(&c, &tx, &sid, &room.id, "Room created but not added to the space").await,
                        None => None,
                    };
                    Ok(Some(AppEvent::Joined { joined: Joined::Room(room), space_id }))
                })
            }
            Command::OpenDm { user_id } => self.spawn("Create chat", move |c| async move {
                let room = actions::create_dm(&c, &user_id).await?;
                Ok(Some(AppEvent::Joined { joined: Joined::Room(room), space_id: None }))
            }),
            Command::Invite { room_id, user_id } => self.spawn("Invite", move |c| async move {
                actions::invite(&c, &room_id, &user_id).await?;
                Ok(Some(AppEvent::Info(format!("Invite sent to {}", user_id.trim()))))
            }),
            Command::SetSpaceChild { space_id, room_id, add } => self.spawn("Edit space", move |c| async move {
                actions::set_space_child(&c, &space_id, &room_id, add).await?;
                Ok(Some(AppEvent::SpaceChild { space_id, room_id, added: add }))
            }),
            Command::Moderate { room_id, action, target, reason } => self.spawn("Moderation", move |c| async move {
                let uid = actions::moderate(&c, &room_id, action, &target, reason.as_deref()).await?;
                Ok(Some(AppEvent::Info(format!("{uid} {}", action.done()))))
            }),
            Command::SetTopic { room_id, topic } => self.spawn("Topic", move |c| async move {
                actions::set_topic(&c, &room_id, &topic).await?;
                Ok(Some(AppEvent::Info("Topic updated".into())))
            }),
            Command::SetName { room_id, name } => self.spawn("Room name", move |c| async move {
                actions::set_name(&c, &room_id, &name).await?;
                Ok(Some(AppEvent::Info(format!("Room renamed to {name}"))))
            }),
            Command::Redact { room_id, event_id } => self.spawn("Delete", move |c| async move {
                actions::redact(&c, &room_id, &event_id).await?;
                Ok(Some(AppEvent::Redacted { room_id, event_id }))
            }),
            Command::LoadHistory { room_id, from } => {
                let (sem, limit, tx) = (self.history.clone(), self.history_limit, self.tx.clone());
                self.spawn("History", move |c| async move {
                    let _permit = sem.acquire().await;
                    let event = match history::load(&c, &room_id, from.as_deref(), limit).await {
                        Ok((messages, prev_token)) => AppEvent::History { room_id, messages, prev_token },
                        Err(e) => AppEvent::HistoryFailed { room_id, error: format_error(&e) },
                    };
                    let _ = tx.send(event);
                    Ok(None)
                })
            }
            Command::LoadHierarchy { space_id } => self.spawn("Space contents", move |c| async move {
                let entries = search::hierarchy(&c, &space_id).await?;
                Ok(Some(AppEvent::Hierarchy { space_id, entries }))
            }),
            // read receipts are not critical: errors ignored
            Command::MarkRead { room_id, event_id } => self.spawn("", move |c| async move {
                let _ = actions::mark_read(&c, &room_id, &event_id).await;
                Ok(None)
            }),
            Command::SearchUsers { query } => self.spawn("User search", move |c| async move {
                let items = search::users(&c, &query).await?;
                Ok(Some(AppEvent::SearchResults { query, items }))
            }),
            Command::SearchRooms { query } => self.spawn("Room search", move |c| async move {
                let items = search::rooms(&c, &query).await?;
                Ok(Some(AppEvent::SearchResults { query, items }))
            }),
            // an error closes the request popup showing the reason
            Command::StartVerification => {
                let (client, tx) = (self.client.clone(), self.tx.clone());
                tokio::spawn(async move {
                    if let Err(e) = verify::start(&client, &tx).await {
                        let user_id = client.user_id().map(|u| u.to_string()).unwrap_or_default();
                        let stage = VerifyStage::Cancelled(format_error(&e));
                        let _ = tx.send(AppEvent::Verification { user_id, flow_id: String::new(), stage });
                    }
                });
            }
            Command::VerifyAccept { user_id, flow_id } => self.spawn("Verification", move |c| async move {
                verify::accept(&c, &user_id, &flow_id).await?;
                Ok(None)
            }),
            Command::VerifyConfirm { user_id, flow_id } => self.spawn("Verification", move |c| async move {
                verify::confirm(&c, &user_id, &flow_id, true).await?;
                Ok(None)
            }),
            Command::VerifyMismatch { user_id, flow_id } => self.spawn("Verification", move |c| async move {
                verify::confirm(&c, &user_id, &flow_id, false).await?;
                Ok(None)
            }),
            // cancelling is not critical: the request expires anyway
            Command::VerifyCancel { user_id, flow_id } => self.spawn("", move |c| async move {
                let _ = verify::cancel(&c, &user_id, &flow_id).await;
                Ok(None)
            }),
            Command::Recover { key } => self.spawn("Recovery key", move |c| async move {
                verify::recover(&c, &key).await?;
                Ok(Some(AppEvent::Recovered))
            }),
            Command::Redecrypt { room_id, events } => self.spawn("Decryption", move |c| async move {
                let messages = verify::redecrypt(&c, &room_id, &events).await?;
                Ok((!messages.is_empty()).then_some(AppEvent::Decrypted { room_id, messages }))
            }),
            Command::SwitchAccount { username } => {
                let session_tx = self.session_tx.clone();
                self.spawn("Switch account", move |_| async move {
                    match session::restore(&username).await? {
                        Restore::Ready(client) => {
                            let _ = session_tx.send(SessionChange::Switch { client, username });
                            Ok(None)
                        }
                        Restore::NeedLogin(note) => {
                            let homeserver = known_homeserver(&username);
                            Ok(Some(AppEvent::NeedLogin { homeserver, username, note: note.map(str::to_owned) }))
                        }
                    }
                })
            }
            Command::Login { homeserver, username, password } => {
                let (session_tx, tx) = (self.session_tx.clone(), self.tx.clone());
                tokio::spawn(async move {
                    match session::login(&homeserver, &username, &password).await {
                        Ok(client) => { let _ = session_tx.send(SessionChange::Switch { client, username: username.trim().to_owned() }); }
                        Err(e) => { let _ = tx.send(AppEvent::LoginFailed(format_error(&e))); }
                    }
                });
            }
            Command::Logout => {
                let (session_tx, username) = (self.session_tx.clone(), self.username.clone());
                self.spawn("Logout", move |c| async move {
                    let res = session::logout(c, &username).await;
                    // local data is removed anyway: move on to another account
                    let _ = session_tx.send(SessionChange::LoggedOut);
                    res.map(|_| None)
                })
            }
        }
    }
}

/// Adds the room to the space; on error reports it and returns None
async fn add_to_space(c: &Client, tx: &Tx, space_id: &str, room_id: &str, context: &str) -> Option<String> {
    match actions::set_space_child(c, space_id, room_id, true).await {
        Ok(()) => Some(space_id.to_string()),
        Err(e) => {
            let _ = tx.send(AppEvent::Error(format!("{context}: {}", format_error(&e))));
            None
        }
    }
}

/// Homeserver of a known account (to prefill the login form)
fn known_homeserver(username: &str) -> String {
    let configured = crate::config::load_accounts().unwrap_or_default();
    session::known_accounts(&configured).into_iter()
        .find(|a| a.username == username)
        .map(|a| a.homeserver)
        .unwrap_or_default()
}

/// Context + root cause, on one line
pub fn format_error(e: &anyhow::Error) -> String {
    let top = e.to_string();
    match e.chain().nth(1) {
        Some(cause) => format!("{top} ({})", short_error(cause)),
        None => top,
    }
}
