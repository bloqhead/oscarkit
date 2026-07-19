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

use oscar_rs::{Buddy, FlapReader, IncomingIm, OscarSession};

pub enum SessionCommand {
    SendMessage { recipient: String, text: String, reply: oneshot::Sender<Result<(), String>> },
    AddBuddy { screen_name: String, group_name: String, reply: oneshot::Sender<Result<(), String>> },
    RemoveBuddy { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
    SetAwayMessage { text: Option<String>, reply: oneshot::Sender<Result<(), String>> },
    RequestUserInfo { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
    SendWarning { screen_name: String, anonymous: bool, reply: oneshot::Sender<Result<(), String>> },
    AddToBlockList { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
    RemoveFromBlockList { screen_name: String, reply: oneshot::Sender<Result<(), String>> },
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
}

impl From<&OscarSession> for SessionSnapshot {
    fn from(session: &OscarSession) -> Self {
        SessionSnapshot {
            screen_name: session.screen_name.clone(),
            buddies: session.buddies.clone(),
            incoming_messages: session.incoming_messages.clone(),
            away_message: session.away_message.clone(),
        }
    }
}

enum FrameEvent {
    Frame(oscar_rs::FlapFrame),
    Closed,
    Error(std::io::Error),
}

async fn run_reader(mut reader: FlapReader, tx: mpsc::Sender<FrameEvent>) {
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
                        Some(command) => handle_command(&mut session, command).await,
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

async fn handle_command(session: &mut OscarSession, command: SessionCommand) {
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
    }
}
