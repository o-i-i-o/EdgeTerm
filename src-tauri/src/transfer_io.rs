//! The disk side of the ZMODEM / XMODEM streaming commands.
//!
//! Kept out of `commands.rs` so the access rules (`transfer_access`) and the
//! file I/O can be exercised without a Tauri app: every entry point takes the
//! grants explicitly instead of reaching for `AppState`.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
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

pub async fn create_file(access: &TransferAccess, path: &str) -> Result<()> {
    let path = access.require(path)?;
    let file = tokio::fs::File::create(path).await?;
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

    let mut file = tokio::fs::OpenOptions::new().write(true).open(path).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    file.write_all(&bytes).await?;
    Ok(())
}

pub async fn finish_file(access: &TransferAccess, path: &str, size: u64) -> Result<()> {
    let path = access.require(path)?;
    let file = tokio::fs::OpenOptions::new().write(true).open(path).await?;
    file.set_len(size).await?;
    file.sync_all().await?;
    Ok(())
}
