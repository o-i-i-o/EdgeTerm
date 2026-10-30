//! Host key changes a connection refused, waiting for the user's decision.
//!
//! `accept_host_key` used to take the host, port and key straight from the
//! webview, so a compromised page could rewrite `known_hosts` for any host and
//! pin a key of its own there. A change is now registered here by the
//! connection that observed it, under a one-shot id; accepting names only that
//! id, and what reaches `known_hosts` is the key this process saw during the
//! handshake.

use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::model::HostKeyChange;

/// How long a reported change stays acceptable. The dialog is answered within
/// seconds of it appearing; the window only keeps a page that never answers
/// from growing the list.
const TTL: Duration = Duration::from_secs(10 * 60);

/// Changes remembered at once, oldest dropped first, in case a page keeps
/// opening sessions and never answers the dialogs.
const MAX_PENDING: usize = 16;

/// The reported changes nobody has decided on yet, in the order they were
/// reported, each with the id the dialog carries.
#[derive(Default)]
pub struct HostKeyChanges {
    pending: Mutex<Vec<(String, Instant, HostKeyChange)>>,
}

impl HostKeyChanges {
    /// Records a change this process observed and returns it carrying the id
    /// the user's decision will name.
    pub fn register(&self, mut change: HostKeyChange) -> HostKeyChange {
        let id = uuid::Uuid::new_v4().to_string();
        change.id = id.clone();
        let mut pending = self.pending.lock();
        pending.retain(|(_, seen, _)| seen.elapsed() < TTL);
        while pending.len() >= MAX_PENDING {
            pending.remove(0);
        }
        pending.push((id, Instant::now(), change.clone()));
        change
    }

    /// The change `id` names, kept registered for as long as the user may
    /// still answer it. `None` for an id this process never issued, or one
    /// that has expired.
    pub fn peek(&self, id: &str) -> Option<HostKeyChange> {
        let mut pending = self.pending.lock();
        pending.retain(|(_, seen, _)| seen.elapsed() < TTL);
        pending
            .iter()
            .find(|(known, _, _)| known == id)
            .map(|(_, _, change)| change.clone())
    }

    /// Drops the change `id` names, after the decision has been carried out.
    /// A failed write leaves it in place, so the dialog can be answered
    /// again.
    pub fn forget(&self, id: &str) {
        self.pending.lock().retain(|(known, _, _)| known != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(host: &str) -> HostKeyChange {
        HostKeyChange {
            id: String::new(),
            host: host.to_string(),
            port: 22,
            key_type: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:abc".to_string(),
            public_key: "ssh-ed25519 AAAA test".to_string(),
            known_hosts: "/home/me/.ssh/known_hosts".to_string(),
            line: 3,
            message: "host key changed".to_string(),
        }
    }

    #[test]
    fn a_registered_change_carries_a_fresh_id() {
        let changes = HostKeyChanges::default();
        let first = changes.register(change("one.example"));
        let second = changes.register(change("two.example"));

        assert!(!first.id.is_empty());
        assert_ne!(first.id, second.id, "every report gets its own id");
        assert_eq!(first.host, "one.example");
    }

    #[test]
    fn a_change_is_answered_by_its_own_id_and_dropped_once_decided() {
        let changes = HostKeyChanges::default();
        let reported = changes.register(change("one.example"));

        assert!(changes.peek("not-issued-by-this-process").is_none());
        let seen = changes.peek(&reported.id).expect("the issued id answers");
        assert_eq!(seen.public_key, reported.public_key);
        assert!(
            changes.peek(&reported.id).is_some(),
            "looking at a change does not consume it: a failed write has to be answerable again"
        );

        changes.forget(&reported.id);
        assert!(
            changes.peek(&reported.id).is_none(),
            "a decided change is not answered twice"
        );
    }

    #[test]
    fn pending_changes_are_bounded() {
        let changes = HostKeyChanges::default();
        let ids: Vec<String> = (0..MAX_PENDING + 4)
            .map(|index| changes.register(change(&format!("host{index}"))).id)
            .collect();

        assert!(changes.peek(&ids[0]).is_none(), "the oldest is dropped");
        assert!(
            changes.peek(&ids[ids.len() - 1]).is_some(),
            "the newest stays"
        );
        assert_eq!(changes.pending.lock().len(), MAX_PENDING);
    }
}
