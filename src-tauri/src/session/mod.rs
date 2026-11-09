pub mod auth;
pub mod cwd;
pub mod encoding;
pub mod ftp;
pub mod host_keys;
pub mod local;
pub mod locale;
pub mod recording;
pub mod serial;
pub mod ssh;
pub mod telnet;
pub mod transfer;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use encoding_rs::Encoding;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use self::recording::Recorder;
use self::transfer::CancelFlag;
use crate::error::{AppError, Result};
use crate::model::{AuthPrompt, DirListing, FileEntry, SessionInfo, SessionKind, SessionProfile};

// --- the host boundary ------------------------------------------------------
//
// Everything below this line talks to the user through these two traits, never
// through a window toolkit. The Tauri implementation lives in `crate::host`;
// a different frontend only has to implement them.

/// Where session output, state changes and authentication challenges go.
///
/// Implementations must be cheap to call from a session's own thread: output
/// arrives in coalesced batches (see [`OutputPump`]) and a slow sink would
/// stall the peer's flow control.
pub trait EventSink: Send + Sync {
    /// Bytes read from the peer, in the order they arrived. Never empty.
    fn output(&self, id: &str, bytes: &[u8]);

    /// The session came up, or ended on its own: see [`emit_state`] for the
    /// close the frontend asked for itself, which is not reported.
    fn state(&self, id: &str, state: &str, message: Option<String>);

    /// Delivers one keyboard-interactive challenge. `false` means the sink
    /// cannot reach a user any more, so the connection has to give up.
    fn auth_prompt(&self, prompt: &AuthPrompt) -> bool;
}

/// Where a transfer's aggregate progress goes. One per `SftpRequest` that
/// copies bytes, made by the host from whatever its frontend speaks.
pub trait ProgressSink: Send + Sync {
    fn send(&self, progress: TransferProgress);
}

/// A sink for transfers nobody is watching: a background sync, or a test.
pub fn no_progress() -> Arc<dyn ProgressSink> {
    struct Discard;

    impl ProgressSink for Discard {
        fn send(&self, _: TransferProgress) {}
    }

    Arc::new(Discard)
}

/// Runs a detached task on the ambient tokio runtime.
///
/// The session core cannot name the host's executor, and a transport may be
/// dropped on a thread that has no runtime (the event loop, for instance), so
/// that case gets a one-worker fallback runtime of its own. The fallback is
/// built once and lives for the process; if it cannot be built, the task is
/// logged and dropped rather than silently ignored.
pub fn spawn_detached<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(future);
        return;
    }
    match fallback_runtime() {
        Ok(runtime) => {
            runtime.spawn(future);
        }
        Err(err) => crate::log_error!(
            "no async runtime is available and the session fallback runtime could not start: {err}"
        ),
    }
}

/// The process-wide fallback runtime, or the error that kept it from starting.
type FallbackRuntime =
    std::result::Result<&'static tokio::runtime::Runtime, &'static std::io::Error>;

fn fallback_runtime() -> FallbackRuntime {
    static FALLBACK: OnceLock<std::result::Result<tokio::runtime::Runtime, std::io::Error>> =
        OnceLock::new();
    FALLBACK
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("edgeterm-session")
                .build()
        })
        .as_ref()
}

