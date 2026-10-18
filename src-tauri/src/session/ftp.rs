use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant, SystemTime};

use suppaftp::list::{File as FtpFile, ListParser};
use suppaftp::types::FileType;
use suppaftp::{Encoding, FtpError, FtpStream, Mode, Status};
use tauri::AppHandle;
use tokio::sync::mpsc::UnboundedReceiver;

use super::transfer::{
    CancelFlag, ProgressReporter, ensure_local_directory, plan_local_upload, safe_local_child,
    validate_local_file_target,
};
use super::{
    SessionCommand, SftpRequest, SftpResponse, TransferProgress, emit_state, join_remote,
    sort_entries,
};
use crate::error::{AppError, Result, err};
use crate::model::{DirListing, FileEntry, SessionProfile};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_secs(60);
const TRANSFER_PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
const TRANSFER_CHUNK_SIZE: usize = 1024 * 1024;
const ENCODING_UTF8: u8 = 1;
const ENCODING_GBK: u8 = 2;

pub struct FtpConnection {
    stream: FtpStream,
    home: String,
    /// Cleared once the server refuses MLSD as unknown or unimplemented, so
    /// later listings go straight to LIST instead of paying a passive data
    /// connection and a command round trip for a guaranteed failure.
    mlsd_supported: bool,
}

/// Establishes a standard, passive-mode FTP connection. It runs on a blocking
/// worker before the session owner thread is started so DNS, the welcome
/// response, and authentication never block Tauri's async runtime.
pub fn connect(profile: &SessionProfile) -> Result<FtpConnection> {
    let host = profile
        .host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .ok_or_else(|| AppError::new("ftp session is missing a host"))?
        .to_string();
    let port = profile.port.unwrap_or(21);
    let username = profile
        .username
        .as_deref()
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .unwrap_or("anonymous")
        .to_string();
    let password = profile
        .password
        .clone()
        .filter(|password| !password.is_empty())
        .unwrap_or_else(|| {
            if username.eq_ignore_ascii_case("anonymous") {
                "anonymous@".to_string()
            } else {
                String::new()
            }
        });

    let remote_control = connect_control(&host, port)?;
    let use_extended_passive = remote_control.peer_addr()?.is_ipv6();
    let mut stream = FtpStream::connect_with_stream(remote_control).map_err(|error| {
        AppError::new(format!(
            "ftp server did not send a welcome response: {error}"
        ))
    })?;
    // SuppaFTP encodes commands and decodes listings, PWD/MLST replies and the
    // welcome banner with this encoding. The server's encoding starts unknown
    // and is latched by its first non-ASCII bytes (see `FtpEncoding`).
    stream.set_encoding(FtpEncoding::default());
    let mut stream = stream.passive_stream_builder(connect_data);
    if use_extended_passive {
        stream.set_mode(Mode::ExtendedPassive);
    }
    // Many servers behind NAT advertise an unroutable private address in PASV;
    // SuppaFTP's workaround replaces it with the control peer, which is the
    // real server now that no proxy sits in between.
    stream.set_passive_nat_workaround(true);
    stream.login(&username, &password).map_err(|error| {
        AppError::new(format!("ftp authentication failed for {username}: {error}"))
    })?;
    stream.transfer_type(FileType::Binary).map_err(|error| {
        AppError::new(format!("ftp server refused binary transfer mode: {error}"))
    })?;
    let home = stream.pwd().map_err(|error| {
        AppError::new(format!(
            "ftp server did not report a working directory: {error}"
        ))
    })?;

    Ok(FtpConnection {
        stream,
        home,
        mlsd_supported: true,
    })
}

fn connect_control(host: &str, port: u16) -> Result<TcpStream> {
    let addresses: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
    if addresses.is_empty() {
        return Err(AppError::new(format!(
            "ftp host {host} did not resolve to an address"
        )));
    }

    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                configure_socket(&stream)?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(AppError::new(format!(
        "cannot connect to ftp server {host}:{port}: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown connection error".to_string())
    )))
}

