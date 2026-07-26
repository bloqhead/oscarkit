//! The `#[tauri::command]` surface — thin wrappers that either kick off a
//! login (spawning the session actor) or forward a `SessionCommand` into an
//! already-running one and await its per-call result. See `session_actor.rs`
//! for why actions go through a channel instead of touching `OscarSession`
//! directly from here.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, State};
use tokio::sync::{mpsc, oneshot};

use crate::chat_actor::{ChatCommand, ChatRoomSnapshot};
use crate::session_actor::{self, SessionCommand, SessionSnapshot};

/// Holds the command channel for the currently logged-in session, if any.
/// Set once by `login`; every other command reads (and clones) it.
pub struct SessionState(pub Mutex<Option<mpsc::Sender<SessionCommand>>>);

/// Command channels for every currently-joined chat room, keyed by
/// `session_actor::room_label_for`'s label — one entry per open room
/// window's `chat_actor`. Unlike `SessionState` there can be several at
/// once, so this is a map rather than a single slot.
pub struct ChatRoomsState(pub Mutex<HashMap<String, mpsc::Sender<ChatCommand>>>);

fn sender_or_err(state: &State<'_, SessionState>) -> Result<mpsc::Sender<SessionCommand>, String> {
    state.0.lock().unwrap().clone().ok_or_else(|| "not logged in".to_string())
}

fn chat_sender_or_err(state: &State<'_, ChatRoomsState>, room_label: &str) -> Result<mpsc::Sender<ChatCommand>, String> {
    state.0.lock().unwrap().get(room_label).cloned().ok_or_else(|| "room is no longer open".to_string())
}

/// Sends `command` (built by `make`) to the session actor and awaits its
/// reply — the shared plumbing behind every action command below.
async fn dispatch(
    state: State<'_, SessionState>,
    make: impl FnOnce(oneshot::Sender<Result<(), String>>) -> SessionCommand,
) -> Result<(), String> {
    let sender = sender_or_err(&state)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    sender.send(make(reply_tx)).await.map_err(|_| "session actor is not running".to_string())?;
    reply_rx.await.map_err(|_| "session actor dropped the reply".to_string())?
}

#[tauri::command]
pub async fn login(
    app: AppHandle,
    state: State<'_, SessionState>,
    server: String,
    screen_name: String,
    password: String,
) -> Result<SessionSnapshot, String> {
    let address = oscar_rs::ServerAddress::parse(&server).map_err(|e| e.to_string())?;
    let session = oscar_rs::login(&address, &screen_name, &password).await.map_err(|e| e.to_string())?;
    let snapshot = SessionSnapshot::from(&session);
    let sender = session_actor::spawn(app, session);
    *state.0.lock().unwrap() = Some(sender);
    Ok(snapshot)
}

#[tauri::command]
pub async fn send_message(state: State<'_, SessionState>, recipient: String, text: String) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::SendMessage { recipient, text, reply }).await
}

#[tauri::command]
pub async fn add_buddy(state: State<'_, SessionState>, screen_name: String, group_name: String) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::AddBuddy { screen_name, group_name, reply }).await
}

#[tauri::command]
pub async fn remove_buddy(state: State<'_, SessionState>, screen_name: String) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::RemoveBuddy { screen_name, reply }).await
}

#[tauri::command]
pub async fn set_away_message(state: State<'_, SessionState>, text: Option<String>) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::SetAwayMessage { text, reply }).await
}

#[tauri::command]
pub async fn request_user_info(state: State<'_, SessionState>, screen_name: String) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::RequestUserInfo { screen_name, reply }).await
}

#[tauri::command]
pub async fn send_warning(state: State<'_, SessionState>, screen_name: String, anonymous: bool) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::SendWarning { screen_name, anonymous, reply }).await
}

#[tauri::command]
pub async fn add_to_block_list(state: State<'_, SessionState>, screen_name: String) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::AddToBlockList { screen_name, reply }).await
}

