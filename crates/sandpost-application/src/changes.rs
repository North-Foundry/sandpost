//! Committed-change notifications for long-lived sessions such as IMAP IDLE.
//!
//! Notifications are coarse hints sent only after the storage operation that caused them has
//! committed. They never carry data: a receiver rereads authoritative state and re-applies
//! authorization, so a lost or lagged notification only delays an update.
use crate::{Application, AuthorizedApplication};
use sandpost_core::ImapAccountIdentifier;
use tokio::sync::broadcast;

/// A committed change that may alter what some session can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationChange {
    /// Mail visibility changed through user deletion or physical maintenance.
    /// Insertions arrive as message events.
    Mail,
    /// Something that decides or organizes mail visibility changed: users, roles, mail access,
    /// scopes, scope assignments, personal filters, or saved Views.
    Visibility,
    /// An IMAP account's settings, credentials, mailboxes, or flags changed.
    ImapAccount(ImapAccountIdentifier),
}

impl Application {
    /// Publish a committed change to every subscriber; having no subscriber is not an error.
    pub fn notify_change(&self, change: ApplicationChange) {
        let _ = self.changes.send(change);
    }

    /// Subscribe to committed changes.
    pub fn subscribe_changes(&self) -> broadcast::Receiver<ApplicationChange> {
        self.changes.subscribe()
    }
}

impl AuthorizedApplication {
    /// Publish a committed change on behalf of this caller.
    pub(crate) fn notify(&self, change: ApplicationChange) {
        self.application.notify_change(change);
    }
}