fn connect_data(address: SocketAddr) -> std::result::Result<TcpStream, FtpError> {
    let stream =
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(FtpError::ConnectionError)?;
    configure_socket(&stream).map_err(FtpError::ConnectionError)?;
    Ok(stream)
}

/// Encoding for commands, listings and text replies. The server's encoding is
/// unknown until the first non-ASCII bytes arrive: valid UTF-8 selects UTF-8,
/// anything else selects GBK, and the choice is sticky for the session.
#[derive(Debug, Default)]
struct FtpEncoding {
    encoding: AtomicU8,
}

impl Encoding for FtpEncoding {
    fn decode(&self, bytes: &[u8]) -> String {
        match self.encoding.load(Ordering::Acquire) {
            ENCODING_GBK => return encoding_rs::GBK.decode(bytes).0.into_owned(),
            ENCODING_UTF8 => return String::from_utf8_lossy(bytes).into_owned(),
            _ => {}
        }

        if std::str::from_utf8(bytes).is_ok() {
            if bytes.iter().any(|byte| !byte.is_ascii()) {
                self.encoding.store(ENCODING_UTF8, Ordering::Release);
            }
            String::from_utf8_lossy(bytes).into_owned()
        } else {
            self.encoding.store(ENCODING_GBK, Ordering::Release);
            encoding_rs::GBK.decode(bytes).0.into_owned()
        }
    }

    fn encode(&self, text: &str) -> std::io::Result<Vec<u8>> {
        if self.encoding.load(Ordering::Acquire) != ENCODING_GBK {
            return Ok(text.as_bytes().to_vec());
        }
        Ok(encoding_rs::GBK.encode(text).0.into_owned())
    }
}

fn configure_socket(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_nodelay(true)?;
    Ok(())
}

/// FTP has no interactive terminal. The single owner thread serializes all
/// control-channel and data-channel commands for the dedicated FTP file pane.
pub fn spawn(
    app: AppHandle,
    id: String,
    mut connection: FtpConnection,
    mut rx: UnboundedReceiver<SessionCommand>,
) -> Result<()> {
    std::thread::Builder::new()
        .name(format!("edgeterm-ftp-{id}"))
        .spawn(move || {
            // `open_session` marks the optimistic tab connected only after the
            // manager has inserted its command handle. Emitting that state
            // from this immediately-started thread would let Filer race the
            // insertion and fail its first Home request.
            let mut close_requested = false;
            while let Some(command) = rx.blocking_recv() {
                match command {
                    SessionCommand::Sftp { request, reply } => {
                        let _ = reply.send(run_ftp(&mut connection, request));
                    }
                    SessionCommand::Close => {
                        close_requested = true;
                        break;
                    }
                    // Standard FTP is a file protocol, not an interactive
                    // terminal, so keyboard input and resize events do not map
                    // to protocol commands.
                    SessionCommand::Write(_) | SessionCommand::Resize { .. } => {}
                    other => {
                        super::reject_unsupported(other, crate::model::SessionKind::Ftp);
                    }
                }
            }

            let _ = connection.stream.quit();
            // The frontend already knows about a close it asked for; see
            // `emit_state`. The channel closing without a Close command means
            // the whole manager is gone, and nobody is listening either.
            if !close_requested {
                emit_state(&app, &id, "closed", None);
            }
        })
        .map_err(err)?;
    Ok(())
}

