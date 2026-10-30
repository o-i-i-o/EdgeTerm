//! The local folders the Filer has shown, which is what the commands that
//! change this machine's file system may touch.
//!
//! Those commands take paths from the webview, and the Filer legitimately
//! browses anywhere, so a path on its own proves nothing. `local_list`
//! therefore records every folder it returns, and an entry may only be
//! created, renamed or removed inside one of them — the same rule
//! `TransferAccess` applies to the paths a native dialog picked.
//!
//! What this buys is smaller than it looks, and the difference matters when
//! deciding what else a boundary needs: the list is filled by a command the
//! page itself can call, so script running in the window can name a folder
//! first and act on it second. The check stops a path that was never shown —
//! a guess at `~/.ssh/id_ed25519`, a bug that built the wrong string, an
//! injection that only knows one command — not a page determined to reach a
//! folder it is willing to list. Making the list unforgeable would mean the
//! Filer could only walk down from folders a native dialog chose, which is
//! not how a file manager with a path box works.
//!
//! Folders are remembered, not files: an entry can appear or disappear
//! between two listings, and refusing a name the folder is about to hold
//! would be wrong. What the user was shown is the folder.

use std::path::{Component, Path, PathBuf};

use parking_lot::Mutex;

use crate::error::{AppError, Result};

/// Folders remembered at once. Browsing keeps adding to this, so the oldest
/// makes way; a long session's history fits many times over.
const MAX_DIRS: usize = 512;

#[derive(Default)]
pub struct LocalScope {
    /// Folders `local_list` has returned, in the order they were shown.
    dirs: Mutex<Vec<PathBuf>>,
}

impl LocalScope {
    /// Records a folder the Filer has just shown. `dir` is what `local_list`
    /// returned, already canonical, so the entries the Filer goes on to name
    /// resolve to the same folder here.
    ///
    /// Called for every listing, including one the page asked for on its own,
    /// which is why the module comment is careful about what this can and
    /// cannot stop.
    pub fn grant(&self, dir: &Path) {
        let mut dirs = self.dirs.lock();
        if dirs.iter().any(|known| known == dir) {
            return;
        }
        if dirs.len() == MAX_DIRS {
            dirs.remove(0);
        }
        dirs.push(dir.to_path_buf());
    }

    /// The path of an entry inside a folder the Filer has shown.
    ///
    /// This is the check for everything that creates, moves or removes a
    /// name: a new folder or file, a delete, either end of a rename. The
    /// entry itself need not exist yet, but the folder holding it must be one
    /// the user has been shown.
    pub fn require_entry(&self, path: &str) -> Result<()> {
        let path = checked(path)?;
        let Some(dir) = path.parent() else {
            return Err(AppError::new(format!("{} names no folder", path.display())));
        };
        self.require_shown(dir)
    }

    /// The path of a folder the Filer has shown, for an operation whose
    /// subject is the folder itself — where a drop is copied into.
    pub fn require_dir(&self, path: &str) -> Result<()> {
        let path = checked(path)?;
        self.require_shown(&path)
    }

    fn require_shown(&self, dir: &Path) -> Result<()> {
        if self.dirs.lock().iter().any(|known| known == dir) {
            return Ok(());
        }
        Err(AppError::new(format!(
            "the Filer has not shown {}; open it first",
            dir.display()
        )))
    }
}

/// `path` as an absolute path with no `..` in it, or an error saying which
/// of the two it broke. Nothing is resolved against the file system: the
/// folder is compared to what `local_list` returned, which is canonical
/// already.
fn checked(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(AppError::new(format!(
            "{} is not an absolute path",
            path.display()
        )));
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(AppError::new(format!(
            "{} must not contain '..'",
            path.display()
        )));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(showing: &[&str]) -> LocalScope {
        let scope = LocalScope::default();
        for dir in showing {
            scope.grant(Path::new(dir));
        }
        scope
    }

    #[test]
    fn an_entry_in_a_shown_folder_is_allowed() {
        let scope = scope(&["/home/user/projects"]);
        assert!(
            scope
                .require_entry("/home/user/projects/report.txt")
                .is_ok()
        );
        // A name the folder does not hold yet: a new file or folder.
        assert!(scope.require_entry("/home/user/projects/src").is_ok());
    }

    #[test]
    fn an_entry_beside_a_shown_folder_is_not() {
        let scope = scope(&["/home/user/projects"]);
        assert!(scope.require_entry("/home/user/.ssh/id_ed25519").is_err());
        // A folder below the shown one is a different folder, and has not
        // been shown itself.
        assert!(
            scope
                .require_entry("/home/user/projects/src/main.rs")
                .is_err()
        );
    }

    #[test]
    fn a_shown_folder_is_its_own_scope() {
        let scope = scope(&["/home/user/projects"]);
        assert!(scope.require_dir("/home/user/projects").is_ok());
        assert!(scope.require_dir("/home/user").is_err());
    }

    #[test]
    fn a_path_that_climbs_out_is_refused() {
        let scope = scope(&["/home/user/projects"]);
        assert!(
            scope
                .require_entry("/home/user/projects/../../.ssh/id_ed25519")
                .is_err()
        );
        assert!(scope.require_dir("/home/user/projects/..").is_err());
    }

    #[test]
    fn a_relative_path_is_refused() {
        let scope = scope(&["/home/user/projects"]);
        assert!(scope.require_entry("projects/report.txt").is_err());
        assert!(scope.require_entry("").is_err());
    }

    #[test]
    fn a_folder_shown_again_keeps_one_entry() {
        let scope = LocalScope::default();
        {
            let dirs = scope.dirs.lock();
            assert!(dirs.is_empty());
        }
        for _ in 0..3 {
            scope.grant(Path::new("/home/user/projects"));
        }
        assert_eq!(scope.dirs.lock().len(), 1);
    }

    #[test]
    fn the_oldest_folder_makes_way_once_the_list_is_full() {
        let scope = LocalScope::default();
        for index in 0..=MAX_DIRS {
            scope.grant(Path::new(&format!("/home/user/{index}")));
        }
        assert_eq!(scope.dirs.lock().len(), MAX_DIRS);
        assert!(scope.require_dir("/home/user/0").is_err());
        assert!(scope.require_dir("/home/user/1").is_ok());
        assert!(scope.require_dir(&format!("/home/user/{MAX_DIRS}")).is_ok());
    }
}
