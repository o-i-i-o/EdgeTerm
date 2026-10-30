//! Session groups across the kinds and namespaces.

use super::*;

#[test]
fn store_round_trips_session_groups_and_profile_membership() {
    let dir = temp_dir("groups");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let prod = store
        .save_group(group("  prod  ", SessionKind::Ssh, None))
        .expect("save group");
    assert!(!prod.id.is_empty());
    // Names are trimmed before they are stored.
    assert_eq!(prod.name, "prod");
    let eu = store
        .save_group(group("eu", SessionKind::Ssh, Some(&prod.id)))
        .expect("save nested group");

    let mut member = profile(SessionKind::Ssh);
    member.group_id = Some(eu.id.clone());
    let member = store.save(member).expect("save profile");
    assert_eq!(member.group_id.as_deref(), Some(eu.id.as_str()));

    // A profile of another kind cannot sit in an SSH group; it falls back to
    // its own kind root rather than being rejected or stranded.
    let mut stray = profile(SessionKind::Serial);
    stray.group_id = Some(eu.id.clone());
    let stray = store.save(stray).expect("save stray profile");
    assert_eq!(stray.group_id, None);

    // Everything survives a reload from disk.
    let reloaded = Store::load_from(path);
    let groups = reloaded.list_groups();
    assert_eq!(groups.len(), 2);
    assert!(
        groups
            .iter()
            .any(|g| g.id == eu.id && g.parent_id.as_deref() == Some(&*prod.id))
    );
    let saved = reloaded
        .list()
        .into_iter()
        .find(|p| p.id == member.id)
        .expect("member profile persisted");
    assert_eq!(saved.group_id.as_deref(), Some(eu.id.as_str()));

    // sessions.json itself stays a plain profile array for older builds.
    let raw = std::fs::read_to_string(dir.join("sessions.json")).expect("read sessions.json");
    assert!(raw.trim_start().starts_with('['));
    assert!(dir.join("session_groups.json").exists());

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn ftp_and_sftp_share_one_group_namespace() {
    let dir = temp_dir("ftp-sftp-groups");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    // A group in the shared "(S)FTP" section is stored under the canonical FTP
    // kind; it holds servers of either protocol.
    let servers = store
        .save_group(group("servers", SessionKind::Ftp, None))
        .expect("save (s)ftp group");

    let mut ftp = profile(SessionKind::Ftp);
    ftp.group_id = Some(servers.id.clone());
    let ftp = store.save(ftp).expect("save ftp member");
    assert_eq!(ftp.group_id.as_deref(), Some(servers.id.as_str()));

    // An SFTP profile joins the same FTP-category group instead of being
    // stranded — this is the shared-namespace behaviour.
    let mut sftp = profile(SessionKind::Sftp);
    sftp.group_id = Some(servers.id.clone());
    let sftp = store.save(sftp).expect("save sftp member");
    assert_eq!(sftp.group_id.as_deref(), Some(servers.id.as_str()));

    // A session of an unrelated category still cannot sit in that group.
    let mut ssh = profile(SessionKind::Ssh);
    ssh.group_id = Some(servers.id.clone());
    let ssh = store.save(ssh).expect("save ssh profile");
    assert_eq!(ssh.group_id, None);

    // Membership survives a reload from disk.
    let reloaded = Store::load_from(path);
    let saved_sftp = reloaded
        .list()
        .into_iter()
        .find(|p| p.id == sftp.id)
        .expect("sftp member persisted");
    assert_eq!(saved_sftp.group_id.as_deref(), Some(servers.id.as_str()));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn telnet_shares_the_ssh_groups_and_keeps_its_own_password() {
    let dir = temp_dir("telnet-groups");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let site = store
        .save_group(group("site", SessionKind::Ssh, None))
        .expect("save ssh group");

    let mut telnet = profile(SessionKind::Telnet);
    telnet.group_id = Some(site.id.clone());
    telnet.host = Some("switch.lan".into());
    // Typed at the login prompt for the user (issue #89), and kept with
    // the other secrets, never in sessions.json.
    telnet.password = Some("secret".into());
    let telnet = store.save(telnet).expect("save telnet member");
    assert_eq!(telnet.group_id.as_deref(), Some(site.id.as_str()));
    assert_eq!(telnet.address(), "switch.lan:23");
    assert_eq!(telnet.password, None);

    let saved = store.get(&telnet.id).expect("get").expect("telnet saved");
    assert_eq!(saved.password.as_deref(), Some("secret"));

    // Switching the protocol in the editor keeps the id; the password does
    // not follow, so it never goes out in clear where it was meant for SSH
    // or the other way round.
    let mut ssh = profile(SessionKind::Ssh);
    ssh.host = Some("router.lan".into());
    ssh.password = Some("ssh-secret".into());
    let ssh = store.save(ssh).expect("save ssh");
    let mut switched = ssh.clone();
    switched.kind = SessionKind::Telnet;
    let switched = store.save(switched).expect("switch to telnet");
    assert_eq!(
        store
            .get(&switched.id)
            .expect("get")
            .expect("saved")
            .password,
        None
    );
    let mut device = switched.clone();
    device.password = Some("telnet-secret".into());
    let device = store.save(device).expect("save telnet password");
    let mut back = device.clone();
    back.kind = SessionKind::Ssh;
    let back = store.save(back).expect("switch to ssh");
    assert_eq!(
        store.get(&back.id).expect("get").expect("saved").password,
        None
    );

    // A telnet profile is no SSH transport, so it can be neither a jump
    // host nor use one.
    let mut via_telnet = profile(SessionKind::Ssh);
    via_telnet.jump_profile_id = Some(telnet.id.clone());
    assert_eq!(
        store.save(via_telnet).expect("save ssh").jump_profile_id,
        None
    );

    let reloaded = Store::load_from(path);
    let persisted = reloaded
        .list()
        .into_iter()
        .find(|p| p.id == telnet.id)
        .expect("telnet persisted");
    assert_eq!(persisted.kind, SessionKind::Telnet);
    assert_eq!(persisted.group_id.as_deref(), Some(site.id.as_str()));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn store_rejects_invalid_group_shapes() {
    let dir = temp_dir("groups-invalid");
    let store = Store::load_from(dir.join("sessions.json"));

    assert!(
        store
            .save_group(group("   ", SessionKind::Ssh, None))
            .is_err()
    );
    assert!(
        store
            .save_group(group("orphan", SessionKind::Ssh, Some("missing")))
            .is_err()
    );

    let ssh = store
        .save_group(group("ssh", SessionKind::Ssh, None))
        .expect("save ssh group");
    // Groups nest only within one session kind.
    assert!(
        store
            .save_group(group("ftp", SessionKind::Ftp, Some(&ssh.id)))
            .is_err()
    );

    let child = store
        .save_group(group("child", SessionKind::Ssh, Some(&ssh.id)))
        .expect("save child");
    // No cycles: neither self-parenting nor moving under a descendant.
    let mut looped = ssh.clone();
    looped.parent_id = Some(ssh.id.clone());
    assert!(store.save_group(looped).is_err());
    let mut looped = ssh.clone();
    looped.parent_id = Some(child.id.clone());
    assert!(store.save_group(looped).is_err());
    // A group keeps its kind for life.
    let mut switched = ssh.clone();
    switched.kind = SessionKind::Ftp;
    assert!(store.save_group(switched).is_err());

    // Renaming in place is fine and keeps the id.
    let mut renamed = child.clone();
    renamed.name = "renamed".into();
    let renamed = store.save_group(renamed).expect("rename");
    assert_eq!(renamed.id, child.id);
    assert_eq!(store.list_groups().len(), 2);

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn deleting_a_group_removes_everything_in_it() {
    let dir = temp_dir("groups-delete");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    let root = store
        .save_group(group("root", SessionKind::Ssh, None))
        .expect("root");
    let mid = store
        .save_group(group("mid", SessionKind::Ssh, Some(&root.id)))
        .expect("mid");
    let leaf = store
        .save_group(group("leaf", SessionKind::Ssh, Some(&mid.id)))
        .expect("leaf");
    let sibling = store
        .save_group(group("sibling", SessionKind::Ssh, Some(&root.id)))
        .expect("sibling");

    let mut in_mid = profile(SessionKind::Ssh);
    in_mid.group_id = Some(mid.id.clone());
    in_mid.auth = Some(AuthKind::Password);
    in_mid.password = Some("hunter2".into());
    let in_mid = store.save(in_mid).expect("save in_mid");
    let mut in_leaf = profile(SessionKind::Ssh);
    in_leaf.group_id = Some(leaf.id.clone());
    let in_leaf = store.save(in_leaf).expect("save in_leaf");
    let mut in_sibling = profile(SessionKind::Ssh);
    in_sibling.group_id = Some(sibling.id.clone());
    let in_sibling = store.save(in_sibling).expect("save in_sibling");

    let on_leaf = store
        .save_sender_command(scoped(
            "leaf",
            CommandScope::Group {
                id: leaf.id.clone(),
            },
        ))
        .expect("save on_leaf");
    let on_in_leaf = store
        .save_sender_command(scoped(
            "in_leaf",
            CommandScope::Profile {
                id: in_leaf.id.clone(),
            },
        ))
        .expect("save on_in_leaf");
    let on_sibling = store
        .save_sender_command(scoped(
            "sibling",
            CommandScope::Group {
                id: sibling.id.clone(),
            },
        ))
        .expect("save on_sibling");
    let everywhere = store
        .save_sender_command(command("everywhere", "ls"))
        .expect("save everywhere");

    store.delete_group(&mid.id).expect("delete mid");

    let remaining: Vec<String> = store.list_groups().into_iter().map(|g| g.id).collect();
    assert!(remaining.contains(&root.id));
    assert!(remaining.contains(&sibling.id));
    assert!(!remaining.contains(&mid.id));
    assert!(!remaining.contains(&leaf.id));

    // The direct member and the nested one are gone, credentials included;
    // the sibling's member is untouched.
    let profiles: Vec<String> = store.list().into_iter().map(|p| p.id).collect();
    assert!(!profiles.contains(&in_mid.id));
    assert!(!profiles.contains(&in_leaf.id));
    assert!(profiles.contains(&in_sibling.id));
    assert!(store.get(&in_mid.id).expect("get").is_none());
    assert!(
        !std::fs::read_to_string(dir.join("credentials.json"))
            .expect("credentials file")
            .contains("hunter2")
    );

    let commands: Vec<String> = store
        .list_sender_commands()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert!(!commands.contains(&on_leaf.id));
    assert!(!commands.contains(&on_in_leaf.id));
    assert!(commands.contains(&on_sibling.id));
    assert!(commands.contains(&everywhere.id));

    // Deleting the top-level group empties the tree, and the result is what
    // a fresh process reads back.
    store.delete_group(&root.id).expect("delete root");
    let reloaded = Store::load_from(path);
    assert!(reloaded.list_groups().is_empty());
    assert!(reloaded.list().is_empty());
    assert_eq!(
        reloaded
            .list_sender_commands()
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![everywhere.id]
    );

    // Unknown ids are a no-op rather than an error.
    reloaded.delete_group("nope").expect("delete unknown");

    std::fs::remove_dir_all(dir).ok();
}
