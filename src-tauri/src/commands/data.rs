//! Exporting and importing saved sessions, program settings, and importing
//! an OpenSSH client configuration.

use std::path::Path;

use tauri::State;

use super::AppState;
use crate::error::{AppError, Result};
use crate::model::{APP_DATA_APP, APP_DATA_EXTENSION, APP_DATA_FORMAT, AppData, DataSummary};
use crate::ssh_config::{self, SshConfigPreview, SshImportSummary};
use crate::store;

/// Writes saved sessions, their groups and Sender tags to `path` as pretty
/// JSON. Passwords and passphrases are never included, but the file still
/// names every host the user keeps and is written owner-only and whole
/// (`store::write_owner_only`), like the store's own files. The path must
/// carry the `.edgeterm` extension (the UI appends it), so every data file is
/// recognisable by name. Settings are a separate export (`export_settings_data`).
#[tauri::command]
pub async fn export_session_data(
    state: State<'_, AppState>,
    path: String,
    exported_at: String,
) -> Result<DataSummary> {
    require_data_file_path(&path)?;
    let mut data = state.store.snapshot();
    data.exported_at = Some(exported_at);
    data.settings = None;
    let summary = DataSummary {
        profiles: data.profiles.len(),
        groups: data.groups.len(),
        sender_commands: data.sender_commands.len(),
        skipped_sender_commands: 0,
    };
    write_data_file(&path, &data).await?;
    Ok(summary)
}

/// Writes only the frontend's settings to `path` as pretty JSON; saved
/// sessions are left out, so a settings backup never carries hosts.
#[tauri::command]
pub async fn export_settings_data(
    path: String,
    settings: serde_json::Value,
    exported_at: String,
) -> Result<()> {
    require_data_file_path(&path)?;
    let data = AppData {
        app: APP_DATA_APP.to_string(),
        format: APP_DATA_FORMAT,
        exported_at: Some(exported_at),
        settings: Some(settings),
        profiles: Vec::new(),
        groups: Vec::new(),
        sender_commands: Vec::new(),
    };
    write_data_file(&path, &data).await
}

/// The blocking, owner-only write every export shares; see
/// `store::write_owner_only`.
async fn write_data_file(path: &str, data: &AppData) -> Result<()> {
    let payload = serde_json::to_string_pretty(data)?;
    let path = path.to_string();
    // The write is blocking file system work, possibly to a network or
    // removable volume: the async runtime has other things to run.
    tokio::task::spawn_blocking(move || store::write_owner_only(Path::new(&path), &payload))
        .await
        .map_err(|error| AppError::new(format!("export task failed: {error}")))??;
    Ok(())
}

/// Parses an EdgeTerm data file so the UI can show what an import would
/// bring in before anything is merged: the name must end in `.edgeterm`, the
/// contents must be JSON with the EdgeTerm marker and a known layout
/// version. Credentials in the file are dropped here so they never reach
/// the webview.
#[tauri::command]
pub fn read_app_data(path: String) -> Result<AppData> {
    require_data_file_path(&path)?;
    let raw = std::fs::read_to_string(&path)?;
    let mut data: AppData = serde_json::from_str(&raw)
        .map_err(|error| AppError::new(format!("not an EdgeTerm data file: {error}")))?;
    store::validate_app_data(&data)?;
    data.profiles = data
        .profiles
        .into_iter()
        .map(store::redact_profile)
        .collect();
    Ok(data)
}

/// Data-file paths arrive from the webview, so they are treated as only
/// partially trusted: absolute, free of `..` components, and carrying the
/// `.edgeterm` extension. In normal use the path comes straight from the
/// system save/open dialog; the extra checks cap the damage a compromised
/// webview could do with the write/read primitives these commands expose.
fn require_data_file_path(path: &str) -> Result<()> {
    let path = crate::local_scope::checked_absolute(path).map_err(|error| {
        AppError::new(format!("not an EdgeTerm data file: {error}"))
    })?;
    if store::is_data_file_path(&path) {
        Ok(())
    } else {
        Err(AppError::new(format!(
            "not an EdgeTerm data file: expected a .{APP_DATA_EXTENSION} file"
        )))
    }
}

/// Merges a file returned by `read_app_data` into the store; see
/// `Store::import_data` for the rules. The frontend applies `settings` itself
/// through the separate settings import.
#[tauri::command]
pub async fn import_app_data(state: State<'_, AppState>, data: AppData) -> Result<DataSummary> {
    if data.profiles.is_empty() && data.groups.is_empty() && data.sender_commands.is_empty() {
        return Err(AppError::new(
            "the file contains no sessions: use Import Settings for a settings file",
        ));
    }
    state.store.import_data(data)
}

/// The file `ssh` reads by default (`~/.ssh/config`), for the open dialog to
/// start at; it may not exist.
#[tauri::command]
pub fn default_ssh_config_path() -> String {
    ssh_config::Context::from_environment()
        .default_config_path()
        .to_string_lossy()
        .into_owned()
}

/// Parses an OpenSSH client configuration and says which of its aliases
/// are already saved, so the dialog can offer the rest.
#[tauri::command]
pub fn read_ssh_config(state: State<'_, AppState>, path: String) -> Result<SshConfigPreview> {
    let context = ssh_config::Context::from_environment();
    ssh_config::preview(&state.store, Path::new(&path), &context)
}

/// Saves the chosen aliases of the file as SSH sessions under `group_id`;
/// see `ssh_config::import` for what comes along and what is updated.
#[tauri::command]
pub async fn import_ssh_config(
    state: State<'_, AppState>,
    path: String,
    aliases: Vec<String>,
    group_id: Option<String>,
) -> Result<SshImportSummary> {
    let context = ssh_config::Context::from_environment();
    let entries = ssh_config::parse_file(Path::new(&path), &context)?;
    ssh_config::import(&state.store, &entries, &aliases, group_id, &context)
}