fn run_ftp(connection: &mut FtpConnection, request: SftpRequest) -> Result<SftpResponse> {
    match request {
        SftpRequest::Home => Ok(SftpResponse::Path(connection.home.clone())),
        SftpRequest::Canonicalize { path } => Ok(SftpResponse::Path(canonicalize(
            &mut connection.stream,
            &path,
        )?)),
        SftpRequest::List { path } => list_directory(connection, &path).map(SftpResponse::Listing),
        SftpRequest::Mkdir { path } => {
            connection.stream.mkdir(path)?;
            Ok(SftpResponse::Done)
        }
        SftpRequest::CreateFile { path } => {
            // FTP has no exclusive create; STOR silently truncates, so probe
            // first. Servers without SIZE answer with an error, which we treat
            // as "not there" — the same trade-off a plain upload makes.
            if connection.stream.size(&path).is_ok() {
                return Err(AppError::new(format!("{path} already exists")));
            }
            connection.stream.put_file(&path, &mut std::io::empty())?;
            Ok(SftpResponse::Done)
        }
        SftpRequest::Remove { path, is_dir } => {
            if is_dir {
                connection.stream.rmdir(path)?;
            } else {
                connection.stream.rm(path)?;
            }
            Ok(SftpResponse::Done)
        }
        SftpRequest::Rename { from, to } => {
            connection.stream.rename(from, to)?;
            Ok(SftpResponse::Done)
        }
        SftpRequest::Download {
            remote,
            local,
            progress,
            cancel,
        } => {
            download_file(&mut connection.stream, &remote, &local, &progress, &cancel)?;
            Ok(SftpResponse::Done)
        }
        SftpRequest::DownloadDirectory {
            remote,
            local,
            progress,
            cancel,
        } => {
            download_directory(connection, &remote, &local, &progress, &cancel)?;
            Ok(SftpResponse::Done)
        }
        SftpRequest::Upload {
            local,
            remote,
            progress,
            cancel,
        } => {
            upload_file(&mut connection.stream, &local, &remote, &progress, &cancel)?;
            Ok(SftpResponse::Done)
        }
        SftpRequest::UploadDirectory {
            local,
            remote,
            progress,
            cancel,
        } => {
            upload_directory(connection, &local, &remote, &progress, &cancel)?;
            Ok(SftpResponse::Done)
        }
    }
}

fn canonicalize(ftp: &mut FtpStream, path: &str) -> Result<String> {
    let original = ftp.pwd()?;
    ftp.cwd(if path.is_empty() { "." } else { path })?;
    let resolved = ftp.pwd();
    let restore = ftp.cwd(&original);
    match (resolved, restore) {
        (Ok(path), Ok(())) => Ok(path),
        (Err(error), _) | (_, Err(error)) => Err(error.into()),
    }
}

fn list_directory(connection: &mut FtpConnection, path: &str) -> Result<DirListing> {
    connection
        .stream
        .cwd(if path.is_empty() { "." } else { path })?;
    let base = connection.stream.pwd()?;

    let mlsd_entries = if connection.mlsd_supported {
        match connection.stream.mlsd(None) {
            Ok(lines) => parse_listing(&base, &lines, ListParser::parse_mlsd),
            Err(error) => {
                if mlsd_unsupported(&error) {
                    connection.mlsd_supported = false;
                }
                None
            }
        }
    } else {
        None
    };
    let list_entries = if mlsd_entries.is_none() {
        connection
            .stream
            .list(None)
            .ok()
            .and_then(|lines| parse_listing(&base, &lines, |line| FtpFile::try_from(line)))
    } else {
        None
    };
    let entries = match mlsd_entries.or(list_entries) {
        Some(entries) => entries,
        None => list_names(connection, &base)?,
    };

    let mut entries = entries;
    sort_entries(&mut entries);
    Ok(DirListing {
        path: base,
        entries,
    })
}

/// A 500 / 502 reply means the server does not implement MLSD at all (vsftpd,
/// for one, answers `500 Unknown command`). Anything else — a dropped data
/// connection, a timeout, a per-directory refusal — may be transient, so the
/// next listing tries MLSD again.
fn mlsd_unsupported(error: &FtpError) -> bool {
    matches!(
        error,
        FtpError::UnexpectedResponse(response)
            if matches!(
                response.status,
                Status::BadCommand | Status::NotImplemented | Status::CommandNotImplemented
            )
    )
}

