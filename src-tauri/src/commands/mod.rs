//! The commands the frontend invokes, split by what they touch.
//!
//! Every `#[tauri::command]` in this folder is registered in `lib.rs`.

use crate::error::{AppError, Result};
use crate::local_scope::LocalScope;
use crate::model::{DirListing, SessionProfile};
use crate::remote_edit::RemoteEdits;
use crate::session::auth::AuthPrompts;
use crate::session::host_keys::HostKeyChanges;
use crate::session::transfer::Transfers;
use crate::session::{SessionManager, SftpResponse};
use crate::store::Store;
use crate::transfer_access::TransferAccess;

pub mod data;
pub mod drag_out;
pub mod local_fs;
pub mod profiles;
pub mod remote_fs;
pub mod sender;
pub mod sessions;
pub mod transfer;

pub struct AppState {
    pub sessions: SessionManager,
    pub store: Store,
    pub remote_edits: RemoteEdits,
    /// Authentication challenges a connecting session is waiting on; see
    /// `answer_auth_prompt`.
    pub auth_prompts: AuthPrompts,
    /// Host key changes a connection refused, so `accept_host_key` can only
    /// accept the key this process saw.
    pub host_key_changes: HostKeyChanges,
    /// Transfers in flight, so `cancel_transfer` can reach one.
    pub transfers: Transfers,
    /// Local files the ZMODEM / XMODEM commands may touch; see
    /// `transfer_access`.
    pub transfer_access: TransferAccess,
    /// Local folders the Filer has shown, which is where the commands that
    /// change this machine's file system may work; see `local_scope`.
    pub local_scope: LocalScope,
}

/// A secret typed into the dialog wins over the saved one; the dialog sends
/// only what the user entered.
fn merge_secrets(mut saved: SessionProfile, incoming: SessionProfile) -> SessionProfile {
    if incoming.password.is_some() {
        saved.password = incoming.password;
    }
    if incoming.passphrase.is_some() {
        saved.passphrase = incoming.passphrase;
    }
    saved
}

fn expect_listing(response: SftpResponse) -> Result<DirListing> {
    match response {
        SftpResponse::Listing(listing) => Ok(listing),
        _ => Err(AppError::new("unexpected sftp response")),
    }
}

fn expect_path(response: SftpResponse) -> Result<String> {
    match response {
        SftpResponse::Path(path) => Ok(path),
        _ => Err(AppError::new("unexpected sftp response")),
    }
}
