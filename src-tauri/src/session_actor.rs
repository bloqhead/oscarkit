//! Owns a live `OscarSession` on a dedicated task and exposes it to Tauri
//! commands only through an `mpsc` channel of `SessionCommand`s. The reason
//! this is an actor rather than an `Arc<Mutex<OscarSession>>` shared
//! directly: `OscarSession::handle_next_frame` awaits a raw socket read that
//! may not resolve for a while, and `oscar_rs::FlapReader::read_frame` is
//! not cancellation-safe (dropping it mid-read loses bytes already pulled
//! off the socket, permanently desyncing the FLAP stream). So the read half
//! runs alone on its own task (nothing ever races it), forwarding whole
//! parsed frames over a channel to this actor, which only ever selects
//! between two cancel-safe `mpsc::Receiver::recv()` calls.

use tokio::sync::{mpsc, oneshot};
use tauri::{AppHandle, Emitter};

use oscar_rs::{Buddy, ChatInvite, FlapReader, IncomingIm, OscarSession};

pub enum SessionCommand {
    SendMessage { recipient: String, text: String, reply: oneshot::Sender<Result<(), String>> },
    AddBuddy { screen_name: String, group_name: String, reply: oneshot::Sender<Result<(), String>> },
    RemoveBuddy { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
    SetAwayMessage { text: Option<String>, reply: oneshot::Sender<Result<(), String>> },
    RequestUserInfo { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
    SendWarning { screen_name: String, anonymous: bool, reply: oneshot::Sender<Result<(), String>> },
    AddToBlockList { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
    RemoveFromBlockList { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
    /// Creates a room, joins it, spawns its `chat_actor`, then best-effort
    /// invites each requested screen name (a failed invite to one recipient
    /// doesn't fail the whole creation — logged and skipped). Replies with
    /// the new room's label so the caller can open its window.
    CreateChatRoom { room_name: String, invite_screen_names: Vec<String>, reply: oneshot::Sender<Result<String, String>> },
    /// Joins the room named by an already-received invite (by its index into
    /// `incoming_chat_invites`) and spawns its `chat_actor`. Declining an
    /// invite needs no command at all — it's a pure frontend-side dismissal.
    AcceptChatInvite { index: usize, reply: oneshot::Sender<Result<String, String>> },
}

/// A plain-data snapshot of the session state the frontend cares about —
/// the boundary type between `OscarSession` (protocol-crate internals) and
/// the UI (JSON over Tauri IPC/events).
#[derive(Clone, serde::Serialize)]
pub struct SessionSnapshot {
    pub screen_name: String,
    pub buddies: Vec<Buddy>,
    pub incoming_messages: Vec<IncomingIm>,
    pub away_message: Option<String>,
    pub incoming_chat_invites: Vec<ChatInvite>,
}

impl From<&OscarSession> for SessionSnapshot {
    fn from(session: &OscarSession) -> Self {
        SessionSnapshot {
            screen_name: session.screen_name.clone(),
            buddies: session.buddies.clone(),
            incoming_messages: session.incoming_messages.clone(),
            away_message: session.away_message.clone(),
            incoming_chat_invites: session.incoming_chat_invites.clone(),
        }
    }
}

pub(crate) enum FrameEvent {
    Frame(oscar_rs::FlapFrame),
    Closed,
    Error(std::io::Error),
}

/// Turns a room cookie (`"{exchange}-{instance}-{name}"`, e.g.
/// `"4-0-MyRoom"`) into a string safe to use as both a Tauri window label
/// and a `ChatRoomsState` key — the raw cookie embeds the room name
/// verbatim, which can contain spaces or other characters window labels
/// don't allow. Deterministic per cookie (not a hash) so the same room
/// always maps to the same label within a run, mirroring how
/// `normalizeScreenName` derives IM window labels from buddy names.
pub(crate) fn room_label_for(room_cookie: &str) -> String {
    let sanitized: String = room_cookie
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    format!("chat-{sanitized}")
}

pub(crate) async fn run_reader(mut reader: FlapReader, tx: mpsc::Sender<FrameEvent>) {
    loop {
        match reader.read_frame().await {
            Ok(Some(frame)) => {
                if tx.send(FrameEvent::Frame(frame)).await.is_err() {
                    break;
                }
            }
            Ok(None) => {
                let _ = tx.send(FrameEvent::Closed).await;
                break;
            }
            Err(e) => {
                let _ = tx.send(FrameEvent::Error(e)).await;
                break;
            }
        }
    }
}

/// Spawns the actor task (plus its dedicated frame-reader task) and returns
/// the command channel the Tauri command handlers send into.
pub fn spawn(app: AppHandle, mut session: OscarSession) -> mpsc::Sender<SessionCommand> {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<SessionCommand>(32);
    let (frame_tx, mut frame_rx) = mpsc::channel::<FrameEvent>(32);

    tauri::async_runtime::spawn(run_reader(session.split_reader(), frame_tx));

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(command) => handle_command(&app, &mut session, command, &mut frame_rx).await,
                        None => break, // frontend/app side dropped the sender — nothing left to serve
                    }
                }
                evt = frame_rx.recv() => {
                    match evt {
                        Some(FrameEvent::Frame(frame)) => {
                            if session.dispatch_frame(frame).await.is_err() {
                                let _ = app.emit("session-error", "connection error".to_string());
                                break;
                            }
                        }
                        Some(FrameEvent::Closed) => {
                            let _ = app.emit("session-error", "connection closed".to_string());
                            break;
                        }
                        Some(FrameEvent::Error(e)) => {
                            let _ = app.emit("session-error", e.to_string());
                            break;
                        }
                        None => break, // reader task gone
                    }
                }
            }
            let _ = app.emit("session-update", SessionSnapshot::from(&session));
        }
    });

    cmd_tx
}

