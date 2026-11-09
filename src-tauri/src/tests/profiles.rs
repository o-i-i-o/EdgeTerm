//! Profiles, the shell they start, and what the store round-trips about
//! them.

use super::*;

#[test]
fn profile_renders_its_address_per_protocol() {
    let mut ssh = profile(SessionKind::Ssh);
    ssh.host = Some("serverx".into());
    ssh.port = Some(9090);
    assert_eq!(ssh.protocol(), "ssh");
    assert_eq!(ssh.address(), "serverx:9090");

    // Port defaults to 22 when unset.
    ssh.port = None;
    assert_eq!(ssh.address(), "serverx:22");

    let mut ftp = profile(SessionKind::Ftp);
    ftp.host = Some("files.example.com".into());
    assert_eq!(ftp.protocol(), "ftp");
    assert_eq!(ftp.address(), "files.example.com:21");
    ftp.port = Some(2121);
    assert_eq!(ftp.address(), "files.example.com:2121");

    let mut sftp = profile(SessionKind::Sftp);
    sftp.host = Some("files.example.com".into());
    assert_eq!(sftp.protocol(), "sftp");
    // SFTP rides SSH, so it defaults to the SSH port.
    assert_eq!(sftp.address(), "files.example.com:22");
    sftp.port = Some(2222);
    assert_eq!(sftp.address(), "files.example.com:2222");

    let mut serial = profile(SessionKind::Serial);
    serial.port_name = Some("/dev/tty.usbserial".into());
    serial.baud_rate = Some(921_600);
    assert_eq!(serial.protocol(), "serial");
    assert_eq!(serial.address(), "/dev/tty.usbserial@921600");

    let mut local = profile(SessionKind::Local);
    local.shell = Some("/bin/zsh".into());
    assert_eq!(local.protocol(), "shell");
    assert_eq!(local.address(), "/bin/zsh");
}

/// Profiles saved before the two SSH compatibility opt-ins were merged name
/// them separately. Loading folds either one into the single switch, and a
/// save writes only that switch from then on.
#[test]
fn a_profile_saved_with_the_old_compatibility_switches_still_loads() {
    for old_key in ["legacyAlgorithms", "lenientKeyEncoding"] {
        let mut saved = serde_json::to_value(profile(SessionKind::Ssh)).unwrap();
        saved[old_key] = true.into();

        let mut loaded: SessionProfile = serde_json::from_value(saved).unwrap();
        assert!(!loaded.legacy_compat, "{old_key} is folded when loaded");
        loaded.fold_legacy_compat();
        assert!(loaded.legacy_compat, "{old_key} opted in");
    }

    let resaved = serde_json::to_value(profile(SessionKind::Ssh)).unwrap();
    assert!(resaved.get("legacyAlgorithms").is_none());
    assert!(resaved.get("lenientKeyEncoding").is_none());
    assert!(resaved.get("legacyCompat").is_none(), "off is omitted");

    // Only an SSH transport keeps it, as in the dialog.
    let mut local = profile(SessionKind::Local);
    local.legacy_compat = true;
    local.fold_legacy_compat();
    assert!(!local.legacy_compat);
}

#[test]
fn blank_shell_field_means_the_default_shell() {
    let default = crate::model::default_shell();
    let mut local = profile(SessionKind::Local);
    assert_eq!(local.shell_command_line(), default);
    local.shell = Some("   ".into());
    assert_eq!(local.shell_command_line(), default);
    assert_eq!(local.address(), default);
    local.shell = Some("  wsl.exe -d Ubuntu ".into());
    assert_eq!(local.shell_command_line(), "wsl.exe -d Ubuntu");
}