fn parse_listing<F>(base: &str, lines: &[String], mut parse: F) -> Option<Vec<FileEntry>>
where
    F: FnMut(&str) -> std::result::Result<FtpFile, suppaftp::list::ParseError>,
{
    let relevant: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.starts_with("total ")
                && !lower.contains("type=cdir;")
                && !lower.contains("type=pdir;")
        })
        .collect();

    let mut entries = Vec::with_capacity(relevant.len());
    for line in relevant {
        let file = parse(line).ok()?;
        if let Some(entry) = entry_from_file(base, &file) {
            entries.push(entry);
        }
    }
    Some(entries)
}

fn entry_from_file(base: &str, file: &FtpFile) -> Option<FileEntry> {
    let name = file.name();
    if name.is_empty() || matches!(name, "." | "..") {
        return None;
    }

    Some(FileEntry {
        name: name.to_string(),
        path: join_remote(base, name),
        is_dir: file.is_directory(),
        is_symlink: file.is_symlink(),
        size: file.size() as u64,
        modified: system_time_to_unix(file.modified()),
        // LIST/MLSD permissions are optional and server-specific. SuppaFTP's
        // parser fills defaults when facts are absent, so reporting them here
        // would make guessed permissions look authoritative.
        permissions: None,
        owner: file.uid().map(|uid| uid.to_string()),
        group: file.gid().map(|gid| gid.to_string()),
    })
}

/// Last-resort listing for servers without MLSD and with a nonstandard LIST
/// format. NLST supplies names; CWD probes distinguish directories from files.
fn list_names(connection: &mut FtpConnection, base: &str) -> Result<Vec<FileEntry>> {
    let names = connection.stream.nlst(None)?;
    let mut entries = Vec::with_capacity(names.len());
    for raw_name in names {
        let trimmed = raw_name.trim_end_matches('/');
        let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
        if name.is_empty() || matches!(name, "." | "..") {
            continue;
        }
        let path = if trimmed.starts_with('/') {
            trimmed.to_string()
        } else {
            join_remote(base, name)
        };
        let is_dir = connection.stream.cwd(&path).is_ok();
        if is_dir {
            connection.stream.cwd(base)?;
        }
        let size = if is_dir {
            0
        } else {
            connection.stream.size(&path).unwrap_or(0) as u64
        };
        let modified = connection
            .stream
            .mdtm(&path)
            .ok()
            .map(|value| value.and_utc().timestamp());
        entries.push(FileEntry {
            name: name.to_string(),
            path,
            is_dir,
            is_symlink: false,
            size,
            modified,
            permissions: None,
            owner: None,
            group: None,
        });
    }
    Ok(entries)
}

fn system_time_to_unix(time: SystemTime) -> Option<i64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

fn download_file(
    ftp: &mut FtpStream,
    remote: &str,
    local: &str,
    progress: &tauri::ipc::Channel<TransferProgress>,
    cancel: &CancelFlag,
) -> Result<()> {
    let total = ftp.size(remote).unwrap_or(0) as u64;
    let mut last_report = Instant::now();

    report_transfer_progress(progress, 0, total);
    let transferred = copy_remote_file(ftp, remote, Path::new(local), cancel, |transferred| {
        if last_report.elapsed() >= TRANSFER_PROGRESS_INTERVAL {
            report_transfer_progress(progress, transferred, total);
            last_report = Instant::now();
        }
    })?;
    report_transfer_progress(progress, transferred, total);
    Ok(())
}

struct FtpDownloadJob {
    remote: String,
    local: PathBuf,
}

