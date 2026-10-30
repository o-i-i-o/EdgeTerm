use std::fmt::{self, Display};

/// What kind of failure this is.
///
/// The message is still the payload that crosses IPC — a plain string is all
/// the frontend can act on — but local code and the log can tell a cancelled
/// dialog from a protocol rejection or a disk failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Trying again may succeed.
    Retry,
    /// The user declined, or closed a dialog.
    Cancelled,
    /// The wire protocol or its peer refused something.
    Protocol,
    /// The environment: IO, permissions, a missing file.
    Environment,
    /// Anything else; the message says what happened.
    Other,
}

/// A flat, display-oriented error.
#[derive(Debug, Clone)]
pub struct AppError {
    message: String,
    kind: ErrorKind,
}

impl AppError {
    pub fn new(msg: impl Display) -> Self {
        Self {
            message: msg.to_string(),
            kind: ErrorKind::Other,
        }
    }

    pub fn cancelled(msg: impl Display) -> Self {
        Self {
            message: msg.to_string(),
            kind: ErrorKind::Cancelled,
        }
    }

    pub fn retry(msg: impl Display) -> Self {
        Self {
            message: msg.to_string(),
            kind: ErrorKind::Retry,
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The message on its own, for the places that wrap an error into a
    /// longer one.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for AppError {}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.message())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// `map_err(err)` adapter for any foreign error type.
pub fn err<E: Display>(e: E) -> AppError {
    AppError::new(e)
}

macro_rules! from_display {
    ($kind:ident: $($t:ty),* $(,)?) => {
        $(impl From<$t> for AppError {
            fn from(e: $t) -> Self {
                Self {
                    message: e.to_string(),
                    kind: ErrorKind::$kind,
                }
            }
        })*
    };
}

// The kinds follow the source: the operating system refused something, a peer
// or protocol did, or the error is one of ours.
from_display!(Environment: std::io::Error);
from_display!(
    Protocol:
        russh::Error,
        russh::keys::Error,
        russh_sftp::client::error::Error,
        suppaftp::FtpError,
        serialport::Error,
);
from_display!(Other: std::num::ParseIntError, serde_json::Error);

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError::new(e)
    }
}

impl From<&str> for AppError {
    fn from(e: &str) -> Self {
        AppError::new(e)
    }
}