/// Feeds `request_service_redirect` (called transitively by
/// `create_and_join_room`/`join_room`) frames straight off the actor's own
/// `frame_rx` — the reader task already forwards every BOS frame there, so
/// this is the "answer" `FrameSource`'s doc comment refers to for real Tauri
/// usage, once `split_reader()` has already handed the actual `FlapReader`
/// off to `run_reader`.
struct ActorFrameSource<'a> {
    rx: &'a mut mpsc::Receiver<FrameEvent>,
}

impl<'a> oscar_rs::FrameSource for ActorFrameSource<'a> {
    async fn next_frame(&mut self) -> Result<oscar_rs::FlapFrame, oscar_rs::OscarError> {
        match self.rx.recv().await {
            Some(FrameEvent::Frame(frame)) => Ok(frame),
            Some(FrameEvent::Closed) | None => Err(oscar_rs::OscarError::ConnectionClosed("bos session")),
            Some(FrameEvent::Error(e)) => Err(oscar_rs::OscarError::Io(e)),
        }
    }
}

async fn handle_command(app: &AppHandle, session: &mut OscarSession, command: SessionCommand, frame_rx: &mut mpsc::Receiver<FrameEvent>) {
    match command {
        SessionCommand::SendMessage { recipient, text, reply } => {
            let _ = reply.send(session.send_message(&recipient, &text).await.map_err(|e| e.to_string()));
        }
        SessionCommand::AddBuddy { screen_name, group_name, reply } => {
            let _ = reply.send(session.add_buddy(&screen_name, &group_name).await.map_err(|e| e.to_string()));
        }
        SessionCommand::RemoveBuddy { screen_name, reply } => {
            let _ = reply.send(session.remove_buddy(&screen_name).await.map_err(|e| e.to_string()));
        }
        SessionCommand::SetAwayMessage { text, reply } => {
            let _ = reply.send(session.set_away_message(text.as_deref()).await.map_err(|e| e.to_string()));
        }
        SessionCommand::RequestUserInfo { screen_name, reply } => {
            let _ = reply.send(session.request_user_info(&screen_name).await.map_err(|e| e.to_string()));
        }
        SessionCommand::SendWarning { screen_name, anonymous, reply } => {
            let _ = reply.send(session.send_warning(&screen_name, anonymous).await.map_err(|e| e.to_string()));
        }
        SessionCommand::AddToBlockList { screen_name, reply } => {
            let _ = reply.send(session.add_to_block_list(&screen_name).await.map_err(|e| e.to_string()));
        }
        SessionCommand::RemoveFromBlockList { screen_name, reply } => {
            let _ = reply.send(session.remove_from_block_list(&screen_name).await.map_err(|e| e.to_string()));
        }
        SessionCommand::CreateChatRoom { room_name, invite_screen_names, reply } => {
            let mut frames = ActorFrameSource { rx: frame_rx };
            match session.create_and_join_room(&room_name, &mut frames).await {
                Ok(room) => {
                    let label = room_label_for(&room.handle.room_cookie);
                    let handle = room.handle.clone();
                    crate::chat_actor::spawn(app.clone(), room, label.clone());
                    for screen_name in &invite_screen_names {
                        let invitation_text = format!("Join {}", handle.room_name);
                        if let Err(e) = session.send_chat_invite(screen_name, &handle, &invitation_text).await {
                            eprintln!("[oscarkit] failed to invite {screen_name} to room {}: {e}", handle.room_name);
                        }
                    }
                    let _ = reply.send(Ok(label));
                }
                Err(e) => {
                    let _ = reply.send(Err(e.to_string()));
                }
            }
        }
        SessionCommand::AcceptChatInvite { index, reply } => {
            let Some(invite) = session.incoming_chat_invites.get(index).cloned() else {
                let _ = reply.send(Err("invite no longer available".to_string()));
                return;
            };
            session.incoming_chat_invites.remove(index);
            let mut frames = ActorFrameSource { rx: frame_rx };
            match session.join_room(&invite.room, &mut frames).await {
                Ok(room) => {
                    let label = room_label_for(&room.handle.room_cookie);
                    crate::chat_actor::spawn(app.clone(), room, label.clone());
                    let _ = reply.send(Ok(label));
                }
                Err(e) => {
                    let _ = reply.send(Err(e.to_string()));
                }
            }
        }
    }
}
