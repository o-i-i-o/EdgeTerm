//! Tests that reach beyond one module: the data file, the filesystem, the
//! OpenSSH import. The helpers below are shared with the unit tests in
//! `session` and friends.

use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

use crate::commands::data::read_app_data;
use crate::fs_local;
use crate::model::{
    APP_DATA_APP, APP_DATA_EXTENSION, APP_DATA_FORMAT, AppData, AuthKind, CommandScope, FileEntry,
    LineEnding, SavedCommand, SessionGroup, SessionKind, SessionProfile, Theme, split_command_line,
};
use crate::remote_edit;
use crate::session::{TransferProgress, join_remote, sort_entries};
use crate::ssh_config::{self, SshConfigEntry};
use crate::store::{
    Store, WindowGeometry, is_data_file_path, portable_data_dir_in, save_startup_theme_at,
    save_window_geometry_at, startup_theme_at, startup_window_at,
};
use crate::transfer_access::TransferAccess;
use crate::transfer_io;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("edgeterm-test-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A progress channel for tests that only care about the result.
fn silent_progress() -> tauri::ipc::Channel<TransferProgress> {
    tauri::ipc::Channel::new(|_| Ok(()))
}

pub(crate) fn profile(kind: SessionKind) -> SessionProfile {
    SessionProfile {
        id: String::new(),
        name: "test".into(),
        kind,
        color: None,
        group_id: None,
        encoding: None,
        locale: None,
        backspace: None,
        confirm_close: None,
        legacy_compat: false,
        legacy_algorithms: false,
        lenient_key_encoding: false,
        shell: None,
        cwd: None,
        host: None,
        port: None,
        username: None,
        auth: None,
        password: None,
        private_key_path: None,
        passphrase: None,
        jump_profile_id: None,
        port_name: None,
        baud_rate: None,
        data_bits: None,
        stop_bits: None,
        parity: None,
        flow_control: None,
    }
}

fn group(name: &str, kind: SessionKind, parent_id: Option<&str>) -> SessionGroup {
    SessionGroup {
        id: String::new(),
        name: name.into(),
        kind,
        parent_id: parent_id.map(str::to_string),
    }
}

fn entry(name: &str, is_dir: bool) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path: format!("/{name}"),
        is_dir,
        is_symlink: false,
        size: 0,
        modified: None,
        permissions: None,
        owner: None,
        group: None,
    }
}

fn command(name: &str, text: &str) -> SavedCommand {
    SavedCommand {
        id: String::new(),
        name: name.into(),
        text: text.into(),
        ending: LineEnding::Lf,
        scope: CommandScope::Global,
        color: None,
    }
}

fn scoped(name: &str, scope: CommandScope) -> SavedCommand {
    SavedCommand {
        scope,
        ..command(name, "ls")
    }
}

fn password_profile(host: &str, password: &str) -> SessionProfile {
    let mut p = profile(SessionKind::Ssh);
    p.name = host.into();
    p.host = Some(host.into());
    p.username = Some("root".into());
    p.auth = Some(AuthKind::Password);
    p.password = Some(password.into());
    p
}

fn stored_password(store: &Store, id: &str) -> Option<String> {
    store.get(id).expect("get").and_then(|p| p.password)
}

fn ssh_context(dir: &Path) -> ssh_config::Context {
    ssh_config::Context {
        home: Some(dir.to_path_buf()),
        ssh_dir: dir.join(".ssh"),
        local_user: "me".into(),
        default_identity: None,
    }
}

fn entry_named<'a>(entries: &'a [SshConfigEntry], alias: &str) -> &'a SshConfigEntry {
    entries
        .iter()
        .find(|entry| entry.alias == alias)
        .unwrap_or_else(|| panic!("alias {alias} was not parsed"))
}

mod files;
mod groups;
mod listing;
mod profiles;
mod remote_edits;
mod secrets;
mod sender;
mod settings;
mod ssh_import;
mod store;
