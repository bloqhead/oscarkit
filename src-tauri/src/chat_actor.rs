//! Per-room actor — one dedicated task (plus its own frame-reader task) per
//! joined chat room, spawned by `session_actor::handle_command` when a room
//! is created or an invite accepted. Structurally parallel to
//! `session_actor.rs`, not nested inside it: a room's connection lifecycle,
//! dispatch, and state don't belong coupled to the single BOS session, and
//! there can be several of these running at once (one per open room window)
//! against the one `session_actor`.

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot};

use oscar_rs::{ChatMessage, ChatOccupant, ChatRoomSession};

use crate::commands::ChatRoomsState;
use crate::session_actor::{run_reader, FrameEvent};

pub enum ChatCommand {
    SendMessage { text: String, reply: oneshot::Sender<Result<(), String>> },
    GetSnapshot { reply: oneshot::Sender<ChatRoomSnapshot> },
}

/// A plain-data snapshot of one room's state — the boundary type between
/// `ChatRoomSession` (protocol-crate internals) and the UI, mirroring
/// `SessionSnapshot`'s role for the main session. `messages` only ever
/// grows with *received* messages (the server never echoes what this client
/// sends — see `ChatRoomSession::send_message`'s doc comment) — the
/// frontend is expected to append-diff against this the same way
/// `useImWindow.ts` already does against `SessionSnapshot::incoming_messages`,
/// and to push its own sent messages in optimistically rather than waiting
/// to see them come back here.
#[derive(Clone, serde::Serialize)]
pub struct ChatRoomSnapshot {
    pub room_name: String,
    pub occupants: Vec<ChatOccupant>,
    pub messages: Vec<ChatMessage>,
    pub my_screen_name: String,
    pub closed: bool,
}

impl ChatRoomSnapshot {
    fn from_room(room: &ChatRoomSession, closed: bool) -> Self {
        ChatRoomSnapshot {
            room_name: room.handle.room_name.clone(),
            occupants: room.occupants.clone(),
            messages: room.messages.clone(),
            my_screen_name: room.my_screen_name.clone(),
            closed,
        }
    }
}

/// Spawns the actor task (plus its dedicated frame-reader task, reusing
/// `session_actor::run_reader` verbatim — it's already generic over any
/// `FlapReader`), self-registers into `ChatRoomsState` under
/// `room_label_for(&room.handle.room_cookie)` so `commands.rs`'s room-scoped
/// commands can reach it, and emits `chat-update` targeted at that room's
/// own window (`emit_to`, not the global `session-update` broadcast IM's
/// single-session design uses — each room's updates are only relevant to
/// that room's own window).
pub fn spawn(app: AppHandle, mut room: ChatRoomSession, room_label: String) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ChatCommand>(32);
    let (frame_tx, mut frame_rx) = mpsc::channel::<FrameEvent>(32);

    app.state::<ChatRoomsState>().0.lock().unwrap().insert(room_label.clone(), cmd_tx);

    tauri::async_runtime::spawn(run_reader(room.split_reader(), frame_tx));

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(command) => handle_command(&mut room, command).await,
                        None => break, // window closed / leave_room dropped the sender
                    }
                }
                evt = frame_rx.recv() => {
                    match evt {
                        Some(FrameEvent::Frame(frame)) => {
                            if room.dispatch_frame(frame).await.is_err() {
                                break;
                            }
                        }
                        Some(FrameEvent::Closed) | Some(FrameEvent::Error(_)) => break,
                        None => break, // reader task gone
                    }
                }
            }
            let _ = app.emit_to(&room_label, "chat-update", ChatRoomSnapshot::from_room(&room, false));
        }

        let _ = app.emit_to(&room_label, "chat-update", ChatRoomSnapshot::from_room(&room, true));
        app.state::<ChatRoomsState>().0.lock().unwrap().remove(&room_label);
    });
}

async fn handle_command(room: &mut ChatRoomSession, command: ChatCommand) {
    match command {
        ChatCommand::SendMessage { text, reply } => {
            let _ = reply.send(room.send_message(&text).await.map_err(|e| e.to_string()));
        }
        ChatCommand::GetSnapshot { reply } => {
            let _ = reply.send(ChatRoomSnapshot::from_room(room, false));
        }
    }
}

