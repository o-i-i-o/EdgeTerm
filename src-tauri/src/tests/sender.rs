//! Sender snippet scopes.

use super::*;

#[test]
fn sender_command_scopes_fall_back_and_follow_their_targets() {
    let dir = temp_dir("scopes");
    let path = dir.join("sessions.json");
    let store = Store::load_from(path.clone());

    // Files written before scopes existed load as global.
    let legacy: SavedCommand =
        serde_json::from_str(r#"{"id":"l","name":"x","text":"x","format":"text","ending":"lf"}"#)
            .expect("legacy command");
    assert_eq!(legacy.scope, CommandScope::Global);

    let parent = store
        .save_group(group("Prod", SessionKind::Ssh, None))
        .expect("parent");
    let child = store
        .save_group(group("EU", SessionKind::Ssh, Some(&parent.id)))
        .expect("child");
    let mut ssh = profile(SessionKind::Ssh);
    ssh.group_id = Some(child.id.clone());
    let ssh = store.save(ssh).expect("ssh");
    let serial = store.save(profile(SessionKind::Serial)).expect("serial");

    // Dangling references fall back to global at save time.
    let dangling = store
        .save_sender_command(scoped(
            "dangling",
            CommandScope::Profile { id: "nope".into() },
        ))
        .expect("save dangling");
    assert_eq!(dangling.scope, CommandScope::Global);

    let on_ssh = store
        .save_sender_command(scoped("ssh", CommandScope::Profile { id: ssh.id.clone() }))
        .expect("save ssh");
    let on_serial = store
        .save_sender_command(scoped(
            "serial",
            CommandScope::Profile {
                id: serial.id.clone(),
            },
        ))
        .expect("save serial");
    let on_child = store
        .save_sender_command(scoped(
            "child",
            CommandScope::Group {
                id: child.id.clone(),
            },
        ))
        .expect("save child");
    let on_parent = store
        .save_sender_command(scoped(
            "parent",
            CommandScope::Group {
                id: parent.id.clone(),
            },
        ))
        .expect("save parent");
    let scope_of = |store: &Store, id: &str| {
        store
            .list_sender_commands()
            .into_iter()
            .find(|c| c.id == id)
            .expect("command")
            .scope
    };

    // A deleted profile takes its own commands with it; the rest stay put.
    let count_before = store.list_sender_commands().len();
    store.delete(&ssh.id).expect("delete ssh");
    store.delete(&serial.id).expect("delete serial");
    let commands = store.list_sender_commands();
    assert_eq!(commands.len(), count_before - 2);
    assert!(
        commands
            .iter()
            .all(|c| c.id != on_ssh.id && c.id != on_serial.id)
    );
    assert_eq!(
        scope_of(&store, &on_child.id),
        CommandScope::Group {
            id: child.id.clone()
        }
    );

    // A deleted group takes its subtree along: nested groups, their sessions
    // and the commands scoped to any of them. Global commands stay.
    let ids = |store: &Store| -> Vec<String> {
        store
            .list_sender_commands()
            .into_iter()
            .map(|c| c.id)
            .collect()
    };
    store.delete_group(&child.id).expect("delete child");
    assert!(!ids(&store).contains(&on_child.id));
    assert!(ids(&store).contains(&on_parent.id));
    store.delete_group(&parent.id).expect("delete parent");
    assert!(!ids(&store).contains(&on_parent.id));
    assert!(ids(&store).contains(&dangling.id));

    // The removals were persisted.
    let reloaded = Store::load_from(path);
    let remaining = ids(&reloaded);
    assert!(!remaining.contains(&on_ssh.id));
    assert!(!remaining.contains(&on_parent.id));
    assert!(remaining.contains(&dangling.id));

    // An import resolves scopes against the merged lists: a profile that
    // arrives in the same file is a valid target, an unknown one is not.
    let mut imported = profile(SessionKind::Local);
    imported.id = "imported-profile".into();
    let summary = reloaded
        .import_data(AppData {
            app: APP_DATA_APP.into(),
            format: APP_DATA_FORMAT,
            exported_at: None,
            settings: None,
            profiles: vec![imported],
            groups: vec![],
            sender_commands: vec![
                scoped(
                    "known",
                    CommandScope::Profile {
                        id: "imported-profile".into(),
                    },
                ),
                scoped("unknown", CommandScope::Group { id: "gone".into() }),
            ],
        })
        .expect("import");
    assert_eq!(summary.sender_commands, 2);
    let commands = reloaded.list_sender_commands();
    let find = |name: &str| commands.iter().find(|c| c.name == name).expect(name);
    assert_eq!(
        find("known").scope,
        CommandScope::Profile {
            id: "imported-profile".into()
        }
    );
    assert_eq!(find("unknown").scope, CommandScope::Global);

    std::fs::remove_dir_all(&dir).ok();
}
