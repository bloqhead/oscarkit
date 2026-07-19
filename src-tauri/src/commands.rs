//! The `#[tauri::command]` surface — thin wrappers that either kick off a
//! login (spawning the session actor) or forward a `SessionCommand` into an
//! already-running one and await its per-call result. See `session_actor.rs`
//! for why actions go through a channel instead of touching `OscarSession`
//! directly from here.

use std::sync::Mutex;

use tauri::{AppHandle, State};
use tokio::sync::{mpsc, oneshot};

use crate::session_actor::{self, SessionCommand, SessionSnapshot};

/// Holds the command channel for the currently logged-in session, if any.
/// Set once by `login`; every other command reads (and clones) it.
pub struct SessionState(pub Mutex<Option<mpsc::Sender<SessionCommand>>>);

fn sender_or_err(state: &State<'_, SessionState>) -> Result<mpsc::Sender<SessionCommand>, String> {
    state.0.lock().unwrap().clone().ok_or_else(|| "not logged in".to_string())
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