fn download_directory(
    connection: &mut FtpConnection,
    remote: &str,
    local: &str,
    progress: &tauri::ipc::Channel<TransferProgress>,
    cancel: &CancelFlag,
) -> Result<()> {
    let local_root = PathBuf::from(local);
    let mut pending = vec![(remote.to_string(), local_root.clone())];
    let mut directories = vec![local_root];
    let mut files = Vec::new();
    let mut total = 0_u64;

    // Build a metadata-only plan first so progress covers the whole tree. File
    // contents are still copied one at a time through the fixed 1 MiB buffer.
    while let Some((remote_dir, local_dir)) = pending.pop() {
        cancel.check()?;
        let listing = list_directory(connection, &remote_dir)?;
        for entry in listing.entries {
            let local_child = safe_local_child(&local_dir, &entry.name)?;
            if entry.is_dir && !entry.is_symlink {
                directories.push(local_child.clone());
                pending.push((entry.path, local_child));
                continue;
            }
            // LIST/MLSD report a symlink without the type of its target. RETR
            // on a linked directory would abort the whole download, so probe
            // with CWD and skip directory links; following them could loop
            // back into an ancestor forever.
            if entry.is_symlink && is_remote_directory(&mut connection.stream, &entry.path) {
                continue;
            }
            total = total.checked_add(entry.size).ok_or_else(|| {
                AppError::new("FTP folder is too large to report transfer progress")
            })?;
            files.push(FtpDownloadJob {
                remote: entry.path,
                local: local_child,
            });
        }
    }

    for directory in &directories {
        ensure_local_directory(directory)?;
    }
    for file in &files {
        validate_local_file_target(&file.local)?;
    }

    let mut reporter = ProgressReporter::begin(progress, total);
    let mut transferred = 0_u64;
    for file in files {
        let completed_before_file = transferred;
        let copied = copy_remote_file(
            &mut connection.stream,
            &file.remote,
            &file.local,
            cancel,
            |file_transferred| {
                reporter.update(completed_before_file.saturating_add(file_transferred));
            },
        )?;
        transferred = transferred.saturating_add(copied);
    }
    reporter.finish(transferred);
    Ok(())
}

fn is_remote_directory(ftp: &mut FtpStream, path: &str) -> bool {
    let Ok(original) = ftp.pwd() else {
        return false;
    };
    if ftp.cwd(path).is_err() {
        return false;
    }
    let _ = ftp.cwd(&original);
    true
}

fn copy_remote_file<F>(
    ftp: &mut FtpStream,
    remote: &str,
    local: &Path,
    cancel: &CancelFlag,
    on_chunk: F,
) -> Result<u64>
where
    F: FnMut(u64),
{
    validate_local_file_target(local)?;
    let mut source = ftp.retr_as_stream(remote)?;
    let mut target = std::fs::File::create(local)?;
    let copied = copy_in_chunks(&mut source, &mut target, cancel, on_chunk);
    // Finishing closes the data connection and reads the server's completion
    // reply, which reports an aborted transfer when the data connection
    // closes half-read; a cancelled copy has no use for that reply.
    let finalized = source.finish();

    let transferred = match copied {
        Ok(transferred) => transferred,
        Err(error) => {
            // A cancelled copy leaves nothing behind: the truncated file
            // would pass for the real one.
            if cancel.is_cancelled() {
                drop(target);
                let _ = std::fs::remove_file(local);
            }
            return Err(error.into());
        }
    };
    finalized?;
    target.flush()?;
    Ok(transferred)
}

fn upload_file(
    ftp: &mut FtpStream,
    local: &str,
    remote: &str,
    progress: &tauri::ipc::Channel<TransferProgress>,
    cancel: &CancelFlag,
) -> Result<()> {
    let metadata = std::fs::metadata(local)?;
    if !metadata.is_file() {
        return Err(AppError::new("only regular files can be uploaded"));
    }
    let total = metadata.len();
    let mut last_report = Instant::now();

    report_transfer_progress(progress, 0, total);
    let transferred = copy_local_file(ftp, Path::new(local), remote, cancel, |transferred| {
        if last_report.elapsed() >= TRANSFER_PROGRESS_INTERVAL {
            report_transfer_progress(progress, transferred, total);
            last_report = Instant::now();
        }
    })?;
    report_transfer_progress(progress, transferred, total);
    Ok(())
}

