//! Reading an OpenSSH client configuration and importing its aliases.

use super::*;

#[test]
fn ssh_config_resolves_aliases_the_way_ssh_does() {
    let dir = temp_dir("ssh-config");
    let context = ssh_context(&dir);
    let text = r#"
# defaults first: the first value found wins, like ssh
Host *
  Port 2200

Host web "odd name" *.internal !bad.internal
  HostName web.example.com
  User deploy
  IdentityFile ~/.ssh/deploy_%r

Host web
  Port 22
  User ignored

Host bastion
    HostName=10.0.0.1
    User = gate

Host api
  HostName %h.example.com
  ProxyJump bastion, ops@10.0.0.2:2022

Match host web
  User never

Match all
  User fallback

Host odd-tokens
  ProxyJump none
"#;
    let entries = ssh_config::parse_text(text, &context);
    let aliases: Vec<&str> = entries.iter().map(|entry| entry.alias.as_str()).collect();
    assert_eq!(
        aliases,
        ["web", "odd name", "bastion", "api", "odd-tokens"],
        "one alias per concrete pattern, in file order; wildcards and negations name none"
    );

    let web = entry_named(&entries, "web");
    assert_eq!(web.host, "web.example.com");
    assert_eq!(
        web.port, 2200,
        "the Host * block above wins over the later Port"
    );
    assert_eq!(web.username, "deploy");
    assert_eq!(web.auth, AuthKind::PublicKey);
    assert_eq!(
        web.private_key_path.as_deref(),
        Some(dir.join(".ssh/deploy_deploy").to_str().unwrap()),
        "~ and %r are expanded"
    );

    let bastion = entry_named(&entries, "bastion");
    assert_eq!(
        bastion.host, "10.0.0.1",
        "Keyword=value is the same as Keyword value"
    );
    assert_eq!(bastion.username, "gate");
    assert_eq!(
        bastion.port, 2200,
        "the Host * block at the top shadows every later Port"
    );

    let api = entry_named(&entries, "api");
    assert_eq!(api.host, "api.example.com", "%h is the alias");
    assert_eq!(api.jumps, ["bastion", "ops@10.0.0.2:2022"]);

    let odd = entry_named(&entries, "odd-tokens");
    assert_eq!(
        odd.host, "odd-tokens",
        "an alias without HostName connects to itself"
    );
    assert!(odd.jumps.is_empty(), "ProxyJump none is no jump");
    assert_eq!(odd.username, "fallback", "Match all applies like Host *");
    assert_eq!(
        odd.auth,
        AuthKind::Password,
        "no key named and none on disk"
    );
}

#[test]
fn ssh_config_includes_files_in_place_and_offers_the_default_identity() {
    let dir = temp_dir("ssh-include");
    let ssh_dir = dir.join(".ssh");
    std::fs::create_dir_all(ssh_dir.join("config.d")).expect("mkdir");
    std::fs::write(
        ssh_dir.join("config.d/10-hosts"),
        "Host included\n  HostName inc.example.com\n",
    )
    .expect("write include");
    std::fs::write(ssh_dir.join("config.d/20-more"), "  Port 2020\nHost more\n").expect("write");
    std::fs::write(ssh_dir.join("id_ed25519"), "not really a key").expect("write key");

    let mut context = ssh_context(&dir);
    context.default_identity = Some(ssh_dir.join("id_ed25519").to_string_lossy().into_owned());
    let text = "Include config.d/* missing-file\n  User after\nHost top\n";
    let entries = ssh_config::parse_text(text, &context);
    let aliases: Vec<&str> = entries.iter().map(|entry| entry.alias.as_str()).collect();
    assert_eq!(aliases, ["included", "more", "top"]);

    let included = entry_named(&entries, "included");
    assert_eq!(included.host, "inc.example.com");
    assert_eq!(
        included.port, 2020,
        "a later included file continues the Host block the earlier one opened"
    );
    assert_eq!(
        included.auth,
        AuthKind::PublicKey,
        "a host that names no key gets the identity ssh would try"
    );
    assert_eq!(
        included.private_key_path.as_deref(),
        context.default_identity.as_deref()
    );
    let more = entry_named(&entries, "more");
    assert_eq!(
        more.username, "after",
        "lines after the Include extend its last block"
    );
    assert_eq!(more.port, 22);
}

