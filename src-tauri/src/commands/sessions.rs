//! Opening, driving and closing sessions, and listing serial ports.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::mpsc;

use super::{AppState, merge_secrets};
use crate::error::{AppError, ErrorKind, Result, err};
use crate::fonts::{self, FontFamily};
use crate::host::TauriHost;
use crate::model::{OpenSessionOutcome, SerialPortDesc, SessionInfo, SessionKind, SessionProfile};
use crate::session::auth::AuthPrompter;
use crate::session::recording::{self, RecordingHandle};
use crate::session::ssh::{ConnectOutcome, SftpConnectOutcome, SharedTransport};
use crate::session::{self, SessionCommand, SessionHandle};

#[tauri::command]
pub async fn open_session(
    app: AppHandle,
    state: State<'_, AppState>,
    profile: SessionProfile,
    session_id: String,
    // The session whose SSH connection this one should run on instead of
    // dialling and authenticating its own — what Duplicate Tab asks
    // for, so a server behind MFA is answered once per connection rather than
    // once per tab. A connection that has gone down in
    // the meantime is simply dialled again.
    reuse_session_id: Option<String>,
    // The grid the frontend has already fitted its terminal to. A local
    // shell starts on it so its first prompt is laid out for the width it
    // will be drawn in; the 80x24 default would leave it one row high.
    cols: u16,
    rows: u16,
) -> Result<OpenSessionOutcome> {
    let name = profile.name.clone();
    let outcome = start_session(
        app,
        state,
        profile,
        session_id,
        reuse_session_id,
        cols,
        rows,
    )
    .await;
    // A failed connection is what the user later reports; a cancelled one is
    // their own doing, so only the former is logged.
    if let Err(error) = &outcome
        && error.kind() != ErrorKind::Cancelled
    {
        crate::log_error!("session {name} could not be opened: {error}");
    }
    outcome
}

/// The SSH connection a new session asked to share, when the session named
/// still has one and it is still up. `None` means the session opens its own:
/// a connection that went down in the meantime has nothing left to share.
fn reuse(state: &State<'_, AppState>, session_id: &Option<String>) -> Option<Arc<SharedTransport>> {
    state.ssh_transports.get(session_id.as_deref()?)
}

