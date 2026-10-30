//! Dragging files out of the window: the OS promise that keeps them
//! alive until the drop finishes.

use tauri::State;
use tauri::ipc::Channel;

use super::AppState;
use crate::error::{AppError, Result, err};
#[cfg(target_os = "macos")]
use crate::file_promise::{self, PromisedDragEvent};
use crate::fs_local;
use crate::model::LocalCopySummary;
use crate::session::TransferProgress;

/// What became of a drag started by `start_file_drag`, reported once the
/// pointer is released so the Filer can drop its dragging state.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDragOutcome {
    /// True when the file was handed to a drop target, false when the drag
    /// was cancelled or never started.
    pub dropped: bool,
    /// Set when the drag could not be started at all.
    pub error: Option<String>,
}

/// Where a remote entry is downloaded to before `start_file_drag` hands it to
/// the system; see `fs_local::drag_staging_path`.
#[tauri::command]
pub fn drag_staging_path(name: String) -> Result<String> {
    Ok(fs_local::drag_staging_path(&name)?
        .to_string_lossy()
        .into_owned())
}

/// The drag preview, the same 32×32 icon the bundle installs.
const DRAG_PREVIEW_ICON: &[u8] = include_bytes!("../../icons/32x32.png");

/// Starts a system drag carrying `paths`, so an entry shown in the Filer can
/// be dropped on the desktop or in a file manager (a remote one after it has
/// been staged locally — on Windows and Linux only; macOS promises remote
/// entries instead, see `start_promised_file_drag`). The pointer must still
/// be down: the drag session attaches to the gesture the user is already
/// making.
///
/// The drag is handed to the main thread rather than started here, because
/// that is the only thread AppKit and GTK accept one from, and a command is
/// not promised to run on it. Nothing waits for it either: Windows runs the
/// whole drag inside `DoDragDrop` before the closure returns, so the outcome
/// only ever arrives through `on_event`.
#[tauri::command]
pub fn start_file_drag(
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    on_event: Channel<FileDragOutcome>,
) -> Result<()> {
    if paths.is_empty() {
        return Err(AppError::new("nothing to drag"));
    }
    let files: Vec<std::path::PathBuf> = paths.into_iter().map(Into::into).collect();
    let target = window.clone();
    // `DragItem` holds a boxed provider in its other variant and so is not
    // `Send`; only the paths cross to the main thread.
    window
        .run_on_main_thread(move || {
            let failed = |error: String| {
                let _ = on_event.send(FileDragOutcome {
                    dropped: false,
                    error: Some(error),
                });
            };
            let finished = {
                let on_event = on_event.clone();
                move |result: drag::DragResult, _position: drag::CursorPosition| {
                    let _ = on_event.send(FileDragOutcome {
                        dropped: matches!(result, drag::DragResult::Dropped),
                        error: None,
                    });
                }
            };
            #[cfg(target_os = "linux")]
            let handle = match target.gtk_window() {
                Ok(handle) => handle,
                Err(error) => return failed(error.to_string()),
            };
            #[cfg(not(target_os = "linux"))]
            let handle = target;
            let outcome = drag::start_drag(
                &handle,
                drag::DragItem::Files(files),
                // The application icon stands in for the file: the platforms
                // want a preview image and EdgeTerm ships no other bitmap the
                // size of a cursor.
                drag::Image::Raw(DRAG_PREVIEW_ICON.to_vec()),
                finished,
                drag::Options::default(),
            );
            if let Err(error) = outcome {
                failed(error.to_string());
            }
        })
        .map_err(err)
}

/// macOS: drags a remote entry out as a promise of a file, so nothing is
/// downloaded until a drop target names the path it wants; see
/// `file_promise`. The pointer must still be down, as for `start_file_drag`.
/// Windows and Linux stage the file before the drag instead
/// (`start_file_drag`), so the front end never sends this there.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn start_promised_file_drag(
    window: tauri::WebviewWindow,
    name: String,
    is_dir: bool,
    on_event: Channel<PromisedDragEvent>,
) -> Result<()> {
    file_promise::start(&window, name, is_dir, DRAG_PREVIEW_ICON, on_event)
}

/// Settles the download a promise drag's `write` event asked for: the
/// receiver gets its file, or `error` as the reason it is not coming.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn finish_promised_file(
    window: tauri::WebviewWindow,
    token: String,
    error: Option<String>,
) -> Result<()> {
    file_promise::finish(&window, token, error)
}

/// Copies a dropped file or folder into the folder the Filer is showing, for
/// a drop on a Filer that has no remote session to upload to. Runs off the
/// async runtime: the walk and the copies are blocking file system work.
///
/// The destination is a folder the Filer has listed (`local_scope`); the
/// source is whatever the user dropped, so only the destination is checked.
#[tauri::command]
pub async fn local_copy_into(
    state: State<'_, AppState>,
    source: String,
    destination: String,
    on_progress: Channel<TransferProgress>,
) -> Result<LocalCopySummary> {
    state.local_scope.require_dir(&destination)?;
    tokio::task::spawn_blocking(move || fs_local::copy_into(&source, &destination, &on_progress))
        .await
        .map_err(err)?
}

#[tauri::command]
pub fn local_rename(state: State<'_, AppState>, from: String, to: String) -> Result<()> {
    // Both ends: a rename may move a name into another folder, and both the
    // name leaving and the one arriving have to be in sight.
    state.local_scope.require_entry(&from)?;
    state.local_scope.require_entry(&to)?;
    fs_local::rename(&from, &to)
}

#[tauri::command]
pub fn local_remove(state: State<'_, AppState>, path: String, is_dir: bool) -> Result<()> {
    state.local_scope.require_entry(&path)?;
    fs_local::remove(&path, is_dir)
}
