//! Remote path handling shared by the Filer panes.

use super::*;

#[test]
fn join_remote_builds_absolute_paths() {
    assert_eq!(
        join_remote("/home/lijian", "notes.txt"),
        "/home/lijian/notes.txt"
    );
    // A trailing slash must not produce a doubled separator.
    assert_eq!(
        join_remote("/home/lijian/", "notes.txt"),
        "/home/lijian/notes.txt"
    );
    assert_eq!(join_remote("/", "etc"), "/etc");
    assert_eq!(join_remote("", "etc"), "/etc");
    // An already-absolute name wins over the base.
    assert_eq!(join_remote("/home", "/var/log"), "/var/log");
}

#[test]
fn sort_entries_puts_directories_first_then_folds_case() {
    let mut entries = vec![
        entry("zebra.txt", false),
        entry("Apple", true),
        entry("beta.txt", false),
        entry("charlie", true),
    ];
    sort_entries(&mut entries);

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Apple", "charlie", "beta.txt", "zebra.txt"]);
}
