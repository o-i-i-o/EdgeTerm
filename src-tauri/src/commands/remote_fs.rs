//! Filesystem work through a session's SFTP or FTP connection.

use std::sync::Arc;

use tauri::State;
use tauri::ipc::Channel;

use super::{AppState, expect_listing, expect_path};
use crate::error::Result;
use crate::host::TauriProgress;
use crate::model::DirListing;
use crate::session::{SftpRequest, TransferProgress};

#[tauri::command]
pub async fn sftp_home(state: State<'_, AppState>, id: String) -> Result<String> {
    expect_path(state.sessions.sftp(&id, SftpRequest::Home).await?)
}

#[tauri::command]
pub async fn sftp_list(state: State<'_, AppState>, id: String, path: String) -> Result<DirListing> {
    expect_listing(state.sessions.sftp(&id, SftpRequest::List { path }).await?)
}

/// Resolves `..`, symlinks and relative paths on the remote host.
#[tauri::command]
pub async fn sftp_canonicalize(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<String> {
    expect_path(
        state
            .sessions
            .sftp(&id, SftpRequest::Canonicalize { path })
            .await?,
    )
}

#[tauri::command]
pub async fn sftp_mkdir(state: State<'_, AppState>, id: String, path: String) -> Result<()> {
    state
        .sessions
        .sftp(&id, SftpRequest::Mkdir { path })
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn sftp_create_file(state: State<'_, AppState>, id: String, path: String) -> Result<()> {
    state
        .sessions
        .sftp(&id, SftpRequest::CreateFile { path })
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    id: String,
    path: String,
    is_dir: bool,
) -> Result<()> {
    state
        .sessions
        .sftp(&id, SftpRequest::Remove { path, is_dir })
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    id: String,
    from: String,
    to: String,
) -> Result<()> {
    state
        .sessions
        .sftp(&id, SftpRequest::Rename { from, to })
        .await?;
    Ok(())
}

/// `transfer`, on this and the three transfers below, is the id
/// `cancel_transfer` can name while the copy runs; without one the transfer
/// cannot be cancelled.
#[tauri::command]
pub async fn sftp_download(
    state: State<'_, AppState>,
    id: String,
    remote: String,
    local: String,
    on_progress: Channel<TransferProgress>,
    transfer: Option<String>,
) -> Result<()> {
    let active = state.transfers.begin(transfer);
    state
        .sessions
        .sftp(
            &id,
            SftpRequest::Download {
                remote,
                local,
                progress: Arc::new(TauriProgress::new(on_progress)),
                cancel: active.flag(),
            },
        )
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn sftp_download_directory(
    state: State<'_, AppState>,
    id: String,
    remote: String,
    local: String,
    on_progress: Channel<TransferProgress>,
    transfer: Option<String>,
) -> Result<()> {
    let active = state.transfers.begin(transfer);
    state
        .sessions
        .sftp(
            &id,
            SftpRequest::DownloadDirectory {
                remote,
                local,
                progress: Arc::new(TauriProgress::new(on_progress)),
                cancel: active.flag(),
            },
        )
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, AppState>,
    id: String,
    local: String,
    remote: String,
    on_progress: Channel<TransferProgress>,
    transfer: Option<String>,
) -> Result<()> {
    let active = state.transfers.begin(transfer);
    state
        .sessions
        .sftp(
            &id,
            SftpRequest::Upload {
                local,
                remote,
                progress: Arc::new(TauriProgress::new(on_progress)),
                cancel: active.flag(),
            },
        )
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn sftp_upload_directory(
    state: State<'_, AppState>,
    id: String,
    local: String,
    remote: String,
    on_progress: Channel<TransferProgress>,
    transfer: Option<String>,
) -> Result<()> {
    let active = state.transfers.begin(transfer);
    state
        .sessions
        .sftp(
            &id,
            SftpRequest::UploadDirectory {
                local,
                remote,
                progress: Arc::new(TauriProgress::new(on_progress)),
                cancel: active.flag(),
            },
        )
        .await?;
    Ok(())
}

/// Stops the transfer the front end registered as `transfer`, if it is still
/// running: the copy ends within a chunk and takes its half-written file
/// with it (see `CancelFlag`).
#[tauri::command]
pub fn cancel_transfer(state: State<'_, AppState>, transfer: String) {
    state.transfers.cancel(&transfer);
}
