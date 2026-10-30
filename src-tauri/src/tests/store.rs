//! The data file itself: import merging, secrets on disk, history.

use super::*;

#[test]
fn store_snapshot_never_carries_credentials() {
    let dir = temp_dir("export");
    let store = Store::load_from(dir.join("sessions.json"));

    let mut ssh = profile(SessionKind::Ssh);
    ssh.auth = Some(AuthKind::Password);
    ssh.password = Some("hunter2".into());
    let ssh = store.save(ssh).expect("save ssh");
    let mut key = profile(SessionKind::Ssh);
    key.auth = Some(AuthKind::PublicKey);
    key.private_key_path = Some("/home/me/.ssh/id_ed25519".into());
    key.passphrase = Some("secret".into());
    store.save(key).expect("save key");
    let group = store
        .save_group(group("Prod", SessionKind::Ssh, None))
        .expect("save group");
    store
        .save_sender_command(command("List", "ls"))
        .expect("save command");

    let data = store.snapshot();
    assert_eq!(data.app, APP_DATA_APP);
    assert_eq!(data.format, APP_DATA_FORMAT);
    assert_eq!(data.profiles.len(), 2);
    assert!(
        data.profiles
            .iter()
            .all(|p| p.password.is_none() && p.passphrase.is_none())
    );
    assert_eq!(data.profiles[0].id, ssh.id);
    assert_eq!(
        data.profiles[1].private_key_path.as_deref(),
        Some("/home/me/.ssh/id_ed25519")
    );
    assert_eq!(data.groups, vec![group]);
    assert_eq!(data.sender_commands.len(), 1);

    let json = serde_json::to_string(&data).expect("serialize");
    assert!(!json.contains("hunter2"));
    assert!(!json.contains("secret"));
    // The local secrets stay usable for reconnects.
    assert_eq!(
        store.get(&ssh.id).expect("get").and_then(|p| p.password),
        Some("hunter2".into())
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_import_merges_by_id_and_drops_secrets_and_bad_links() {
    let dir = temp_dir("import");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let mut local = profile(SessionKind::Ssh);
    local.name = "before".into();
    local.auth = Some(AuthKind::Password);
    local.password = Some("keep-me".into());
    let local = store.save(local).expect("save local");
    let existing_group = store
        .save_group(group("Existing", SessionKind::Ssh, None))
        .expect("save group");
    let existing_command = store
        .save_sender_command(command("Old", "ls"))
        .expect("save command");

    let mut replaced = local.clone();
    replaced.name = "after".into();
    replaced.password = Some("from-file".into());
    replaced.group_id = Some(existing_group.id.clone());
    let mut orphan = profile(SessionKind::Serial);
    orphan.id = "serial-1".into();
    // A serial profile cannot live in an SSH group.
    orphan.group_id = Some(existing_group.id.clone());
    let mut stray = profile(SessionKind::Ssh);
    stray.id = "ssh-2".into();
    stray.group_id = Some("no-such-group".into());
    let mut fresh = profile(SessionKind::Local);
    fresh.passphrase = Some("x".into());

    let mut renamed_command = existing_command.clone();
    renamed_command.name = "Renamed".into();

    let data = AppData {
        app: APP_DATA_APP.into(),
        format: APP_DATA_FORMAT,
        exported_at: None,
        settings: None,
        profiles: vec![replaced, orphan, stray, fresh],
        groups: vec![
            SessionGroup {
                id: "a".into(),
                name: "  A  ".into(),
                kind: SessionKind::Ssh,
                parent_id: Some("b".into()),
            },
            SessionGroup {
                id: "b".into(),
                name: "B".into(),
                kind: SessionKind::Ssh,
                parent_id: Some("a".into()),
            },
            SessionGroup {
                id: "c".into(),
                name: "C".into(),
                kind: SessionKind::Serial,
                parent_id: Some("a".into()),
            },
            SessionGroup {
                id: "d".into(),
                name: "D".into(),
                kind: SessionKind::Ssh,
                parent_id: Some("missing".into()),
            },
            group("   ", SessionKind::Ssh, None),
            // Same id as an existing group of another kind: refused.
            SessionGroup {
                id: existing_group.id.clone(),
                name: "Existing".into(),
                kind: SessionKind::Ftp,
                parent_id: None,
            },
        ],
        sender_commands: vec![renamed_command.clone(), command("New", "pwd")],
    };

    let summary = store.import_data(data).expect("import");
    assert_eq!(summary.profiles, 4);
    assert_eq!(summary.groups, 4);
    assert_eq!(summary.sender_commands, 2);
    assert_eq!(summary.skipped_sender_commands, 0);

    let profiles = store.list();
    assert_eq!(profiles.len(), 4);
    assert!(
        profiles
            .iter()
            .all(|p| p.password.is_none() && p.passphrase.is_none())
    );
    let after = profiles
        .iter()
        .find(|p| p.id == local.id)
        .expect("replaced");
    assert_eq!(after.name, "after");
    assert_eq!(after.group_id.as_deref(), Some(existing_group.id.as_str()));
    // Local credentials survive a same-id import; the file's password is ignored.
    assert_eq!(
        store.get(&local.id).expect("get").and_then(|p| p.password),
        Some("keep-me".into())
    );
    assert_eq!(
        profiles
            .iter()
            .find(|p| p.id == "serial-1")
            .unwrap()
            .group_id,
        None
    );
    assert_eq!(
        profiles.iter().find(|p| p.id == "ssh-2").unwrap().group_id,
        None
    );
    assert!(profiles.iter().all(|p| !p.id.is_empty()));

    let groups = store.list_groups();
    // The blank-named group and the kind-changing one were refused.
    assert_eq!(groups.len(), 5);
    let find = |id: &str| groups.iter().find(|g| g.id == id).expect(id);
    assert_eq!(find(&existing_group.id).kind, SessionKind::Ssh);
    assert_eq!(find("a").name, "A");
    // The a ↔ b cycle is broken at one end, "c" cannot hang under another
    // kind, and "d" pointed nowhere.
    let linked: Vec<_> = ["a", "b"]
        .iter()
        .filter(|id| find(id).parent_id.is_some())
        .collect();
    assert_eq!(linked.len(), 1);
    assert_eq!(find("c").parent_id, None);
    assert_eq!(find("d").parent_id, None);

    let commands = store.list_sender_commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], renamed_command);
    assert_eq!(commands[1].name, "New");

    // Everything was persisted, and reopening is safe.
    let reloaded = Store::load_from(path);
    assert_eq!(reloaded.list().len(), 4);
    assert_eq!(reloaded.list_groups().len(), 5);
    assert_eq!(reloaded.list_sender_commands().len(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_import_refuses_foreign_or_newer_files() {
    let dir = temp_dir("import-refuse");
    let store = Store::load_from(dir.join("sessions.json"));
    let mut data = store.snapshot();
    data.app = "Other".into();
    assert!(store.import_data(data.clone()).is_err());
    data.app = APP_DATA_APP.into();
    data.format = APP_DATA_FORMAT + 1;
    assert!(store.import_data(data.clone()).is_err());
    data.format = 0;
    assert!(store.import_data(data).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn data_files_are_recognised_by_extension_and_contents() {
    assert_eq!(APP_DATA_EXTENSION, "edgeterm");
    assert!(is_data_file_path(Path::new("backup.edgeterm")));
    assert!(is_data_file_path(Path::new("/tmp/A.EDGETERM")));
    assert!(!is_data_file_path(Path::new("backup.json")));
    assert!(!is_data_file_path(Path::new("edgeterm")));
    assert!(!is_data_file_path(Path::new("backup.edgeterm.json")));

    let dir = temp_dir("data-file");
    let store = Store::load_from(dir.join("sessions.json"));
    let mut ssh = profile(SessionKind::Ssh);
    ssh.auth = Some(AuthKind::Password);
    ssh.password = Some("hunter2".into());
    store.save(ssh).expect("save");
    let valid = serde_json::to_string(&store.snapshot()).expect("serialize");

    // Right contents, wrong name: refused before the file is even read.
    let json = dir.join("backup.json");
    std::fs::write(&json, &valid).expect("write json");
    let error = read_app_data(json.display().to_string()).expect_err("json refused");
    assert!(error.to_string().contains(".edgeterm"), "{error}");

    // Right name, wrong contents.
    let garbage = dir.join("garbage.edgeterm");
    std::fs::write(&garbage, "not json").expect("write garbage");
    assert!(read_app_data(garbage.display().to_string()).is_err());
    let foreign = dir.join("foreign.edgeterm");
    std::fs::write(&foreign, r#"{"app":"Other","format":1}"#).expect("write foreign");
    assert!(read_app_data(foreign.display().to_string()).is_err());
    let missing = dir.join("missing.edgeterm");
    assert!(read_app_data(missing.display().to_string()).is_err());

    // Right name and contents; a password smuggled into the file is dropped.
    let good = dir.join("backup.edgeterm");
    let smuggled = valid.replace("\"password\":null", "\"password\":\"x\"");
    assert_ne!(smuggled, valid, "the snapshot serialises an empty password");
    std::fs::write(&good, smuggled).expect("write");
    let data = read_app_data(good.display().to_string()).expect("read");
    assert_eq!(data.app, APP_DATA_APP);
    assert_eq!(data.profiles.len(), 1);
    assert!(data.profiles[0].password.is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_merges_and_persists_command_history() {
    let dir = temp_dir("history");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    store.record_command("ls -la", "ssh:db:22").unwrap();
    store.record_command("ls -la", "ssh:db:22").unwrap();
    store
        .record_command("make test  ", "shell:/bin/zsh")
        .unwrap();
    store.record_command("   ", "shell:/bin/zsh").unwrap();

    let entries = store.list_command_history();
    assert_eq!(
        entries.len(),
        2,
        "duplicates merge, blank lines are dropped"
    );
    let ls = entries
        .iter()
        .find(|entry| entry.command == "ls -la")
        .expect("merged entry");
    assert_eq!(ls.count, 2);
    assert_eq!(ls.host, "ssh:db:22");
    assert!(
        entries.iter().any(|entry| entry.command == "make test"),
        "trailing whitespace is trimmed before storing"
    );

    let reloaded = Store::load_from(path.clone());
    assert_eq!(reloaded.list_command_history().len(), 2);

    reloaded.record_command("ls -la", "shell:/bin/zsh").unwrap();
    reloaded.delete_command_history("ls -la").unwrap();
    let remaining = Store::load_from(path).list_command_history();
    assert_eq!(
        remaining
            .iter()
            .map(|entry| entry.command.as_str())
            .collect::<Vec<_>>(),
        ["make test"],
        "deleting a command forgets it on every host, on disk too"
    );

    reloaded.clear_command_history().unwrap();
    assert!(reloaded.list_command_history().is_empty());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn store_persists_secrets_separately_for_restart_reconnects() {
    let dir = temp_dir("secrets");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let mut p = profile(SessionKind::Ssh);
    p.host = Some("10.0.0.1".into());
    p.auth = Some(AuthKind::Password);
    p.password = Some("hunter2".into());
    let saved_password = store.save(p).expect("save password");

    let mut key_profile = profile(SessionKind::Ssh);
    key_profile.name = "key-server".into();
    key_profile.host = Some("10.0.0.2".into());
    key_profile.auth = Some(AuthKind::PublicKey);
    key_profile.passphrase = Some("open-sesame".into());
    let saved_key = store.save(key_profile).expect("save passphrase");

    let raw = std::fs::read_to_string(&path).expect("read profiles");
    assert!(
        !raw.contains("hunter2"),
        "password must not reach sessions.json"
    );
    assert!(
        !raw.contains("open-sesame"),
        "passphrase must not reach sessions.json"
    );
    assert!(
        saved_password.password.is_none(),
        "returned profile is redacted"
    );
    assert!(
        saved_key.passphrase.is_none(),
        "returned profile is redacted"
    );

    // Saving an edit made from the redacted profile list must keep the existing
    // credential instead of accidentally clearing it.
    let mut renamed = saved_password.clone();
    renamed.name = "renamed-server".into();
    store.save(renamed).expect("save redacted edit");

    // A fresh Store instance hydrates the app-local credentials without using
    // the OS credential vault, which models reopening after a restart.
    let reloaded = Store::load_from(path);
    assert_eq!(
        reloaded
            .get(&saved_password.id)
            .expect("read password")
            .and_then(|p| p.password),
        Some("hunter2".into())
    );
    assert_eq!(
        reloaded
            .get(&saved_key.id)
            .expect("read passphrase")
            .and_then(|p| p.passphrase),
        Some("open-sesame".into())
    );

    reloaded
        .delete(&saved_password.id)
        .expect("delete password");
    reloaded.delete(&saved_key.id).expect("delete passphrase");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_persists_ftp_password_separately() {
    let dir = temp_dir("ftp-secrets");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let mut ftp = profile(SessionKind::Ftp);
    ftp.name = "file-server".into();
    ftp.host = Some("ftp.example.com".into());
    ftp.username = Some("uploader".into());
    ftp.password = Some("ftp-secret".into());
    let saved = store.save(ftp).expect("save ftp profile");

    let raw = std::fs::read_to_string(&path).expect("read profiles");
    assert!(!raw.contains("ftp-secret"));
    assert!(saved.password.is_none());

    let reloaded = Store::load_from(path);
    assert_eq!(
        reloaded
            .get(&saved.id)
            .expect("read ftp password")
            .and_then(|profile| profile.password),
        Some("ftp-secret".into())
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_persists_sftp_secrets_like_ssh() {
    let dir = temp_dir("sftp-secrets");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    // SFTP rides an SSH transport, so it stores credentials by auth kind like
    // SSH: a password for password auth, a passphrase for public-key auth.
    let mut password_profile = profile(SessionKind::Sftp);
    password_profile.name = "sftp-password".into();
    password_profile.host = Some("sftp.example.com".into());
    password_profile.username = Some("uploader".into());
    password_profile.auth = Some(AuthKind::Password);
    password_profile.password = Some("sftp-secret".into());
    let saved_password = store.save(password_profile).expect("save sftp password");

    let mut key_profile = profile(SessionKind::Sftp);
    key_profile.name = "sftp-key".into();
    key_profile.host = Some("sftp.example.com".into());
    key_profile.auth = Some(AuthKind::PublicKey);
    key_profile.passphrase = Some("sftp-passphrase".into());
    let saved_key = store.save(key_profile).expect("save sftp passphrase");

    let raw = std::fs::read_to_string(&path).expect("read profiles");
    assert!(!raw.contains("sftp-secret"));
    assert!(!raw.contains("sftp-passphrase"));
    assert!(saved_password.password.is_none());
    assert!(saved_key.passphrase.is_none());

    let reloaded = Store::load_from(path);
    assert_eq!(
        reloaded
            .get(&saved_password.id)
            .expect("read sftp password")
            .and_then(|profile| profile.password),
        Some("sftp-secret".into())
    );
    assert_eq!(
        reloaded
            .get(&saved_key.id)
            .expect("read sftp passphrase")
            .and_then(|profile| profile.passphrase),
        Some("sftp-passphrase".into())
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn store_files_are_owner_readable_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("perms");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());
    let mut saved = profile(SessionKind::Ssh);
    saved.password = Some("private".into());
    store.save(saved).expect("save");
    store
        .save_sender_command(SavedCommand {
            id: String::new(),
            name: "private tag".into(),
            text: "secret command".into(),
            ending: LineEnding::Lf,
            scope: CommandScope::Global,
            color: None,
        })
        .expect("save sender command");

    for file in [
        path,
        dir.join("credentials.json"),
        dir.join("sender_commands.json"),
    ] {
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{} must be private", file.display());
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// The store rewrites each file whole through a staging file it renames over
/// the target; a write that leaves the staging file behind would litter the
/// config directory and hide the real files from a listing.
#[test]
fn a_saved_store_leaves_no_staging_file_behind() {
    let dir = temp_dir("atomic");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    store.save(profile(SessionKind::Ssh)).expect("save");
    store
        .save_group(group("work", SessionKind::Ssh, None))
        .expect("save group");

    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read config dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging files left behind: {leftovers:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