/// Opens the session `open_session` was asked for, so the command itself can
/// log whatever came back.
async fn start_session(
    app: AppHandle,
    state: State<'_, AppState>,
    profile: SessionProfile,
    // The frontend mints the id so it can have a terminal listening before the
    // first byte of output arrives.
    session_id: String,
    reuse_session_id: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<OpenSessionOutcome> {
    // A profile may arrive by id (from the tree) or inline (quick connect).
    let profile = if !profile.id.is_empty() {
        match state.store.get(&profile.id)? {
            // Prefer a secret supplied by the dialog over the saved credential.
            Some(saved) => merge_secrets(saved, profile),
            None => profile,
        }
    } else {
        profile
    };

    let id = if session_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        session_id
    };
    let (tx, rx) = mpsc::channel(session::COMMAND_QUEUE);
    let events: Arc<dyn session::EventSink> = Arc::new(TauriHost::new(app));
    let info = session::make_info(&id, &profile);
    // Shared with the backend's output pump and command loop: nothing is
    // recorded until the user asks, and then only that session records.
    let recording = RecordingHandle::new(&id, &profile);

    let owner_thread = match profile.kind {
        SessionKind::Ftp => {
            let connect_profile = profile.clone();
            let conn = tokio::task::spawn_blocking(move || session::ftp::connect(&connect_profile))
                .await
                .map_err(|e| AppError::new(format!("ftp connection task failed: {e}")))??;
            session::ftp::spawn(events.clone(), id.clone(), conn, rx)?;
            None
        }
        SessionKind::Local => {
            session::local::spawn(
                events.clone(),
                id.clone(),
                &profile,
                rx,
                recording,
                cols,
                rows,
            )?;
            None
        }
        SessionKind::Serial => Some(session::serial::spawn(
            events.clone(),
            id.clone(),
            &profile,
            rx,
            recording,
        )?),
        SessionKind::Telnet => {
            let stream = session::telnet::connect(&profile).await?;
            session::telnet::spawn(
                events.clone(),
                id.clone(),
                stream,
                profile.port.unwrap_or(session::telnet::DEFAULT_PORT),
                rx,
                recording,
                session::telnet::AutoLogin::new(&profile, std::time::Instant::now()),
            );
            None
        }
        SessionKind::Ssh => {
            // A connection to share, when one was asked for and is still up.
            // A live one that will not take another channel is an error, not
            // a cue to log in again: the user asked for no second login, and
            // behind MFA a surprise login costs another code.
            let shared = match reuse(&state, &reuse_session_id) {
                // `connect_on` attributes its own errors: an unusable
                // transport is refused under its name, a refused channel as
                // the session limit it is (see `connect_on`).
                Some(transport) => Some(session::ssh::connect_on(transport, &profile).await?),
                None => None,
            };
            let conn = match shared {
                Some(conn) => conn,
                None => {
                    let prompter = AuthPrompter::ui(events.as_ref(), &state.auth_prompts, &id);
                    match session::ssh::connect(
                        &profile,
                        &state.store.jump_chain(&profile)?,
                        &prompter,
                    )
                    .await?
                    {
                        ConnectOutcome::Ready(conn) => conn,
                        // Nothing was opened; the user decides whether to trust
                        // the new key and the frontend retries with the same
                        // session id. The change is registered first, so accepting
                        // it names the key seen here and not one the page supplies.
                        ConnectOutcome::HostKeyChanged(change) => {
                            return Ok(OpenSessionOutcome::HostKeyChanged {
                                change: state.host_key_changes.register(change),
                            });
                        }
                    }
                }
            };
            state.ssh_transports.register(&id, &conn.transport());
            session::ssh::spawn(events.clone(), id.clone(), conn, rx, recording);
            None
        }
        SessionKind::Sftp => {
            // As with a shell above: a live connection is shared or the open
            // is refused, never silently re-logged in (behind MFA that would
            // surprise the user with a challenge for a code). A connection
            // that `reuse` no longer yields has gone down, which dials its
            // own — but a standing one that cannot carry this profile is
            // refused as an error, not logged in to again.
            let shared = match reuse(&state, &reuse_session_id) {
                Some(transport) => Some(session::ssh::connect_sftp_on(transport, &profile)?),
                None => None,
            };
            let conn = match shared {
                Some(conn) => conn,
                None => {
                    let prompter = AuthPrompter::ui(events.as_ref(), &state.auth_prompts, &id);
                    match session::ssh::connect_sftp(
                        &profile,
                        &state.store.jump_chain(&profile)?,
                        &prompter,
                    )
                    .await?
                    {
                        SftpConnectOutcome::Ready(conn) => conn,
                        // Same host-key decision as a shell session on the
                        // same transport, registered the same way.
                        SftpConnectOutcome::HostKeyChanged(change) => {
                            return Ok(OpenSessionOutcome::HostKeyChanged {
                                change: state.host_key_changes.register(change),
                            });
                        }
                    }
                }
            };
            state.ssh_transports.register(&id, &conn.transport());
            session::ssh::spawn_sftp(events.clone(), id.clone(), conn, rx);
            None
        }
    };

    state.sessions.insert(SessionHandle {
        info: info.clone(),
        tx,
        encoding: session::encoding::terminal_encoding(&profile),
        owner_thread,
    });
    Ok(OpenSessionOutcome::Connected { info })
}

/// Record the key a host now presents, replacing every `known_hosts` entry
/// the file held for it, after the user accepted a reported `HostKeyChange`.
///
/// Only the id is taken from the webview: the host, port and key come from
/// the change this process registered when the handshake was refused, so a
/// page cannot pin a key of its own for a host it did not see.
#[tauri::command]
pub fn accept_host_key(state: State<'_, AppState>, id: String) -> Result<()> {
    let change = state
        .host_key_changes
        .peek(&id)
        .ok_or_else(|| AppError::new("this host key change is no longer waiting for a decision"))?;
    session::ssh::accept_host_key(&change.host, change.port, &change.public_key)?;
    // Only once the file holds the key: a write that failed leaves the dialog
    // on screen, and answering it again has to find the change still there.
    state.host_key_changes.forget(&id);
    Ok(())
}

