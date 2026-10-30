//! Filesystem work on this machine, for the Filer panes.

use tauri::{AppHandle, State};

use super::AppState;
use crate::error::{AppError, Result, err};
use crate::fs_local;
use crate::model::{DirListing, Theme};
use crate::session;
use crate::store;

#[tauri::command]
pub fn local_home() -> String {
    fs_local::home()
}

/// Where the session's shell is right now, for the Filer's "Reveal Working
/// Directory" action; see `session::cwd`.
#[tauri::command]
pub async fn session_cwd(state: State<'_, AppState>, id: String) -> Result<String> {
    state.sessions.query_cwd(&id).await
}

/// This machine's host name, so the frontend can tell a local shell's OSC 7
/// directory report from one a remote shell sent through a hand-typed ssh.
#[tauri::command]
pub fn local_hostname() -> String {
    session::cwd::local_hostname()
}

#[tauri::command]
pub async fn local_list(path: String) -> Result<DirListing> {
    tokio::task::spawn_blocking(move || fs_local::list(&path))
        .await
        .map_err(|error| AppError::new(format!("local listing task failed: {error}")))?
}

#[tauri::command]
pub fn local_parent(path: String) -> String {
    fs_local::parent_of(&path)
}

#[tauri::command]
pub fn local_mkdir(path: String) -> Result<()> {
    fs_local::mkdir(&path)
}

#[tauri::command]
pub fn local_create_file(path: String) -> Result<()> {
    fs_local::create_file(&path)
}

#[tauri::command]
pub fn local_is_directory(path: String) -> bool {
    fs_local::is_directory(&path)
}

/// Opens a local file with the system default application, or with `with`
/// (a macOS `.app` bundle, a Windows executable or a Unix program path) when
/// given. Errors only cover launching; whatever the program does afterwards is
/// its own business.
#[tauri::command]
pub fn open_local_path(path: String, with: Option<String>) -> Result<()> {
    tauri_plugin_opener::open_path(&path, with.as_deref()).map_err(err)
}

/// Remembers the theme the front end is showing, so the next launch creates
/// its window in the matching background colour (`store::save_startup_theme`).
#[tauri::command]
pub async fn set_startup_theme(theme: Theme) -> Result<()> {
    store::save_startup_theme(theme)
}

/// Shows the main window once the frontend has committed its first render.
/// Windows keeps it DWM-cloaked until two animation frames have run; the
/// native timeouts still reveal it if the frontend never acknowledges.
#[tauri::command]
pub fn show_main_window(window: tauri::WebviewWindow) -> Result<()> {
    window.show().map_err(err)?;
    #[cfg(target_os = "windows")]
    {
        // A cloaked HWND is visible to WebView2, which can now paint and
        // drive requestAnimationFrame without exposing the bare background.
        // Some WebView2/DWM combinations may throttle cloaked windows, so
        // never make the JS acknowledgement the only way to reveal it.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            crate::reveal_main_window(&window);
        });
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window.set_focus();
        Ok(())
    }
}

/// Windows: reveal the painted WebView2 window after two visible animation
/// frames. The native timeout above covers a stalled compositor or JS error.
/// Elsewhere the window is already visible, so there is nothing to do.
#[cfg(target_os = "windows")]
#[tauri::command]
pub fn finish_main_window(window: tauri::WebviewWindow) {
    crate::reveal_main_window(&window);
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn finish_main_window() {}

/// The clipboard's text, read by the process rather than the page. On macOS
/// this is the paste path (#47): WebKit lets a page read the pasteboard only
/// from inside its own paste command (⌘V, Edit → Paste on the system menu)
/// and answers any other gesture with a "Paste" confirmation menu the user
/// has to click, so a paste key the user chose could not read it from the
/// page. Elsewhere the page reads its own clipboard
/// (`navigator.clipboard.readText`), and the window is created with
/// `enable_clipboard_access` so WebView2 grants that; but a WebView2 profile
/// that refused the permission prompt before the app granted it keeps
/// refusing (the refusal is stored per origin and the prompt is not raised
/// again), so on Windows the front end falls back to this (#45). Empty when
/// the clipboard holds no text, as the page's read is.
#[tauri::command]
pub fn read_clipboard_text() -> Result<String> {
    #[cfg(windows)]
    {
        if !clipboard_win::is_format_avail(clipboard_win::formats::CF_UNICODETEXT) {
            return Ok(String::new());
        }
        clipboard_win::get_clipboard_string().map_err(err)
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
        let text =
            NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString });
        Ok(text.map(|text| text.to_string()).unwrap_or_default())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("the page reads the clipboard on this platform".into())
    }
}

