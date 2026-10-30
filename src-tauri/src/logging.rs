//! Where the process records what went wrong.
//!
//! Release builds are stripped, panic on abort, and the Windows binary has no
//! console, so `stderr` is nobody's record afterwards. This file survives: the
//! panic hook writes the message and location, `log_error!` mirrors what used
//! to go only to `eprintln!`. It sits beside the configuration (so portable
//! mode keeps its log with itself) and rotates at `MAX_BYTES`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::{SecondsFormat, Utc};
use parking_lot::Mutex;

/// Rotate once the log reaches this size; one `.1` file is kept beside it.
const MAX_BYTES: u64 = 1024 * 1024;

static LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();

/// Opens the log and installs the panic hook. Called once, first thing in
/// `run()`, so a failure anywhere after it leaves a trace.
pub fn install() {
    LOG.get_or_init(|| Mutex::new(open_log()));
    install_panic_hook();
}

/// Writes one line to the log, and to `stderr` so a terminal run shows the
/// same thing. Prefer the `log_error!` macro.
pub fn record(level: &str, message: std::fmt::Arguments<'_>) {
    use std::fmt::Write as _;

    let mut line = String::with_capacity(96);
    let _ = writeln!(line, "{} {level} {message}", timestamp());
    // A GUI build has no console; this is for the development one.
    eprint!("{line}");

    let file = LOG.get_or_init(|| Mutex::new(open_log()));
    // `try_lock`: a panic hook must not deadlock on a lock the panicking
    // thread already holds.
    if let Some(mut slot) = file.try_lock()
        && let Some(log) = slot.as_mut()
    {
        let _ = log.write_all(line.as_bytes());
        let _ = log.flush();
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        record("PANIC", format_args!("thread '{name}': {info}"));
        previous(info);
    }));
}

fn open_log() -> Option<File> {
    let dir = log_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("edgeterm.log");
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_BYTES) {
        let previous = dir.join("edgeterm.log.1");
        // Windows refuses to rename onto an existing file.
        let _ = std::fs::remove_file(&previous);
        let _ = std::fs::rename(&path, previous);
    }

    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path).ok()
}

fn log_dir() -> PathBuf {
    match crate::store::portable_data_dir() {
        Some(dir) => dir.join("logs"),
        None => dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("EdgeTerm")
            .join("logs"),
    }
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Mirrors a message to the log and to `stderr`; see `record`.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logging::record("ERROR", format_args!($($arg)*))
    };
}