pub enum SftpRequest {
    Home,
    List {
        path: String,
    },
    Canonicalize {
        path: String,
    },
    Mkdir {
        path: String,
    },
    /// Creates an empty file, refusing to touch one that already exists.
    CreateFile {
        path: String,
    },
    Remove {
        path: String,
        is_dir: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Download {
        remote: String,
        local: String,
        progress: Arc<dyn ProgressSink>,
        /// Raised by `cancel_transfer` and polled between chunks; a copy
        /// that stops on it leaves no half-written file behind.
        cancel: CancelFlag,
    },
    DownloadDirectory {
        remote: String,
        local: String,
        progress: Arc<dyn ProgressSink>,
        cancel: CancelFlag,
    },
    Upload {
        local: String,
        remote: String,
        progress: Arc<dyn ProgressSink>,
        cancel: CancelFlag,
    },
    UploadDirectory {
        local: String,
        remote: String,
        progress: Arc<dyn ProgressSink>,
        cancel: CancelFlag,
    },
}

#[derive(Debug)]
pub enum SftpResponse {
    Listing(DirListing),
    Path(String),
    Done,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub transferred: u64,
    pub total: u64,
}

pub enum SessionCommand {
    Write(Vec<u8>),
    /// Binary protocol data that needs transport-level backpressure. The
    /// owner resolves `reply` only after the bytes have actually been written
    /// to the PTY, SSH channel, or serial port.
    WriteConfirmed {
        data: Vec<u8>,
        reply: oneshot::Sender<Result<()>>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Sftp {
        request: SftpRequest,
        reply: oneshot::Sender<Result<SftpResponse>>,
    },
    /// The working directory of the shell's foreground job, for the Filer.
    /// Local sessions ask the OS, SSH sessions ask the server; see `cwd`.
    QueryCwd {
        reply: oneshot::Sender<Result<String>>,
    },
    Close,
}

pub struct SessionHandle {
    pub info: SessionInfo,
    pub tx: mpsc::Sender<SessionCommand>,
    /// What the session's bytes are in; keyboard text is encoded to it by
    /// `write_text` (see `encoding`).
    pub encoding: &'static Encoding,
    /// Serial sessions use a blocking owner thread. Joining it during close
    /// guarantees that the OS device handle is gone before the command returns.
    pub owner_thread: Option<std::thread::JoinHandle<()>>,
}

/// How many commands may wait for one session's owner thread. Deep enough for
/// a burst of keystrokes, shallow enough that a flood of pastes cannot pin
/// memory: the owner drains it only as fast as the peer accepts bytes.
pub const COMMAND_QUEUE: usize = 256;

/// One `write_text` call may not queue more than this. The paste dialog warns
/// far below it; the cap is about a runaway caller, not the user.
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionHandle>>,
}

impl SessionManager {
    pub fn insert(&self, handle: SessionHandle) {
        self.sessions.lock().insert(handle.info.id.clone(), handle);
    }

    pub fn remove(&self, id: &str) -> Option<SessionHandle> {
        self.sessions.lock().remove(id)
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions
            .lock()
            .values()
            .map(|h| h.info.clone())
            .collect()
    }

    pub fn send(&self, id: &str, cmd: SessionCommand) -> Result<()> {
        let guard = self.sessions.lock();
        Self::dispatch(Self::handle(&guard, id)?, cmd)
    }

    /// Keyboard and paste text, encoded the way the session expects it.
    pub fn write_text(&self, id: &str, text: &str) -> Result<()> {
        let guard = self.sessions.lock();
        let handle = Self::handle(&guard, id)?;
        let bytes = encoding::encode_input(handle.encoding, text);
        if bytes.len() > MAX_WRITE_BYTES {
            return Err(AppError::new(format!(
                "refusing to queue {} bytes of input at once",
                bytes.len()
            )));
        }
        Self::dispatch(handle, SessionCommand::Write(bytes))
    }

    fn handle<'a>(
        guard: &'a HashMap<String, SessionHandle>,
        id: &str,
    ) -> Result<&'a SessionHandle> {
        guard
            .get(id)
            .ok_or_else(|| AppError::new(format!("session {id} is not open")))
    }

    fn dispatch(handle: &SessionHandle, cmd: SessionCommand) -> Result<()> {
        // `try_send`, never a blocking send: `dispatch` is reached from
        // commands that run on the event loop's thread (a resize, a close),
        // where waiting for room would freeze the window.
        handle.tx.try_send(cmd).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => AppError::retry(format!(
                "session {} is busy; the command was not queued",
                handle.info.id
            )),
            mpsc::error::TrySendError::Closed(_) => {
                AppError::new(format!("session {} has terminated", handle.info.id))
            }
        })
    }

    /// Sends one bounded binary chunk and waits until the owning session has
    /// actually written it. Unlike the ordinary keyboard-input path, this
    /// applies backpressure so a large protocol transfer cannot accumulate in
    /// the command queue.
    pub async fn write_confirmed(&self, id: &str, data: Vec<u8>) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.send(id, SessionCommand::WriteConfirmed { data, reply })?;
        response
            .await
            .map_err(|_| AppError::new(format!("session {id} closed before writing binary data")))?
    }

    /// Round-trips an SFTP request through the owning session task.
    pub async fn sftp(&self, id: &str, request: SftpRequest) -> Result<SftpResponse> {
        let (reply, rx) = oneshot::channel();
        self.send(id, SessionCommand::Sftp { request, reply })?;
        rx.await
            .map_err(|_| AppError::new("session closed before the request completed"))?
    }

    /// Round-trips a working-directory query through the owning session.
    pub async fn query_cwd(&self, id: &str) -> Result<String> {
        let (reply, rx) = oneshot::channel();
        self.send(id, SessionCommand::QueryCwd { reply })?;
        rx.await
            .map_err(|_| AppError::new("session closed before the request completed"))?
    }
}

// --- events -----------------------------------------------------------------

