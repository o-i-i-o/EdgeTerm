//! Copies of remote files handed to a local editor.

use super::*;

/// The temporary folder is shared with the rest of the machine, so the copies
/// this module keeps in it are the only ones a cleanup may remove.

#[test]
fn remote_edit_cleanup_leaves_foreign_entries_alone() {
    let root = temp_dir("remote-edit-clean");
    let leftover = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("sshd_config"), b"PasswordAuthentication no\n").unwrap();

    let unsent = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&unsent).unwrap();
    std::fs::write(unsent.join(".unsynced"), b"").unwrap();

    let foreign = root.join("someone-elses-notes");
    std::fs::create_dir_all(&foreign).unwrap();

    #[cfg(unix)]
    let link = {
        let link = root.join(uuid::Uuid::new_v4().to_string());
        std::os::unix::fs::symlink(&foreign, &link).unwrap();
        link
    };

    remote_edit::clean_leftovers_in(&root);

    assert!(!leftover.exists(), "a synced copy is removed");
    assert!(unsent.join(".unsynced").exists(), "unsent copies are kept");
    assert!(foreign.exists(), "an entry this module did not name stays");
    #[cfg(unix)]
    assert!(!link.exists(), "a link is removed, not followed");
    assert!(foreign.exists(), "following the link would have emptied it");

    std::fs::remove_dir_all(&root).ok();
}

/// Editor copies are often privileged files, and the OS temporary folder is
/// shared: the folder they land in must exclude everyone else, including one
/// an earlier build created with the default mode.
#[cfg(unix)]
#[test]
fn remote_edit_folders_are_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = temp_dir("remote-edit-mode");
    let dir = root.join("EdgeTerm").join("edit");
    remote_edit::private_dir(&dir).expect("create the copy root");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(dir.parent().unwrap()), 0o700, "the folder above too");

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    remote_edit::private_dir(&dir).expect("an existing root is kept");
    assert_eq!(mode(&dir), 0o700, "and closed on the next run");

    std::fs::remove_dir_all(&root).ok();
}
