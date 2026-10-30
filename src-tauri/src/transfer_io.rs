//! The disk side of the ZMODEM / XMODEM streaming commands.
//!
//! Kept out of `commands.rs` so the access rules (`transfer_access`) and the
//! file I/O can be exercised without a Tauri app: every entry point takes the
//! grants explicitly instead of reaching for `AppState`.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use std::path::Path;
use std::time::UNIX_EPOCH;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::error::{AppError, Result, err};
use crate::model::ZmodemFileInfo;
use crate::transfer_access::TransferAccess;

/// Keep file IPC bounded. zmodem.js further divides outgoing data into 8 KiB
/// protocol subpackets and XMODEM into 128 / 1024-byte blocks, while this
/// larger application-level chunk keeps the number of disk and IPC round
/// trips reasonable. The frontend shares these commands between the two
/// protocols (terminalTransfer.ts).
pub const CHUNK_SIZE: usize = 1024 * 1024;

pub async fn file_info(access: &TransferAccess, path: &str) -> Result<ZmodemFileInfo> {
    let path = access.require(path)?;
    let metadata = tokio::fs::metadata(&path).await?;
    if !metadata.is_file() {
        return Err(AppError::new(format!(
            "not a regular file: {}",
            path.display()
        )));
    }

    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::new(format!("file has no usable name: {}", path.display())))?
        .to_string();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());

    Ok(ZmodemFileInfo {
        name,
        size: metadata.len(),
        modified,
    })
}

pub async fn read_chunk(
    access: &TransferAccess,
    path: &str,
    offset: u64,
    length: u32,
) -> Result<String> {
    let path = access.require(path)?;
    let length = length as usize;
    if length == 0 || length > CHUNK_SIZE {
        return Err(AppError::new(format!(
            "ZMODEM read length must be between 1 and {CHUNK_SIZE} bytes"
        )));
    }

    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut bytes = Vec::with_capacity(length);
    file.take(length as u64).read_to_end(&mut bytes).await?;
    Ok(B64.encode(bytes))
}

/// Opens a granted file to receive a transfer, refusing a link planted at its
/// name and anything that is not a regular file.
///
/// The grant came from the save dialog, but a grant is only a name: between
/// the pick and the first chunk another process on this machine could replace
/// it with a link to a file of its choosing, or with a FIFO that swallows the
/// bytes and passes them on. The name is looked at first (opening a FIFO for
/// writing would otherwise wait for a reader), `O_NOFOLLOW` refuses a link
/// that appears in between, and the handle is checked afterwards because that
/// is the file actually written.
///
/// On Windows there is no `O_NOFOLLOW`, so the handle check sees through a
/// reparse point planted in that window and the write lands wherever it points.
/// Closing that would take `FILE_FLAG_OPEN_REPARSE_POINT` plus a reparse-tag
/// check, which is not done here.
///
/// A link already at the name where the transfer creates a file is refused:
/// the bytes would go somewhere the user did not pick.
async fn open_for_write(path: &Path, create: bool) -> Result<tokio::fs::File> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(AppError::new(format!(
                "refusing to write through the link at {}",
                path.display()
            )));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(AppError::new(format!(
                "not a regular file: {}",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let mut options = tokio::fs::OpenOptions::new();
    options.write(true);
    if create {
        options.create(true).truncate(true);
    }
    #[cfg(unix)]
    {
        // A link that appeared since the check above makes this fail rather
        // than redirect the write.
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).await?;
    if !file.metadata().await?.is_file() {
        return Err(AppError::new(format!(
            "not a regular file: {}",
            path.display()
        )));
    }
    Ok(file)
}

pub async fn create_file(access: &TransferAccess, path: &str) -> Result<()> {
    let path = access.require(path)?;
    let file = open_for_write(&path, true).await?;
    file.sync_all().await?;
    Ok(())
}

pub async fn write_chunk(
    access: &TransferAccess,
    path: &str,
    offset: u64,
    data: &str,
) -> Result<()> {
    let path = access.require(path)?;
    let bytes = B64.decode(data).map_err(err)?;
    if bytes.len() > CHUNK_SIZE {
        return Err(AppError::new(format!(
            "ZMODEM write chunk exceeds {CHUNK_SIZE} bytes"
        )));
    }

    let mut file = open_for_write(&path, false).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    file.write_all(&bytes).await?;
    Ok(())
}

pub async fn finish_file(access: &TransferAccess, path: &str, size: u64) -> Result<()> {
    let path = access.require(path)?;
    let file = open_for_write(&path, false).await?;
    file.set_len(size).await?;
    file.sync_all().await?;
    Ok(())
}