/// Reports a change the session went through on its own: it came up, or the
/// peer / device / shell ended it. A close the frontend requested through
/// `close_session` is deliberately *not* reported: the frontend records that
/// state itself when the command returns, and it may reconnect under the
/// same session id right away, so a late "closed" from the old session would
/// otherwise be taken for the new one going down.
pub fn emit_state(sink: &dyn EventSink, id: &str, state: &str, message: Option<String>) {
    sink.state(id, state, message);
}

/// Coalesces small writes so a flood of output does not turn into a flood of
/// host messages. Callers must `flush` whenever their read loop goes idle.
/// Every byte pushed also goes to the session's recording, when the profile
/// asked for one (see `recording`); dropping the pump ends the recording.
pub struct OutputPump {
    sink: Arc<dyn EventSink>,
    id: String,
    buf: Vec<u8>,
    recorder: Option<Recorder>,
}

impl OutputPump {
    const MAX: usize = 64 * 1024;

    pub fn new(sink: Arc<dyn EventSink>, id: String, recorder: Option<Recorder>) -> Self {
        Self {
            sink,
            id,
            buf: Vec::with_capacity(8192),
            recorder,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if let Some(recorder) = &self.recorder {
            recorder.write(bytes);
        }
        self.buf.extend_from_slice(bytes);
        if self.buf.len() >= Self::MAX {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        self.sink.output(&self.id, &self.buf);
        self.buf.clear();
    }
}

// --- helpers shared by the backends ----------------------------------------

/// Answers the request-reply commands a backend cannot serve, so the caller
/// gets an error instead of a dropped reply channel.
pub fn reject_unsupported(cmd: SessionCommand, kind: SessionKind) {
    match cmd {
        SessionCommand::Sftp { reply, .. } => {
            let _ = reply.send(Err(AppError::new(format!(
                "{:?} sessions have no remote filesystem",
                kind
            ))));
        }
        SessionCommand::QueryCwd { reply } => {
            let _ = reply.send(Err(AppError::new(format!(
                "{:?} sessions have no shell working directory",
                kind
            ))));
        }
        SessionCommand::WriteConfirmed { reply, .. } => {
            let _ = reply.send(Err(AppError::new(format!(
                "{:?} session does not accept binary terminal data",
                kind
            ))));
        }
        _ => {}
    }
}

/// Normalises a possibly-relative remote path into an absolute one.
pub fn join_remote(base: &str, name: &str) -> String {
    if name.starts_with('/') {
        return name.to_string();
    }
    if base.is_empty() || base == "/" {
        format!("/{name}")
    } else if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

pub fn make_info(id: &str, profile: &SessionProfile) -> SessionInfo {
    SessionInfo {
        id: id.to_string(),
        profile_id: if profile.id.is_empty() {
            None
        } else {
            Some(profile.id.clone())
        },
        name: profile.name.clone(),
        kind: profile.kind,
        protocol: profile.protocol().to_string(),
        address: profile.address(),
        color: profile.color.clone(),
        supports_remote_files: matches!(
            profile.kind,
            SessionKind::Ssh | SessionKind::Ftp | SessionKind::Sftp
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn confirmed_binary_write_waits_for_the_session_owner() {
        let manager = Arc::new(SessionManager::default());
        let (tx, mut rx) = mpsc::channel(COMMAND_QUEUE);
        manager.insert(SessionHandle {
            info: SessionInfo {
                id: "confirmed-write".into(),
                profile_id: None,
                name: "test".into(),
                kind: SessionKind::Ssh,
                protocol: "ssh".into(),
                address: "example.test:22".into(),
                color: None,
                supports_remote_files: true,
            },
            tx,
            encoding: encoding_rs::UTF_8,
            owner_thread: None,
        });

        let write_task = tokio::spawn({
            let manager = manager.clone();
            async move {
                manager
                    .write_confirmed("confirmed-write", vec![1, 2, 3, 4])
                    .await
            }
        });

        let command = rx.recv().await.expect("write command");
        let SessionCommand::WriteConfirmed { data, reply } = command else {
            panic!("expected a confirmed write command");
        };
        assert_eq!(data, vec![1, 2, 3, 4]);
        assert!(
            !write_task.is_finished(),
            "the caller must remain blocked until the owner finishes writing"
        );

        reply.send(Ok(())).expect("acknowledge write");
        write_task
            .await
            .expect("write task should not panic")
            .expect("confirmed write should succeed");
    }

    #[tokio::test]
    async fn text_is_encoded_for_the_session_it_goes_to() {
        let manager = SessionManager::default();
        let (tx, mut rx) = mpsc::channel(COMMAND_QUEUE);
        manager.insert(SessionHandle {
            info: SessionInfo {
                id: "gbk".into(),
                profile_id: None,
                name: "test".into(),
                kind: SessionKind::Ssh,
                protocol: "ssh".into(),
                address: "example.test:22".into(),
                color: None,
                supports_remote_files: true,
            },
            tx,
            encoding: encoding_rs::GBK,
            owner_thread: None,
        });

        manager.write_text("gbk", "ls 你好\r").expect("write");
        let SessionCommand::Write(bytes) = rx.recv().await.expect("write command") else {
            panic!("expected a plain write");
        };
        assert_eq!(bytes, b"ls \xc4\xe3\xba\xc3\r".to_vec());

        assert!(manager.write_text("missing", "x").is_err());
    }

    /// An `EventSink` that keeps every batch it was handed, so a test can see
    /// exactly what a pump sent and how often.
    #[derive(Default)]
    struct RecordingSink {
        batches: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl RecordingSink {
        fn batches(&self) -> Vec<(String, Vec<u8>)> {
            self.batches.lock().clone()
        }
    }

    impl EventSink for RecordingSink {
        fn output(&self, id: &str, bytes: &[u8]) {
            self.batches.lock().push((id.to_string(), bytes.to_vec()));
        }

        fn state(&self, _: &str, _: &str, _: Option<String>) {}

        fn auth_prompt(&self, _: &AuthPrompt) -> bool {
            true
        }
    }

    #[test]
    fn output_goes_to_the_host_when_the_pump_flushes() {
        let sink = Arc::new(RecordingSink::default());
        let mut pump = OutputPump::new(sink.clone(), "s1".into(), None);

        pump.push(b"hello ");
        pump.push(b"world");
        assert!(
            sink.batches().is_empty(),
            "nothing may go out before a flush"
        );

        pump.flush();
        assert_eq!(
            sink.batches(),
            vec![("s1".to_string(), b"hello world".to_vec())]
        );
    }

    #[test]
    fn flushing_an_empty_pump_sends_nothing() {
        let sink = Arc::new(RecordingSink::default());
        let mut pump = OutputPump::new(sink.clone(), "s1".into(), None);

        pump.flush();
        assert!(sink.batches().is_empty());

        pump.push(b"x");
        pump.flush();
        pump.flush();
        assert_eq!(sink.batches(), vec![("s1".to_string(), b"x".to_vec())]);
    }

    #[test]
    fn reaching_the_limit_flushes_without_waiting_for_the_read_loop() {
        let sink = Arc::new(RecordingSink::default());
        let mut pump = OutputPump::new(sink.clone(), "s1".into(), None);

        pump.push(&vec![7_u8; OutputPump::MAX - 1]);
        assert!(sink.batches().is_empty(), "one byte short of the limit");

        pump.push(b"x");
        let batches = sink.batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].1.len(), OutputPump::MAX);

        pump.flush();
        assert_eq!(sink.batches().len(), 1, "the buffer was emptied");
    }

    #[test]
    fn a_recorded_session_gets_the_same_bytes_as_the_host() {
        let dir = std::env::temp_dir().join(format!("edgeterm-pump-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("scratch folder");

        let mut profile = crate::tests::profile(SessionKind::Local);
        profile.name = "pump".into();
        profile.record = true;
        profile.record_dir = Some(dir.display().to_string());
        let recorder = Recorder::start("pump", &profile).expect("start recording");

        let sink = Arc::new(RecordingSink::default());
        let mut pump = OutputPump::new(sink.clone(), "pump".into(), Some(recorder));
        pump.push(b"hello ");
        pump.push(b"world");
        pump.flush();
        assert_eq!(
            sink.batches(),
            vec![("pump".to_string(), b"hello world".to_vec())]
        );

        // Dropping the pump drops the recorder, whose writer thread then
        // closes the file with its trailer.
        drop(pump);
        let path = std::fs::read_dir(&dir)
            .expect("recording folder")
            .map(|entry| entry.expect("recording entry").path())
            .next()
            .expect("one recording");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let contents = std::fs::read(&path).unwrap_or_default();
            if contents.ends_with(b"====\r\n") {
                let text = String::from_utf8_lossy(&contents);
                assert!(text.contains("\r\nhello world"), "recording: {text}");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the recording was never closed"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_detached_task_runs_on_the_ambient_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let (tx, rx) = oneshot::channel();

        runtime.block_on(async move {
            spawn_detached(async move {
                let _ = tx.send(());
            });
            rx.await.expect("the detached task never ran");
        });
    }
}