fn upload_directory(
    connection: &mut FtpConnection,
    local: &str,
    remote: &str,
    progress: &tauri::ipc::Channel<TransferProgress>,
    cancel: &CancelFlag,
) -> Result<()> {
    // Same shape as the download: a metadata-only plan first so progress
    // covers the whole tree, then one file at a time through the fixed buffer.
    let plan = plan_local_upload(Path::new(local), remote)?;
    for directory in &plan.directories {
        cancel.check()?;
        ensure_remote_directory(&mut connection.stream, directory)?;
    }

    let mut reporter = ProgressReporter::begin(progress, plan.total);
    let mut transferred = 0_u64;
    for file in plan.files {
        let completed_before_file = transferred;
        let copied = copy_local_file(
            &mut connection.stream,
            &file.local,
            &file.remote,
            cancel,
            |file_transferred| {
                reporter.update(completed_before_file.saturating_add(file_transferred));
            },
        )?;
        transferred = transferred.saturating_add(copied);
    }
    reporter.finish(transferred);
    Ok(())
}

/// MKD fails when the folder already exists, which is expected when merging
/// into a remote tree, so an existing directory is accepted after a CWD probe.
fn ensure_remote_directory(ftp: &mut FtpStream, path: &str) -> Result<()> {
    match ftp.mkdir(path) {
        Ok(()) => Ok(()),
        Err(_) if is_remote_directory(ftp, path) => Ok(()),
        Err(error) => Err(AppError::new(format!(
            "cannot create remote folder {path}: {error}"
        ))),
    }
}

fn copy_local_file<F>(
    ftp: &mut FtpStream,
    local: &Path,
    remote: &str,
    cancel: &CancelFlag,
    on_chunk: F,
) -> Result<u64>
where
    F: FnMut(u64),
{
    let mut source = std::fs::File::open(local)?;
    let mut target = ftp.put_with_stream(remote)?;
    let copied = copy_in_chunks(&mut source, &mut target, cancel, on_chunk);
    let flushed = target.flush();
    // Finishing closes the data connection and reads the server's completion
    // reply; a cancelled upload discards it below, after removing the remote
    // half-written file.
    let finalized = target.finish();

    let transferred = match copied {
        Ok(transferred) => transferred,
        Err(error) => {
            // A cancelled upload takes its half-written file off the server
            // too, now that the data connection is closed.
            if cancel.is_cancelled() {
                let _ = ftp.rm(remote);
            }
            return Err(error.into());
        }
    };
    flushed?;
    finalized?;
    Ok(transferred)
}

/// Streams bytes through a fixed-size buffer; stops between chunks once
/// `cancel` is raised, with the `Interrupted` error `CancelFlag::check_io`
/// describes.
fn copy_in_chunks<R, W, F>(
    source: &mut R,
    target: &mut W,
    cancel: &CancelFlag,
    mut on_chunk: F,
) -> std::io::Result<u64>
where
    R: Read,
    W: Write,
    F: FnMut(u64),
{
    let mut buffer = vec![0; TRANSFER_CHUNK_SIZE];
    let mut transferred = 0;
    loop {
        cancel.check_io()?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            return Ok(transferred);
        }
        target.write_all(&buffer[..count])?;
        transferred += count as u64;
        on_chunk(transferred);
    }
}

fn report_transfer_progress(
    progress: &tauri::ipc::Channel<TransferProgress>,
    transferred: u64,
    total: u64,
) {
    let _ = progress.send(TransferProgress { transferred, total });
}

#[cfg(test)]
mod tests {
    use super::{
        CancelFlag, FtpEncoding, TRANSFER_CHUNK_SIZE, copy_in_chunks, entry_from_file,
        mlsd_unsupported, parse_listing,
    };
    use suppaftp::list::File as FtpFile;
    use suppaftp::list::ListParser;
    use suppaftp::types::Response;
    use suppaftp::{Encoding, FtpError, Status};