/// Puts text on the clipboard from the process, for a program in a terminal
/// that set it with OSC 52 (#65). That write arrives with the program's
/// output, never inside a click or key press, and WebKit lets a page write
/// the pasteboard only from inside a user gesture; WebView2 wants the page
/// focused. On Linux the page writes it itself: `enable_clipboard_access`
/// sets WebKitGTK's javascript-can-access-clipboard, which lifts the
/// gesture requirement there, so the front end never sends this.
#[cfg(any(windows, target_os = "macos"))]
#[tauri::command]
pub fn write_clipboard_text(text: String) -> Result<()> {
    #[cfg(windows)]
    {
        clipboard_win::set_clipboard_string(&text).map_err(err)
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
        use objc2_foundation::NSString;
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        if pasteboard.setString_forType(&NSString::from_str(&text), unsafe {
            NSPasteboardTypeString
        }) {
            Ok(())
        } else {
            Err("the pasteboard refused the text".into())
        }
    }
}

/// Brings the mouse pointer back, for the moment before a native dialog
/// opens.
///
/// macOS hides the pointer while the user types: WebKit asks AppKit for
/// `setHiddenUntilMouseMoves:` on every keystroke that lands in an editable
/// area, and a terminal is one, so the pointer is gone the instant a command
/// is typed. AppKit lifts that at the next mouse-moved event *this* process
/// handles, which is why it is never noticed — the pointer is back before
/// the hand is. A file panel breaks that: its window delivers no mouse-moved
/// events to us, so the flag stays set and the panel has no pointer at all
/// for as long as it is up. `rz` is where it hurts, since the panel opens
/// straight off the command the user just typed, and the file to send then
/// has to be picked blind.
///
/// Clearing the flag costs nothing when the pointer is already visible, so
/// every native dialog goes through it (see `nativeDialog.ts`). Elsewhere
/// this is a no-op: Windows hides the pointer while typing through the
/// system setting, which restores it on the next mouse move wherever it is.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn show_pointer(window: tauri::WebviewWindow) -> Result<()> {
    // AppKit, so the main thread; a command is not promised to run on it.
    window
        .run_on_main_thread(|| objc2_app_kit::NSCursor::setHiddenUntilMouseMoves(false))
        .map_err(err)
}

/// A no-op: Windows hides the pointer while typing through the system
/// setting, which restores it on the next mouse move wherever it is.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn show_pointer() -> Result<()> {
    Ok(())
}

/// Whether this copy runs in portable mode (a `data` directory next to the
/// executable holds all configuration). The updater must not run the NSIS
/// installer then; the front end opens the release page instead.
#[tauri::command]
pub fn portable_mode() -> bool {
    crate::store::portable_data_dir().is_some()
}

/// Shows the Windows "Open with" chooser for a local file. Other platforms
/// have no system-wide chooser; the front end picks an application itself,
/// so it never sends this.
#[cfg(windows)]
#[tauri::command]
pub fn open_with_dialog(path: String) -> Result<()> {
    // The shell's classic entry point behind Explorer's "Open with…".
    std::process::Command::new("rundll32.exe")
        .arg("shell32.dll,OpenAs_RunDLL")
        .arg(&path)
        .spawn()
        .map_err(err)?;
    Ok(())
}

/// Where a remote file opened in a local application is downloaded to; the
/// same place again while the file is still being watched. See
/// `remote_edit::RemoteEdits::local_path`.
#[tauri::command]
pub fn remote_edit_path(
    state: State<'_, AppState>,
    id: String,
    remote: String,
    name: String,
) -> Result<String> {
    let path = state.remote_edits.local_path(&id, &remote, &name)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Starts sending `local` back to `remote` on session `id` whenever it
/// changes. Called right after the download, so the copy is the baseline.
#[tauri::command]
pub fn watch_remote_edit(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    local: String,
    remote: String,
) -> Result<()> {
    state
        .remote_edits
        .watch(app, id, std::path::PathBuf::from(local), remote)
}

/// Stops watching every file opened from session `id` and removes the synced
/// copies; for when its tab is closed for good (a disconnect keeps the
/// watchers, see `RemoteEdits`).
#[tauri::command]
pub fn stop_remote_edits(app: AppHandle, state: State<'_, AppState>, id: String) {
    state.remote_edits.stop_session(&app, &id);
}
