//! The Tauri side of the session core's host traits.
//!
//! `session` reports output, state changes and authentication challenges
//! through [`session::EventSink`] and transfer progress through
//! [`session::ProgressSink`]; neither trait mentions a window toolkit. This
//! module is the only place that turns those calls into Tauri events, so it is
//! also the only place that knows their wire format.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter};

use crate::model::AuthPrompt;
use crate::session::{EventSink, ProgressSink, TransferProgress};

pub const EVENT_OUTPUT: &str = "session:output";
pub const EVENT_STATE: &str = "session:state";
pub const EVENT_AUTH_PROMPT: &str = "session:auth-prompt";

/// Delivers session events to the frontend as Tauri events.
pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutputEvent {
    id: String,
    /// base64 so that multi-byte UTF-8 sequences split across reads survive the
    /// trip intact; the frontend feeds the raw bytes straight into xterm.js.
    data: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StateEvent {
    id: String,
    state: String,
    message: Option<String>,
}

impl EventSink for TauriHost {
    fn output(&self, id: &str, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let _ = self.app.emit(
            EVENT_OUTPUT,
            OutputEvent {
                id: id.to_string(),
                data: B64.encode(bytes),
            },
        );
    }

    fn state(&self, id: &str, state: &str, message: Option<String>) {
        let _ = self.app.emit(
            EVENT_STATE,
            StateEvent {
                id: id.to_string(),
                state: state.to_string(),
                message,
            },
        );
    }

    fn auth_prompt(&self, prompt: &AuthPrompt) -> bool {
        self.app.emit(EVENT_AUTH_PROMPT, prompt).is_ok()
    }
}

/// Sends a transfer's progress over the `Channel` one command invocation
/// handed in.
pub struct TauriProgress(Channel<TransferProgress>);

impl TauriProgress {
    pub fn new(channel: Channel<TransferProgress>) -> Self {
        Self(channel)
    }
}

impl ProgressSink for TauriProgress {
    fn send(&self, progress: TransferProgress) {
        let _ = self.0.send(progress);
    }
}
