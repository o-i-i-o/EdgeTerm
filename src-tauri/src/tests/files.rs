//! Local file work: dropping entries into the Filer, copying, listing,
//! and the transfer commands' path whitelist.

use super::*;

#[test]
fn dropping_a_folder_on_the_filer_merges_it_into_the_shown_directory() {
    let dir = temp_dir("copy-into");
    let source = dir.join("project");
    std::fs::create_dir_all(source.join("src")).unwrap();
    std::fs::write(source.join("README.md"), b"hello").unwrap();
    std::fs::write(source.join("src/main.rs"), vec![7; 40]).unwrap();
    let destination = dir.join("shown");
    // An existing folder of the same name is merged into, and a file already
    // there is replaced — the rules a download into a local folder follows.
    std::fs::create_dir_all(destination.join("project")).unwrap();
    std::fs::write(destination.join("project/keep.txt"), b"kept").unwrap();
    std::fs::write(destination.join("project/README.md"), b"stale").unwrap();

    let summary = fs_local::copy_into(
        source.to_str().unwrap(),
        destination.to_str().unwrap(),
        &silent_progress(),
    )
    .unwrap();

    assert_eq!(summary.files, 2);
    assert_eq!(summary.skipped, 0);
    let copied = destination.join("project");
    assert_eq!(std::fs::read(copied.join("README.md")).unwrap(), b"hello");
    assert_eq!(
        std::fs::read(copied.join("src/main.rs")).unwrap(),
        vec![7; 40]
    );
    assert_eq!(std::fs::read(copied.join("keep.txt")).unwrap(), b"kept");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// A remote entry staged for a drag is a copy of a server's file, and the OS
/// temporary folder is shared with every local user: the folders it lands in
/// must exclude them.
#[cfg(unix)]
#[test]
fn drag_staging_folders_are_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let staged = fs_local::drag_staging_path("report.txt").expect("stage a copy");
    let dir = staged.parent().expect("the per-drag folder");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(dir), 0o700);
    assert_eq!(
        mode(dir.parent().unwrap()),
        0o700,
        "the EdgeTerm folder above it too"
    );
    assert_eq!(staged.file_name().unwrap(), "report.txt");

    // A folder an earlier build created with the default mode is closed on
    // the next run rather than refused.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    fs_local::private_dir(dir).expect("an existing folder is kept");
    assert_eq!(mode(dir), 0o700);

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn dropping_an_item_on_the_folder_it_lives_in_leaves_it_alone() {
    let dir = temp_dir("copy-self");
    let file = dir.join("notes.txt");
    std::fs::write(&file, b"original").unwrap();

    // Copying a path onto itself would truncate it, so it is skipped instead.
    let summary = fs_local::copy_into(
        file.to_str().unwrap(),
        dir.to_str().unwrap(),
        &silent_progress(),
    )
    .unwrap();
    assert_eq!((summary.files, summary.skipped), (0, 1));
    assert_eq!(std::fs::read(&file).unwrap(), b"original");

    let folder = dir.join("tree");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("inner.txt"), b"inner").unwrap();
    let summary = fs_local::copy_into(
        folder.to_str().unwrap(),
        dir.to_str().unwrap(),
        &silent_progress(),
    )
    .unwrap();
    assert_eq!((summary.files, summary.skipped), (0, 1));
    assert_eq!(std::fs::read(folder.join("inner.txt")).unwrap(), b"inner");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_copy_refuses_a_destination_inside_its_own_source() {
    let dir = temp_dir("copy-loop");
    let source = dir.join("outer");
    std::fs::create_dir_all(source.join("inner")).unwrap();

    assert!(
        fs_local::copy_into(
            source.to_str().unwrap(),
            source.join("inner").to_str().unwrap(),
            &silent_progress(),
        )
        .is_err()
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_copy_follows_file_links_skips_directory_links_and_never_writes_through_one() {
    let dir = temp_dir("copy-links");
    let source = dir.join("tree");
    std::fs::create_dir_all(source.join("real")).unwrap();
    std::fs::write(source.join("real/data.bin"), vec![1; 10]).unwrap();
    std::os::unix::fs::symlink(source.join("real/data.bin"), source.join("alias.bin")).unwrap();
    std::os::unix::fs::symlink(&source, source.join("loop")).unwrap();
    std::os::unix::fs::symlink(source.join("missing"), source.join("dangling")).unwrap();
    let destination = dir.join("shown");
    std::fs::create_dir_all(&destination).unwrap();

    let summary = fs_local::copy_into(
        source.to_str().unwrap(),
        destination.to_str().unwrap(),
        &silent_progress(),
    )
    .unwrap();

    assert_eq!((summary.files, summary.skipped), (2, 0));
    let copied = destination.join("tree");
    assert!(copied.join("alias.bin").is_file());
    assert!(copied.join("real/data.bin").is_file());
    assert!(!copied.join("loop").exists());
    assert!(!copied.join("dangling").exists());

    // A link already sitting where a file would land must not be written
    // through: the write would land wherever it points.
    let outside = dir.join("outside.txt");
    std::fs::write(&outside, b"untouched").unwrap();
    let planted = dir.join("planted");
    std::fs::create_dir_all(&planted).unwrap();
    std::os::unix::fs::symlink(&outside, planted.join("notes.txt")).unwrap();
    std::fs::write(dir.join("notes.txt"), b"new").unwrap();
    assert!(
        fs_local::copy_into(
            dir.join("notes.txt").to_str().unwrap(),
            planted.to_str().unwrap(),
            &silent_progress(),
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn local_listing_reports_directories_and_sizes() {
    let dir = temp_dir("fs");
    std::fs::create_dir(dir.join("subdir")).unwrap();
    std::fs::write(dir.join("hello.txt"), b"0123456789").unwrap();

    let listing = fs_local::list(dir.to_str().unwrap()).expect("list");
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["subdir", "hello.txt"]);

    let file = &listing.entries[1];
    assert!(!file.is_dir);
    assert_eq!(file.size, 10);
    assert!(file.modified.is_some());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn local_parent_walks_up_one_level() {
    assert_eq!(fs_local::parent_of("/home/lijian/notes"), "/home/lijian");
    // The root has no parent to walk to.
    assert_eq!(fs_local::parent_of("/"), "/");
}

#[test]
fn local_file_operations_are_non_recursive() {
    let dir = temp_dir("local-operations");
    let folder = dir.join("folder");
    fs_local::mkdir(folder.to_str().unwrap()).expect("create folder");

    let original = folder.join("original.txt");
    let renamed = folder.join("renamed.txt");
    std::fs::write(&original, b"data").unwrap();
    fs_local::rename(original.to_str().unwrap(), renamed.to_str().unwrap()).expect("rename file");

    assert!(
        fs_local::remove(folder.to_str().unwrap(), true).is_err(),
        "non-empty directories must never be removed recursively"
    );
    fs_local::remove(renamed.to_str().unwrap(), false).expect("remove file");
    fs_local::remove(folder.to_str().unwrap(), true).expect("remove empty folder");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn zmodem_file_io_uses_bounded_chunks_and_preserves_offsets() {
    const CHUNK_SIZE: usize = transfer_io::CHUNK_SIZE;

    let dir = temp_dir("zmodem-file-io");
    let path = dir.join("payload.bin");
    let path_string = path.to_string_lossy().into_owned();
    let access = TransferAccess::default();
    assert!(
        transfer_io::create_file(&access, &path_string)
            .await
            .is_err(),
        "a path the dialog never returned must be refused"
    );
    access.grant(path.clone());

    transfer_io::create_file(&access, &path_string)
        .await
        .expect("create target");

    let first = vec![0x5a; CHUNK_SIZE];
    let second = vec![0xa5; 17];
    transfer_io::write_chunk(&access, &path_string, 0, &B64.encode(&first))
        .await
        .expect("write first chunk");
    transfer_io::write_chunk(
        &access,
        &path_string,
        CHUNK_SIZE as u64,
        &B64.encode(&second),
    )
    .await
    .expect("write second chunk");
    transfer_io::finish_file(&access, &path_string, (first.len() + second.len()) as u64)
        .await
        .expect("finish target");

    let info = transfer_io::file_info(&access, &path_string)
        .await
        .expect("read metadata");
    assert_eq!(info.name, "payload.bin");
    assert_eq!(info.size, (first.len() + second.len()) as u64);

    let encoded = transfer_io::read_chunk(&access, &path_string, CHUNK_SIZE as u64, 17)
        .await
        .expect("read second chunk");
    assert_eq!(B64.decode(encoded).unwrap(), second);
    assert!(
        transfer_io::read_chunk(&access, &path_string, 0, (CHUNK_SIZE + 1) as u32)
            .await
            .is_err(),
        "reads larger than the fixed application chunk must be rejected"
    );
    assert!(
        transfer_io::write_chunk(
            &access,
            &path_string,
            0,
            &B64.encode(vec![0; CHUNK_SIZE + 1])
        )
        .await
        .is_err(),
        "writes larger than the fixed application chunk must be rejected"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A receive writes to a name the dialog returned, but a name is not a file:
/// anything that can write the folder could put a link there and have the
/// download land somewhere else.
#[cfg(unix)]
#[tokio::test]
async fn a_link_planted_at_the_destination_is_refused() {
    let dir = temp_dir("zmodem-link");
    let elsewhere = dir.join("elsewhere.txt");
    std::fs::write(&elsewhere, b"untouched").unwrap();
    let link = dir.join("payload.bin");
    std::os::unix::fs::symlink(&elsewhere, &link).unwrap();

    let access = TransferAccess::default();
    access.grant(link.clone());
    let link_string = link.to_string_lossy().into_owned();

    assert!(
        transfer_io::create_file(&access, &link_string)
            .await
            .is_err(),
        "a link at the destination must not be written through"
    );
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"untouched");
    assert!(
        transfer_io::write_chunk(&access, &link_string, 0, &B64.encode(b"x"))
            .await
            .is_err()
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn transfers_accept_only_paths_the_dialog_returned() {
    let access = TransferAccess::default();
    let picked = std::env::temp_dir().join("edgeterm-picked.bin");
    let picked_string = picked.to_string_lossy().into_owned();

    assert!(access.require(&picked_string).is_err());
    access.grant(picked.clone());
    assert_eq!(access.require(&picked_string).unwrap(), picked);

    // A pick covers exactly that file: a neighbour in the same directory, a
    // relative path and a traversal are all refused.
    let neighbour = std::env::temp_dir().join("edgeterm-other.bin");
    assert!(access.require(&neighbour.to_string_lossy()).is_err());
    assert!(access.require("edgeterm-picked.bin").is_err());
    let traversal = format!("{}/../etc/passwd", std::env::temp_dir().display());
    assert!(access.require(&traversal).is_err());
}