    #[test]
    fn parses_mlsd_entries_into_remote_listing() {
        let lines = vec![
            "type=cdir;modify=20260812120000; .".to_string(),
            "type=dir;modify=20260810112233; projects".to_string(),
            "type=file;size=42;modify=20260811123456; notes.txt".to_string(),
        ];
        let entries = parse_listing("/home/test", &lines, ListParser::parse_mlsd)
            .expect("valid MLSD listing");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "/home/test/projects");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].path, "/home/test/notes.txt");
        assert_eq!(entries[1].size, 42);
    }

    #[test]
    fn parses_posix_ftp_list_entry() {
        let file = FtpFile::try_from("-rw-r--r-- 1 1000 1000 8192 Nov 5 2018 archive file.txt")
            .expect("POSIX LIST entry");
        let entry = entry_from_file("/pub", &file).expect("visible file");

        assert_eq!(entry.name, "archive file.txt");
        assert_eq!(entry.path, "/pub/archive file.txt");
        assert_eq!(entry.size, 8192);
        assert!(!entry.is_dir);
    }

    #[test]
    fn directory_decoder_preserves_utf8_and_converts_gbk() {
        let utf8 = FtpEncoding::default();
        let text = "资料/测试.txt";
        assert_eq!(utf8.decode(text.as_bytes()), text);
        assert_eq!(utf8.encode(text).unwrap(), text.as_bytes());

        // GBK bytes for “资料/测试.txt”; the first invalid UTF-8 latch GBK.
        let gbk = FtpEncoding::default();
        assert_eq!(
            gbk.decode(b"\xd7\xca\xc1\xcf/\xb2\xe2\xca\xd4.txt"),
            "资料/测试.txt"
        );
        assert_eq!(
            gbk.encode("CWD 资料\r\n").unwrap(),
            b"CWD \xd7\xca\xc1\xcf\r\n"
        );
        // The choice is sticky: later traffic keeps using the latched encoding.
        assert_eq!(gbk.decode(b"\xd7\xca\xc1\xcf"), "资料");
    }

    #[test]
    fn copy_in_chunks_streams_large_inputs_with_a_bounded_buffer() {
        let payload = vec![0x5a; TRANSFER_CHUNK_SIZE * 3 + 17];
        let mut source = payload.as_slice();
        let mut target = Vec::new();
        let mut checkpoints = Vec::new();

        let transferred =
            copy_in_chunks(&mut source, &mut target, &CancelFlag::default(), |total| {
                checkpoints.push(total);
            })
            .expect("copy succeeds");

        assert_eq!(transferred, payload.len() as u64);
        assert_eq!(target, payload);
        assert_eq!(checkpoints.len(), 4);
        assert!(
            checkpoints
                .windows(2)
                .all(|pair| pair[1] - pair[0] <= TRANSFER_CHUNK_SIZE as u64),
            "no transfer step may exceed the fixed-size buffer"
        );
    }

    #[test]
    fn mlsd_refusal_is_remembered_but_transient_failures_are_not() {
        let refused = |status: Status| {
            FtpError::UnexpectedResponse(Response::new(status, b"500 Unknown command.".to_vec()))
        };
        assert!(mlsd_unsupported(&refused(Status::BadCommand)));
        assert!(mlsd_unsupported(&refused(Status::NotImplemented)));
        assert!(mlsd_unsupported(&refused(Status::CommandNotImplemented)));

        // A per-directory refusal or a broken data connection must not
        // disable MLSD for the rest of the session.
        assert!(!mlsd_unsupported(&refused(Status::FileUnavailable)));
        assert!(!mlsd_unsupported(&FtpError::ConnectionError(
            std::io::Error::new(std::io::ErrorKind::TimedOut, "data connection timed out")
        )));
    }
}
