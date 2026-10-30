//! The only local files the ZMODEM / XMODEM commands may touch.
//!
//! Those commands take paths from the webview, so `pick_transfer_path` opens
//! the native dialog, remembers the files the user chose, and everything else
//! is refused. A compromised webview can still call the streaming commands,
//! but it cannot make the dialog choose on the user's behalf.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;

use crate::error::{AppError, Result};

/// How many picks are remembered per run. A long session with many transfers
/// must not grow this without bound; the oldest pick makes way.
const MAX_PICKS: usize = 256;

#[derive(Default)]
pub struct TransferAccess {
    picked: Mutex<Vec<PathBuf>>,
}

impl TransferAccess {
    /// Records a path the user picked in a native dialog.
    pub fn grant(&self, path: PathBuf) {
        let mut picked = self.picked.lock();
        if picked.contains(&path) {
            return;
        }
        if picked.len() == MAX_PICKS {
            picked.remove(0);
        }
        picked.push(path);
    }

    /// The path, when the user picked exactly it; an error otherwise.
    ///
    /// Matching is exact on purpose: a whole directory would hand out every
    /// file beside the one the user picked.
    pub fn require(&self, path: &str) -> Result<PathBuf> {
        let path = Path::new(path);
        if !path.is_absolute() {
            return Err(AppError::new(
                "a transfer may only use a path chosen in the file dialog: \
                 expected an absolute path",
            ));
        }
        if path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
        {
            return Err(AppError::new(
                "a transfer may only use a path chosen in the file dialog: \
                 the path must not contain '..'",
            ));
        }
        if self.picked.lock().iter().any(|known| known == path) {
            return Ok(path.to_path_buf());
        }
        Err(AppError::new(
            "a transfer may only use a path chosen in the file dialog",
        ))
    }
}
