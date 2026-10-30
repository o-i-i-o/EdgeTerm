//! Exporting and importing the whole app data file, and importing an
//! OpenSSH client configuration.

use std::path::Path;

use tauri::State;

use super::AppState;
use crate::error::{AppError, Result};
use crate::model::{APP_DATA_EXTENSION, AppData, DataSummary};
use crate::ssh_config::{self, SshConfigPreview, SshImportSummary};
use crate::store;

/// Writes saved sessions, their groups, Sender tags and the frontend's
/// settings to `path` as pretty JSON. Passwords and passphrases are never
/// included, but the file still names every host the user keeps and is
/// written owner-only and whole (`store::write_owner_only`), like the store's
/// own files. The path must carry the `.edgeterm` extension (the UI appends
/// it), so every data file is recognisable by name.
#[tauri::command]
pub async fn export_app_data(
    state: State<'_, AppState>,
    path: String,
    settings: serde_json::Value,
    exported_at: String,
) -> Result<DataSummary> {
    require_data_file_path(&path)?;
    let mut data = state.store.snapshot();
    data.exported_at = Some(exported_at);
    data.settings = Some(settings);
    let summary = DataSummary {
        profiles: data.profiles.len(),
        groups: data.groups.len(),
        sender_commands: data.sender_commands.len(),
        skipped_sender_commands: 0,
    };
    let payload = serde_json::to_string_pretty(&data)?;
    // The write is blocking file system work, possibly to a network or
    // removable volume: the async runtime has other things to run.
    tokio::task::spawn_blocking(move || store::write_owner_only(Path::new(&path), &payload))
        .await
        .map_err(|error| AppError::new(format!("export task failed: {error}")))??;
    Ok(summary)
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
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(AppError::new(
            "not an EdgeTerm data file: expected an absolute path",
        ));
    }
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(AppError::new(
            "not an EdgeTerm data file: path must not contain '..'",
        ));
    }
    if store::is_data_file_path(path) {
        Ok(())
    } else {
        Err(AppError::new(format!(
            "not an EdgeTerm data file: expected a .{APP_DATA_EXTENSION} file"
        )))
    }
}

/// Merges a file returned by `read_app_data` into the store; see
/// `Store::import_data` for the rules. The frontend applies `settings` itself.
#[tauri::command]
pub async fn import_app_data(state: State<'_, AppState>, data: AppData) -> Result<DataSummary> {
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
