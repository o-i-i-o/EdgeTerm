//! Saved session profiles and the panel groups they sit in.

use tauri::State;

use super::AppState;
use crate::error::Result;
use crate::model::{SessionGroup, SessionProfile};

// Every command below that changes the store is `async` on purpose: the write
// behind it ends with a flush, and Tauri runs a synchronous command inline on
// the event loop's thread, where that flush would hold up the window's input
// and repaint. As an async command it waits on a runtime thread instead.

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Vec<SessionProfile> {
    state.store.list()
}

#[tauri::command]
pub async fn save_profile(
    state: State<'_, AppState>,
    profile: SessionProfile,
) -> Result<SessionProfile> {
    state.store.save(profile)
}

/// Saves a copy of a saved session, stored credentials included.
#[tauri::command]
pub async fn duplicate_profile(state: State<'_, AppState>, id: String) -> Result<SessionProfile> {
    state.store.duplicate(&id)
}

/// Deletes a saved session with its credentials and the Sender commands
/// scoped to it.
#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, id: String) -> Result<()> {
    state.store.delete(&id)
}

#[tauri::command]
pub fn list_session_groups(state: State<'_, AppState>) -> Vec<SessionGroup> {
    state.store.list_groups()
}

#[tauri::command]
pub async fn save_session_group(
    state: State<'_, AppState>,
    group: SessionGroup,
) -> Result<SessionGroup> {
    state.store.save_group(group)
}

/// Removes a group with everything in it: nested groups, their sessions
/// (credentials included) and the Sender commands scoped to any of them.
#[tauri::command]
pub async fn delete_session_group(state: State<'_, AppState>, id: String) -> Result<()> {
    state.store.delete_group(&id)
}
