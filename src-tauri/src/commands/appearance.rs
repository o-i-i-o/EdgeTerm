//! The terminal backdrop image.
//!
//! The picked file is copied into the configuration directory under a
//! generated name, so the user's original may move or be deleted without
//! breaking the setting, and the webview reads it back as a data URL. Only
//! the file name is stored on the frontend; nothing here accepts a path
//! outside the image directory.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::error::{AppError, Result};
use crate::store;

/// The webview receives the image as a data URL, so a large file costs both
/// IPC time and renderer memory; a wallpaper beyond this is refused.
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const PREFIX: &str = "background-";

fn image_dir() -> PathBuf {
    store::config_dir().join("backgrounds")
}

/// The canonical extension for a supported image, or `None`.
fn extension(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "bmp" => Some("bmp"),
        _ => None,
    }
}

fn mime(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/bmp",
    }
}

/// A saved name must be a bare `background-*` file name: no separators, no
/// parent directory, nothing that could resolve outside the image directory.
fn checked_path(name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if !name.starts_with(PREFIX) || path.file_name().and_then(|part| part.to_str()) != Some(name) {
        return Err(AppError::new("invalid background image name"));
    }
    Ok(image_dir().join(name))
}

/// Copies `path` in under a new name and returns that name.
///
/// `keep` is the image still in use: the dialog saves a replacement before
/// the user applies it, so the previous copy must survive a cancel. Every
/// other earlier copy is removed.
#[tauri::command]
pub fn save_background_image(path: String, keep: Option<String>) -> Result<String> {
    let source = Path::new(&path);
    let ext = extension(source).ok_or_else(|| AppError::new("unsupported image format"))?;
    let metadata = std::fs::metadata(source)
        .map_err(|err| AppError::new(format!("cannot read image: {err}")))?;
    if !metadata.is_file() {
        return Err(AppError::new("not a file"));
    }
    if metadata.len() > MAX_BYTES {
        return Err(AppError::new("image is larger than 8 MB"));
    }

    let dir = image_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|err| AppError::new(format!("cannot create image directory: {err}")))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let name = format!("{PREFIX}{stamp}.{ext}");
    std::fs::copy(source, dir.join(&name))
        .map_err(|err| AppError::new(format!("cannot copy image: {err}")))?;

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let other = entry.file_name();
            let other = other.to_string_lossy();
            let in_use = Some(other.as_ref()) == keep.as_deref();
            if other.starts_with(PREFIX) && other != name && !in_use {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(name)
}

/// Reads a saved image back as a data URL the CSP allows.
#[tauri::command]
pub fn read_background_image(name: String) -> Result<String> {
    let path = checked_path(&name)?;
    let ext = extension(&path).ok_or_else(|| AppError::new("unsupported image format"))?;
    let bytes =
        std::fs::read(&path).map_err(|err| AppError::new(format!("cannot read image: {err}")))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AppError::new("image is larger than 8 MB"));
    }
    Ok(format!(
        "data:{};base64,{}",
        mime(ext),
        STANDARD.encode(bytes)
    ))
}

/// Removes a copy the user discarded; a missing file is already the goal.
#[tauri::command]
pub fn delete_background_image(name: String) -> Result<()> {
    match std::fs::remove_file(checked_path(&name)?) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::new(format!("cannot remove image: {err}"))),
    }
}
