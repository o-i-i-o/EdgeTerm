//! The sender panel's saved snippets, and the shell command history.

use tauri::State;

use super::AppState;
use crate::error::Result;
use crate::model::{CommandHistoryEntry, SavedCommand};

#[tauri::command]
pub fn list_sender_commands(state: State<'_, AppState>) -> Vec<SavedCommand> {
    state.store.list_sender_commands()
}

#[tauri::command]
pub async fn save_sender_command(
    state: State<'_, AppState>,
    command: SavedCommand,
) -> Result<SavedCommand> {
    state.store.save_sender_command(command)
}

#[tauri::command]
pub async fn delete_sender_command(state: State<'_, AppState>, id: String) -> Result<()> {
    state.store.delete_sender_command(&id)
}

#[tauri::command]
pub fn list_command_history(state: State<'_, AppState>) -> Vec<CommandHistoryEntry> {
    state.store.list_command_history()
}

#[tauri::command]
pub async fn record_command(
    state: State<'_, AppState>,
    command: String,
    host: String,
) -> Result<()> {
    state.store.record_command(&command, &host)
}

#[tauri::command]
pub async fn delete_command_history(state: State<'_, AppState>, command: String) -> Result<()> {
    state.store.delete_command_history(&command)
}

#[tauri::command]
pub async fn clear_command_history(state: State<'_, AppState>) -> Result<()> {
    state.store.clear_command_history()
}