#[test]
fn ssh_config_import_links_a_single_jump_host_and_updates_saved_sessions() {
    let dir = temp_dir("ssh-import");
    let store = Store::load_from(dir.join("sessions.json"));
    let context = ssh_context(&dir);
    let group = store
        .save_group(group("Imported", SessionKind::Ssh, None))
        .expect("save group");

    // A session saved by hand that the file also describes, by name.
    let mut by_hand = profile(SessionKind::Ssh);
    by_hand.name = "api".into();
    by_hand.host = Some("old.example.com".into());
    by_hand.username = Some("old".into());
    by_hand.auth = Some(AuthKind::Password);
    by_hand.password = Some("secret".into());
    by_hand.color = Some("#123456".into());
    let by_hand = store.save(by_hand).expect("save by hand");

    let text = "\
Host bastion
  HostName 10.0.0.1
  User gate
Host api
  HostName api.example.com
  User svc
  ProxyJump ops@10.0.0.2:2022
Host unrelated
  HostName nowhere
";
    let entries = ssh_config::parse_text(text, &context);
    let preview_existing: Vec<Option<String>> = {
        let preview = ssh_config::preview_entries(&store, entries.clone());
        preview
            .iter()
            .map(|entry| entry.existing_id.clone())
            .collect()
    };
    assert_eq!(
        preview_existing,
        [None, Some(by_hand.id.clone()), None],
        "the preview marks the saved session the alias would update"
    );

    let summary = ssh_config::import(
        &store,
        &entries,
        &["api".to_string()],
        Some(group.id.clone()),
        &context,
    )
    .expect("import");
    assert_eq!(
        (summary.added, summary.updated, summary.jump_hosts),
        (0, 1, 1),
        "api is updated and its single literal jump host gets a profile of its own"
    );
    assert!(summary.warnings.is_empty(), "{:?}", summary.warnings);

    let profiles = store.list();
    let names: Vec<&str> = profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect();
    assert!(
        !names.contains(&"unrelated"),
        "unselected aliases stay out: {names:?}"
    );
    assert!(
        !names.contains(&"bastion"),
        "a host no selected session jumps through is not pulled in: {names:?}"
    );

    let api = store
        .get(&by_hand.id)
        .expect("get")
        .expect("api still saved");
    assert_eq!(api.host.as_deref(), Some("api.example.com"));
    assert_eq!(api.username.as_deref(), Some("svc"));
    assert_eq!(
        api.color.as_deref(),
        Some("#123456"),
        "what the file does not say stays"
    );
    assert_eq!(
        api.auth,
        Some(AuthKind::Password),
        "no key named: auth is left alone"
    );
    assert_eq!(
        api.password.as_deref(),
        Some("secret"),
        "and so is the stored password"
    );
    assert_eq!(api.group_id, None, "an updated session keeps its place");

    let literal = profiles
        .iter()
        .find(|profile| profile.name == "10.0.0.2")
        .expect("literal hop profile");
    assert_eq!(literal.port, Some(2022));
    assert_eq!(literal.username.as_deref(), Some("ops"));
    assert_eq!(literal.group_id.as_deref(), Some(group.id.as_str()));
    assert_eq!(
        api.jump_profile_id.as_deref(),
        Some(literal.id.as_str()),
        "api tunnels through its one jump host"
    );
    assert_eq!(literal.jump_profile_id, None);

    // Importing again changes nothing: the alias and its jump host match.
    let again = ssh_config::import(&store, &entries, &["api".to_string()], None, &context)
        .expect("import again");
    assert_eq!(
        (again.added, again.updated, again.jump_hosts),
        (0, 1, 0),
        "api updated in place; its literal jump host already exists"
    );
    assert_eq!(store.list().len(), 2, "just api and its one jump host");
}

#[test]
fn ssh_config_import_drops_a_multi_hop_proxy_jump() {
    // web reaches bastion directly; db names a two-hop chain. A per-session
    // jump host cannot hold a chain, and writing db's chain onto bastion would
    // reroute web too, so db's ProxyJump is dropped rather than guessed at.
    let dir = temp_dir("ssh-multi-hop");
    let store = Store::load_from(dir.join("sessions.json"));
    let context = ssh_context(&dir);
    let text = "\
Host bastion
  HostName 10.0.0.1
  User gate
Host web
  HostName web.internal
  ProxyJump bastion
Host db
  HostName db.internal
  ProxyJump entry.example.com,bastion
";
    let entries = ssh_config::parse_text(text, &context);
    let summary = ssh_config::import(
        &store,
        &entries,
        &["web".to_string(), "db".to_string()],
        None,
        &context,
    )
    .expect("import");
    assert_eq!(
        (summary.added, summary.jump_hosts, summary.jumps_ignored),
        (3, 0, 1),
        "web, db and the pulled-in bastion added; db's multi-hop jump is skipped"
    );
    assert_eq!(
        summary.warnings.len(),
        1,
        "db's multi-hop jump warns once: {:?}",
        summary.warnings
    );

    let profiles = store.list();
    let by_name = |name: &str| {
        profiles
            .iter()
            .find(|profile| profile.name == name)
            .unwrap_or_else(|| panic!("profile {name} missing"))
    };
    assert!(
        !profiles.iter().any(|p| p.name == "entry.example.com"),
        "a dropped chain pulls in none of its hosts"
    );
    let bastion = by_name("bastion");
    assert_eq!(
        bastion.jump_profile_id, None,
        "bastion keeps its direct path"
    );
    assert_eq!(
        by_name("web").jump_profile_id.as_deref(),
        Some(bastion.id.as_str()),
        "web still reaches its single jump host bastion"
    );
    assert_eq!(
        by_name("db").jump_profile_id,
        None,
        "db is saved as a plain session; its chain was left off"
    );
}
