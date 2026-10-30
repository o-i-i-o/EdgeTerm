//! Credentials sealed at rest.

use super::*;

#[test]
fn credentials_are_sealed_on_disk_and_readable_after_restart() {
    let dir = temp_dir("vault");
    let path = dir.join("sessions.json");
    let credentials_path = dir.join("credentials.json");
    let read_credentials = || std::fs::read_to_string(&credentials_path).expect("credentials");

    let store = Store::load_from(path.clone());
    let saved = store
        .save(password_profile("10.0.0.1", "s3cret"))
        .expect("save");
    let sealed = read_credentials();
    assert!(
        !sealed.contains("s3cret"),
        "the secret must not be on disk in clear"
    );
    assert!(sealed.contains("\"encrypted\": true"));
    assert_eq!(
        stored_password(&store, &saved.id).as_deref(),
        Some("s3cret")
    );

    // The same machine and account open it again without any prompt.
    let reloaded = Store::load_from(path.clone());
    assert_eq!(
        stored_password(&reloaded, &saved.id).as_deref(),
        Some("s3cret")
    );
    // Every write uses a fresh nonce.
    reloaded
        .save(password_profile("10.0.0.2", "other"))
        .expect("save");
    assert_ne!(read_credentials(), sealed);

    // A file written before sealing existed is sealed on first load.
    std::fs::write(
        &credentials_path,
        format!("{{\"{}\": {{\"password\": \"legacy\"}}}}", saved.id),
    )
    .expect("write legacy file");
    let upgraded = Store::load_from(path.clone());
    assert_eq!(
        stored_password(&upgraded, &saved.id).as_deref(),
        Some("legacy")
    );
    assert!(!read_credentials().contains("legacy"));

    // A file from another machine (a different salt stands in for a
    // different machine id) yields no secrets, does not crash, and is
    // replaced by the next save.
    let mut envelope: serde_json::Value =
        serde_json::from_str(&read_credentials()).expect("envelope json");
    envelope["salt"] = serde_json::Value::String(B64.encode([7u8; 16]));
    std::fs::write(&credentials_path, envelope.to_string()).expect("write foreign file");
    let foreign = Store::load_from(path.clone());
    assert_eq!(stored_password(&foreign, &saved.id), None);
    let mut renewed = password_profile("10.0.0.1", "again");
    renewed.id = saved.id.clone();
    foreign.save(renewed).expect("save after foreign file");
    assert_eq!(
        stored_password(&Store::load_from(path), &saved.id).as_deref(),
        Some("again")
    );

    std::fs::remove_dir_all(&dir).ok();
}