#[tauri::command]
pub async fn remove_from_block_list(state: State<'_, SessionState>, screen_name: String) -> Result<(), String> {
    dispatch(state, |reply| SessionCommand::RemoveFromBlockList { screen_name, reply }).await
}

/// Ends the current session. Clearing the stored sender is enough to tear
/// everything down: the actor's next `cmd_rx.recv()` sees all senders
/// dropped and returns `None`, so its loop breaks, dropping `OscarSession`
/// (closing the write half); the reader task's next send over `frame_tx`
/// then fails (the actor's `frame_rx` dropped with it), so it breaks too and
/// drops the read half, fully closing the connection. No explicit shutdown
/// signal needed.
#[tauri::command]
pub fn logout(state: State<'_, SessionState>) {
    *state.0.lock().unwrap() = None;
}

/// Creates a room, joins it, and best-effort invites `invite_screen_names`.
/// Routes through `SessionState` (not `ChatRoomsState`) because creating a
/// room needs the live `OscarSession` for the ChatNav/Chat redirects and to
/// send the invites — `ChatRoomsState` only exists once a room is already
/// joined.
#[tauri::command]
pub async fn create_room(state: State<'_, SessionState>, room_name: String, invite_screen_names: Vec<String>) -> Result<String, String> {
    let sender = sender_or_err(&state)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    sender
        .send(SessionCommand::CreateChatRoom { room_name, invite_screen_names, reply: reply_tx })
        .await
        .map_err(|_| "session actor is not running".to_string())?;
    reply_rx.await.map_err(|_| "session actor dropped the reply".to_string())?
}

/// Joins the room named by an already-received invite. Also routes through
/// `SessionState` — same reasoning as `create_room` (the invite lives on
/// `OscarSession::incoming_chat_invites`, and joining needs the BOS
/// connection for the Chat redirect).
#[tauri::command]
pub async fn accept_chat_invite(state: State<'_, SessionState>, index: usize) -> Result<String, String> {
    let sender = sender_or_err(&state)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    sender
        .send(SessionCommand::AcceptChatInvite { index, reply: reply_tx })
        .await
        .map_err(|_| "session actor is not running".to_string())?;
    reply_rx.await.map_err(|_| "session actor dropped the reply".to_string())?
}

#[tauri::command]
pub async fn send_chat_message(chat_state: State<'_, ChatRoomsState>, room_label: String, text: String) -> Result<(), String> {
    let sender = chat_sender_or_err(&chat_state, &room_label)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    sender
        .send(ChatCommand::SendMessage { text, reply: reply_tx })
        .await
        .map_err(|_| "room actor is not running".to_string())?;
    reply_rx.await.map_err(|_| "room actor dropped the reply".to_string())?
}

#[tauri::command]
pub async fn get_chat_snapshot(chat_state: State<'_, ChatRoomsState>, room_label: String) -> Result<ChatRoomSnapshot, String> {
    let sender = chat_sender_or_err(&chat_state, &room_label)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    sender.send(ChatCommand::GetSnapshot { reply: reply_tx }).await.map_err(|_| "room actor is not running".to_string())?;
    reply_rx.await.map_err(|_| "room actor dropped the reply".to_string())
}

/// Leaves a room. Same drop-cascade teardown `logout` relies on for the
/// main session: dropping the sender makes the chat actor's next
/// `cmd_rx.recv()` return `None`, breaking its loop, dropping the
/// `ChatRoomSession` (closing the write half), which in turn makes the
/// reader task's next `frame_tx.send()` fail, closing the read half too. No
/// explicit "leave" SNAC exists on the wire — the server infers it from the
/// disconnect and broadcasts `ChatUsersLeft` to whoever's left.
#[tauri::command]
pub fn leave_room(chat_state: State<'_, ChatRoomsState>, room_label: String) {
    chat_state.0.lock().unwrap().remove(&room_label);
}