/// Answers one round of an SSH server's keyboard-interactive challenge (an
/// MFA code, a push confirmation, a menu choice) for the connection waiting
/// on `id`. `responses` is `None` when the user cancelled. The answers are
/// used once and never stored.
#[tauri::command]
pub fn answer_auth_prompt(
    state: State<'_, AppState>,
    id: String,
    responses: Option<Vec<String>>,
) -> Result<()> {
    state.auth_prompts.answer(&id, responses)
}

#[tauri::command]
pub fn close_session(state: State<'_, AppState>, id: String) -> Result<()> {
    // Nothing new may be opened on this session's connection once it is
    // closing; whether the connection itself goes down depends on the other
    // sessions sharing it (see `session::ssh::SharedTransport`).
    state.ssh_transports.forget(&id);
    if let Some(handle) = state.sessions.remove(&id) {
        // Never wait for room here: this runs on the event loop thread. A close
        // that cannot be queued is still a close, because dropping the sender
        // ends the channel and every owner loop drains it to `None`.
        let _ = handle.tx.try_send(SessionCommand::Close);
        drop(handle.tx);
        if let Some(owner_thread) = handle.owner_thread {
            owner_thread.join().map_err(|_| {
                AppError::new(format!("serial session {id} panicked while closing"))
            })?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Vec<SessionInfo> {
    state.sessions.list()
}

/// Starts or stops writing a live terminal session's output to a file.
/// `recording` is the state the user wants; `dir` is that session kind's
/// configured recording folder, with absent or blank meaning its default. The
/// reply is the state afterwards, so the frontend never has to guess.
#[tauri::command]
pub async fn set_session_recording(
    state: State<'_, AppState>,
    id: String,
    recording: bool,
    dir: Option<String>,
) -> Result<bool> {
    state.sessions.set_recording(&id, recording, dir).await
}

/// Where recordings of `kind` go when the user has not chosen a folder; the
/// recording folder dialog shows it as the row's placeholder.
#[tauri::command]
pub fn default_recording_dir(kind: SessionKind) -> String {
    recording::default_dir(kind).display().to_string()
}

/// Creates the folder recordings of `kind` go to (`dir`, or the kind's
/// default) and opens it in the system file manager, so the button works
/// before the first recording has created it. Replies with the folder opened.
#[tauri::command]
pub fn open_recording_folder(kind: SessionKind, dir: Option<String>) -> Result<String> {
    let path = recording::resolve_dir(dir.as_deref(), kind);
    recording::ensure_dir(&path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(err)?;
    Ok(path.display().to_string())
}

/// The font families installed on this machine; see `fonts`. Reading the
/// font directories takes a moment, so it runs off the async runtime.
#[tauri::command]
pub async fn list_system_fonts() -> Vec<FontFamily> {
    tauri::async_runtime::spawn_blocking(fonts::system_font_families)
        .await
        .unwrap_or_default()
}

/// The family the Linux terminal stack leads with; see
/// `fonts::system_monospace_family`. `fc-match` is a process, so it runs off
/// the async runtime too.
#[tauri::command]
pub async fn system_monospace_family() -> Option<String> {
    tauri::async_runtime::spawn_blocking(fonts::system_monospace_family)
        .await
        .ok()
        .flatten()
}

/// Keyboard and paste input, which xterm.js hands us as a UTF-8 string; it
/// goes out in the session's own encoding.
#[tauri::command]
pub fn write_session(state: State<'_, AppState>, id: String, data: String) -> Result<()> {
    state.sessions.write_text(&id, &data)
}

/// Raw bytes, base64-encoded, for ZMODEM's protocol frames; waits until
/// the session has written them (see `SessionManager::write_confirmed`).
#[tauri::command]
pub async fn write_session_binary(
    state: State<'_, AppState>,
    id: String,
    data: String,
) -> Result<()> {
    let bytes = B64.decode(data).map_err(err)?;
    state.sessions.write_confirmed(&id, bytes).await
}

#[tauri::command]
pub fn resize_session(state: State<'_, AppState>, id: String, cols: u16, rows: u16) -> Result<()> {
    state
        .sessions
        .send(&id, SessionCommand::Resize { cols, rows })
}

#[tauri::command]
pub fn list_serial_ports() -> Result<Vec<SerialPortDesc>> {
    session::serial::list_ports()
}