#[test]
fn shell_command_line_splits_into_program_and_arguments() {
    let words = |s: &str| split_command_line(s, false).unwrap();
    assert_eq!(words("/bin/zsh"), ["/bin/zsh"]);
    assert_eq!(words("/bin/zsh -l"), ["/bin/zsh", "-l"]);
    assert_eq!(
        words("  pwsh   -NoLogo\t-NoProfile "),
        ["pwsh", "-NoLogo", "-NoProfile"]
    );
    // Quotes group words with spaces and are removed; `""` is an empty argument.
    assert_eq!(
        words(r#""/Applications/My Shell/sh" 'a b' c"#),
        ["/Applications/My Shell/sh", "a b", "c"]
    );
    assert_eq!(words("sh -c \"\""), ["sh", "-c", ""]);
    assert_eq!(words(r#"pre"fix ed"post"#), ["prefix edpost"]);
    // POSIX escapes: a backslash outside quotes, the shell-escapable set
    // inside double quotes, nothing inside single quotes.
    assert_eq!(words(r"a\ b"), ["a b"]);
    assert_eq!(words(r#""say \"hi\" \$x \n""#), [r#"say "hi" $x \n"#]);
    assert_eq!(words(r"'no \escape'"), [r"no \escape"]);

    assert!(split_command_line("", false).is_err());
    assert!(split_command_line("   ", false).is_err());
    assert!(split_command_line(r#"sh "unterminated"#, false).is_err());
    assert!(split_command_line("sh 'unterminated", false).is_err());
    assert!(split_command_line(r"sh \", false).is_err());
}

#[test]
fn windows_shell_command_line_keeps_backslashes_and_single_quotes() {
    let words = |s: &str| split_command_line(s, true).unwrap();
    assert_eq!(words("wsl.exe -d Ubuntu"), ["wsl.exe", "-d", "Ubuntu"]);
    assert_eq!(
        words(r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#),
        [r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoLogo"]
    );
    assert_eq!(
        words(r"C:\Windows\System32\cmd.exe /k"),
        [r"C:\Windows\System32\cmd.exe", "/k"]
    );
    // `'` is not a quote in cmd and a trailing backslash is a path character.
    assert_eq!(words("pwsh -c 'Get-Date'"), ["pwsh", "-c", "'Get-Date'"]);
    assert_eq!(words(r"cmd /k cd C:\"), ["cmd", "/k", "cd", r"C:\"]);
    assert!(split_command_line(r#"pwsh "unterminated"#, true).is_err());
}

#[test]
fn store_duplicates_profiles_with_their_credentials() {
    let dir = temp_dir("store-duplicate");
    let store = Store::load_from(dir.join("sessions.json"));

    let mut web = profile(SessionKind::Ssh);
    web.name = "web".into();
    web.host = Some("10.0.0.1".into());
    web.password = Some("hunter2".into());
    let web = store.save(web).expect("save");
    let mut db = profile(SessionKind::Ssh);
    db.name = "db".into();
    let db = store.save(db).expect("save");

    let copy = store.duplicate(&web.id).expect("duplicate");
    assert_ne!(copy.id, web.id);
    assert_eq!(copy.name, "web (copy)");
    assert_eq!(copy.host.as_deref(), Some("10.0.0.1"));
    assert_eq!(copy.password, None, "the returned copy carries no secret");
    let ids: Vec<_> = store.list().into_iter().map(|p| p.id).collect();
    assert_eq!(ids, [web.id.clone(), copy.id.clone(), db.id.clone()]);
    assert_eq!(
        store.get(&copy.id).expect("get").and_then(|p| p.password),
        Some("hunter2".into())
    );

    // Copies count on from the original's name, and the copies' secrets are
    // their own: deleting one leaves the others intact.
    let again = store.duplicate(&web.id).expect("duplicate");
    assert_eq!(again.name, "web (copy 2)");
    let of_copy = store.duplicate(&copy.id).expect("duplicate");
    assert_eq!(of_copy.name, "web (copy 3)");
    store.delete(&copy.id).expect("delete");
    assert_eq!(
        store.get(&again.id).expect("get").and_then(|p| p.password),
        Some("hunter2".into())
    );

    assert!(store.duplicate("missing").is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_assigns_ids_and_round_trips_profiles() {
    let dir = temp_dir("store");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let mut p = profile(SessionKind::Ssh);
    p.name = "serverx".into();
    p.host = Some("10.0.0.1".into());
    let saved = store.save(p).expect("save");

    assert!(!saved.id.is_empty(), "an id should be minted on first save");
    assert_eq!(store.list().len(), 1);
    assert_eq!(
        store.get(&saved.id).expect("get").map(|p| p.name),
        Some("serverx".into())
    );

    // Saving the same id updates in place rather than appending.
    let mut updated = saved.clone();
    updated.name = "serverx-prod".into();
    store.save(updated).expect("update");
    assert_eq!(store.list().len(), 1);
    assert_eq!(
        store.get(&saved.id).expect("get").map(|p| p.name),
        Some("serverx-prod".into())
    );

    // A fresh store reads back what was persisted.
    let reloaded = Store::load_from(path);
    assert_eq!(reloaded.list().len(), 1);

    store.delete(&saved.id).expect("delete");
    assert!(store.list().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_resolves_jump_host_chains_and_refuses_loops() {
    let dir = temp_dir("jump");
    let store = Store::load_from(dir.join("sessions.json"));

    let mut bastion = profile(SessionKind::Ssh);
    bastion.name = "bastion".into();
    bastion.host = Some("bastion.example".into());
    bastion.password = Some("pw-bastion".into());
    let bastion = store.save(bastion).expect("save bastion");

    let mut master = profile(SessionKind::Ssh);
    master.name = "master".into();
    master.host = Some("10.0.0.1".into());
    master.jump_profile_id = Some(bastion.id.clone());
    let master = store.save(master).expect("save master");

    let mut node = profile(SessionKind::Sftp);
    node.name = "node".into();
    node.host = Some("10.0.0.7".into());
    node.jump_profile_id = Some(master.id.clone());
    let node = store.save(node).expect("save node");

    // First hop first, and each hop carries its stored credentials so the
    // connection code can authenticate on it.
    let chain = store.jump_chain(&node).expect("chain");
    let names: Vec<&str> = chain.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["bastion", "master"]);
    assert_eq!(chain[0].password.as_deref(), Some("pw-bastion"));
    assert!(store.jump_chain(&bastion).expect("direct").is_empty());

    // bastion → node → master → bastion would loop: refused, not saved.
    let mut looped = store.get(&bastion.id).expect("get").expect("bastion");
    looped.jump_profile_id = Some(node.id.clone());
    assert!(store.save(looped).is_err());
    assert!(
        store
            .get(&bastion.id)
            .expect("get")
            .expect("bastion")
            .jump_profile_id
            .is_none()
    );

    // A jump host must be an existing SSH transport, and only an SSH
    // transport can have one: anything else falls back to a direct
    // connection, like a stale group id.
    let mut stale = profile(SessionKind::Ssh);
    stale.jump_profile_id = Some("no-such-profile".into());
    assert!(store.save(stale).expect("save").jump_profile_id.is_none());
    let mut plain_ftp = profile(SessionKind::Ftp);
    plain_ftp.name = "ftp".into();
    let plain_ftp = store.save(plain_ftp).expect("save ftp");
    let mut via_ftp = profile(SessionKind::Ssh);
    via_ftp.jump_profile_id = Some(plain_ftp.id.clone());
    assert!(store.save(via_ftp).expect("save").jump_profile_id.is_none());
    let mut local = profile(SessionKind::Local);
    local.jump_profile_id = Some(bastion.id.clone());
    assert!(store.save(local).expect("save").jump_profile_id.is_none());

    // The reference survives a restart, and a deleted jump host is reported
    // when the dependent session is opened rather than silently bypassed.
    let reloaded = Store::load_from(dir.join("sessions.json"));
    let node = reloaded.get(&node.id).expect("get").expect("node");
    assert_eq!(node.jump_profile_id.as_deref(), Some(master.id.as_str()));
    reloaded.delete(&master.id).expect("delete master");
    let error = reloaded.jump_chain(&node).expect_err("missing jump host");
    assert!(error.to_string().contains("no longer exists"), "{error}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn store_persists_sender_commands_across_restarts() {
    let dir = temp_dir("sender-commands");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let saved = store
        .save_sender_command(SavedCommand {
            id: String::new(),
            name: "List files".into(),
            text: "ls -la".into(),
            ending: LineEnding::Lf,
            scope: CommandScope::Global,
            color: None,
        })
        .expect("save sender command");
    assert!(!saved.id.is_empty());

    let mut updated = saved.clone();
    updated.ending = LineEnding::Crlf;
    store
        .save_sender_command(updated.clone())
        .expect("update sender command");
    assert_eq!(store.list_sender_commands(), vec![updated.clone()]);

    let reloaded = Store::load_from(path);
    assert_eq!(reloaded.list_sender_commands(), vec![updated]);
    reloaded
        .delete_sender_command(&saved.id)
        .expect("delete sender command");
    assert!(
        Store::load_from(dir.join("sessions.json"))
            .list_sender_commands()
            .is_empty()
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sender_commands_saved_with_a_format_still_load() {
    // Files written before hex sending was removed carry a `format` per
    // command; it is ignored and the command is sent as text. A parse
    // failure here would silently empty the library (`read_json` falls back
    // to nothing), so this is the compatibility check that matters.
    let dir = temp_dir("sender-format");
    std::fs::write(
        dir.join("sender_commands.json"),
        r#"[{"id":"a","name":"frame","text":"DE AD","format":"hex","ending":"crlf","scope":{"type":"global"}},
            {"id":"b","name":"ls","text":"ls","format":"text","ending":"lf","scope":{"type":"kind","kind":"ssh"}}]"#,
    )
    .expect("write legacy file");
    let commands = Store::load_from(dir.join("sessions.json")).list_sender_commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].text, "DE AD");
    assert_eq!(commands[0].ending, LineEnding::Crlf);
    assert_eq!(
        commands[1].scope,
        CommandScope::Kind {
            kind: SessionKind::Ssh
        }
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sender_command_colors_round_trip_and_default_to_none() {
    let dir = temp_dir("sender-color");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let tinted = store
        .save_sender_command(SavedCommand {
            color: Some("teal".into()),
            ..command("deploy", "make deploy")
        })
        .expect("save tinted");
    let plain = store
        .save_sender_command(command("ls", "ls"))
        .expect("save plain");

    // An untinted tag writes no `color`, so an older build reads back
    // exactly what it wrote.
    let json = std::fs::read_to_string(dir.join("sender_commands.json")).expect("read file");
    assert_eq!(json.matches("\"color\"").count(), 1, "{json}");

    let reloaded = Store::load_from(path).list_sender_commands();
    assert_eq!(reloaded, vec![tinted, plain]);

    // A palette name from a newer build is kept as text rather than failing
    // the whole file (`read_json` would then empty the library).
    let future: SavedCommand = serde_json::from_str(
        r#"{"id":"f","name":"x","text":"x","ending":"lf","color":"chartreuse"}"#,
    )
    .expect("unknown color still parses");
    assert_eq!(future.color.as_deref(), Some("chartreuse"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn close_confirmation_round_trips_and_defaults_to_asking() {
    let dir = temp_dir("close-confirm");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let mut quiet = profile(SessionKind::Ssh);
    quiet.name = "quiet".into();
    quiet.confirm_close = Some(false);
    let quiet = store.save(quiet).expect("save");
    let back = Store::load_from(path.clone())
        .get(&quiet.id)
        .expect("get")
        .expect("saved profile");
    assert_eq!(back.confirm_close, Some(false));

    // An untouched profile writes nothing, so a file an older build reads
    // back is exactly what it wrote; a missing field means "ask".
    let mut plain = profile(SessionKind::Local);
    plain.name = "plain".into();
    let plain = store.save(plain).expect("save plain");
    let json = std::fs::read_to_string(&path).expect("read sessions.json");
    assert_eq!(json.matches("\"confirmClose\"").count(), 1, "{json}");
    let plain = Store::load_from(path)
        .get(&plain.id)
        .expect("get")
        .expect("plain profile");
    assert!(plain.confirm_close.is_none());

    std::fs::remove_dir_all(&dir).ok();
}
