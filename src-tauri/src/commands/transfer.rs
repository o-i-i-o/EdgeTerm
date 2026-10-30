//! The local half of the ZMODEM / XMODEM transfers the terminal runs,
//! plus the file dialog that is their only source of paths.

use tauri::{AppHandle, State};

use super::AppState;
use crate::error::{AppError, Result, err};
use crate::model::ZmodemFileInfo;
use crate::transfer_io;

/// Opens the native file dialog and remembers what the user picked; see
/// `transfer_access`. `mode` is "open" (a file to read) or "save" (a place
/// to write one). `None` means the user cancelled.
#[tauri::command]
pub async fn pick_transfer_path(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: String,
    multiple: bool,
    title: Option<String>,
    default_path: Option<String>,
) -> Result<Option<Vec<String>>> {
    use tauri_plugin_dialog::{DialogExt as _, FilePath};

    let (sender, picked) = tokio::sync::oneshot::channel::<Option<Vec<FilePath>>>();
    let mut dialog = app.dialog().file();
    if let Some(title) = title {
        dialog = dialog.set_title(title);
    }
    match mode.as_str() {
        "open" if multiple => dialog.pick_files(move |paths| {
            let _ = sender.send(paths);
        }),
        "open" => dialog.pick_file(move |path| {
            let _ = sender.send(path.map(|path| vec![path]));
        }),
        "save" => {
            if let Some(name) = default_path {
                dialog = dialog.set_file_name(name);
            }
            dialog.save_file(move |path| {
                let _ = sender.send(path.map(|path| vec![path]));
            });
        }
        other => {
            return Err(AppError::new(format!("unknown file dialog mode: {other}")));
        }
    }

    let picked = picked
        .await
        .map_err(|_| AppError::new("the file dialog closed"))?
        .unwrap_or_default();
    let mut chosen = Vec::with_capacity(picked.len());
    for path in picked {
        let path = path.into_path().map_err(err)?;
        state.transfer_access.grant(path.clone());
        chosen.push(path.to_string_lossy().into_owned());
    }
    Ok((!chosen.is_empty()).then_some(chosen))
}

#[tauri::command]
pub async fn zmodem_file_info(state: State<'_, AppState>, path: String) -> Result<ZmodemFileInfo> {
    transfer_io::file_info(&state.transfer_access, &path).await
}

#[tauri::command]
pub async fn zmodem_read_chunk(
    state: State<'_, AppState>,
    path: String,
    offset: u64,
    length: u32,
) -> Result<String> {
    transfer_io::read_chunk(&state.transfer_access, &path, offset, length).await
}

#[tauri::command]
pub async fn zmodem_create_file(state: State<'_, AppState>, path: String) -> Result<()> {
    transfer_io::create_file(&state.transfer_access, &path).await
}

#[tauri::command]
pub async fn zmodem_write_chunk(
    state: State<'_, AppState>,
    path: String,
    offset: u64,
    data: String,
) -> Result<()> {
    transfer_io::write_chunk(&state.transfer_access, &path, offset, &data).await
}

#[tauri::command]
pub async fn zmodem_finish_file(state: State<'_, AppState>, path: String, size: u64) -> Result<()> {
    transfer_io::finish_file(&state.transfer_access, &path, size).await
}
