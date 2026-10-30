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
