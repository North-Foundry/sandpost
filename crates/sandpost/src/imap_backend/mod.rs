//! Connect the IMAP protocol to the application's IMAP accounts.
//!
//! This adapter implements `sandpost-imap`'s backend traits over the application's
//! [`ImapService`], translating protocol flags and errors. Every decision about credentials,
//! mailboxes, membership, and authorization stays in the application.
use sandpost_application::{
    ImapAccessError, ImapAccountSession, ImapChangeWatch, ImapLogin, ImapMailboxHandle, ImapService,
};
use sandpost_core::ImapMessageFlags;
use sandpost_imap::{
    Account, AuthenticationOutcome, BackendError, ChangeWatch, CopyOutcome, Credentials,
    FlagChange, FlagOperation, Flags, ImapBackend, MailboxChanges, MailboxError, MailboxListing,
    MailboxSnapshot, MailboxStatus, MessageContent, MessageMetadata, MessageState, SpecialUse,
    SynchronizationRequest, SystemFlag, Uid,
};
use sandpost_storage::{ImapFlagChange, ImapFlagOperation};
use std::sync::Arc;

/// Authenticates IMAP sessions against users' IMAP accounts.
#[derive(Clone)]
pub(crate) struct ImapAccounts {
    pub service: ImapService,
}

impl ImapBackend for ImapAccounts {
    type Account = ImapAccountAdapter;

    /// Verify credentials without logging the password.
    async fn authenticate(
        &self,
        credentials: Credentials,
    ) -> Result<AuthenticationOutcome<ImapAccountAdapter>, BackendError> {
        let login = self
            .service
            .authenticate(
                &credentials.username,
                &credentials.password,
                credentials.remote_address,
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, "IMAP authentication could not be verified");
                BackendError::Unavailable(error.to_string())
            })?;
        let username = &credentials.username;
        Ok(match login {
            ImapLogin::Authenticated(session) => {
                tracing::debug!(account = %session.account_identifier(), "IMAP session authenticated");
                AuthenticationOutcome::Authenticated(ImapAccountAdapter(Arc::new(session)))
            }
            ImapLogin::Invalid => {
                tracing::info!(%username, remote = ?credentials.remote_address, "IMAP authentication failed");
                AuthenticationOutcome::InvalidCredentials
            }
            ImapLogin::Throttled => {
                tracing::warn!(%username, remote = ?credentials.remote_address, "IMAP authentication throttled");
                AuthenticationOutcome::Throttled
            }
        })
    }
}

/// One authenticated IMAP account session.
pub(crate) struct ImapAccountAdapter(Arc<ImapAccountSession>);

/// Change notifications of one account.
pub(crate) struct WatchAdapter(ImapChangeWatch);

impl ChangeWatch for WatchAdapter {
    /// Wait for a notification that may concern the account.
    async fn changed(&mut self) -> bool {
        self.0.changed().await
    }
}

/// Translate an application access failure into a session-level backend error.
fn backend_error(error: ImapAccessError) -> BackendError {
    match error {
        ImapAccessError::Revoked => BackendError::AccountRevoked,
        error => {
            tracing::warn!(%error, "IMAP operation failed");
            BackendError::Unavailable(error.to_string())
        }
    }
}

/// Translate an application access failure into a mailbox error.
fn mailbox_error(error: ImapAccessError) -> MailboxError {
    match error {
        ImapAccessError::NonExistent => MailboxError::NonExistent,
        ImapAccessError::NotPermitted(reason) => MailboxError::NotPermitted(reason),
        error => MailboxError::Backend(backend_error(error)),
    }
}

/// Convert stored flags into protocol flags.
fn protocol_flags(flags: ImapMessageFlags) -> Flags {
    let mut converted = Flags::from_parts(0, flags.keywords);
    for (set, flag) in [
        (flags.seen, SystemFlag::Seen),
        (flags.answered, SystemFlag::Answered),
        (flags.flagged, SystemFlag::Flagged),
        (flags.draft, SystemFlag::Draft),
        (flags.deleted, SystemFlag::Deleted),
    ] {
        if set {
            converted.insert(flag);
        }
    }
    converted
}

/// Convert protocol flags into stored flags.
fn stored_flags(flags: &Flags) -> ImapMessageFlags {
    ImapMessageFlags {
        seen: flags.contains(SystemFlag::Seen),
        answered: flags.contains(SystemFlag::Answered),
        flagged: flags.contains(SystemFlag::Flagged),
        draft: flags.contains(SystemFlag::Draft),
        deleted: flags.contains(SystemFlag::Deleted),
        keywords: flags.keywords().to_vec(),
    }
}

impl Account for ImapAccountAdapter {
    type Mailbox = ImapMailboxHandle;
    type Watch = WatchAdapter;

    async fn validate(&self) -> Result<(), BackendError> {
        self.0.validate().await.map_err(backend_error)
    }

    async fn mailboxes(&self) -> Result<Vec<MailboxListing>, BackendError> {
        Ok(self
            .0
            .mailboxes()
            .await
            .map_err(backend_error)?
            .into_iter()
            .map(|listing| MailboxListing {
                name: listing.name,
                selectable: listing.selectable,
                special_use: listing.is_trash.then_some(SpecialUse::Trash),
                subscribed: listing.subscribed,
            })
            .collect())
    }

    async fn set_subscription(&self, name: &str, subscribed: bool) -> Result<(), MailboxError> {
        self.0
            .set_subscription(name, subscribed)
            .await
            .map_err(mailbox_error)
    }

    /// Create a private mailbox through the owning account session.
    async fn create(&self, name: &str) -> Result<(), MailboxError> {
        self.0.create_mailbox(name).await.map_err(mailbox_error)
    }

    /// Delete a private mailbox through the owning account session.
    async fn delete(&self, name: &str) -> Result<(), MailboxError> {
        self.0.delete_mailbox(name).await.map_err(mailbox_error)
    }

    /// Rename a private mailbox through the owning account session.
    async fn rename(&self, source: &str, destination: &str) -> Result<(), MailboxError> {
        self.0
            .rename_mailbox(source, destination)
            .await
            .map_err(mailbox_error)
    }

    async fn open(
        &self,
        name: &str,
        read_only: bool,
    ) -> Result<MailboxSnapshot<ImapMailboxHandle>, MailboxError> {
        let snapshot = self.0.open(name, read_only).await.map_err(mailbox_error)?;
        Ok(MailboxSnapshot {
            handle: snapshot.handle,
            uid_validity: snapshot.uid_validity,
            uid_next: snapshot.uid_next,
            messages: snapshot
                .members
                .into_iter()
                .map(|(uid, flags, recent)| MessageState {
                    uid,
                    flags: protocol_flags(flags),
                    recent,
                })
                .collect(),
            seen_only: snapshot.seen_only,
            cursor: snapshot.cursor,
        })
    }

    async fn status(&self, name: &str) -> Result<MailboxStatus, MailboxError> {
        let status = self.0.status(name).await.map_err(mailbox_error)?;
        Ok(MailboxStatus {
            messages: status.messages,
            recent: status.recent,
            uid_next: status.uid_next,
            uid_validity: status.uid_validity,
            unseen: status.unseen,
        })
    }

    async fn synchronize(
        &self,
        mailbox: &ImapMailboxHandle,
        request: SynchronizationRequest,
    ) -> Result<MailboxChanges, MailboxError> {
        let changes = self
            .0
            .synchronize(
                mailbox,
                request.highest_uid,
                request.known_count,
                request.cursor,
                request.claim_recent,
            )
            .await
            .map_err(mailbox_error)?;
        Ok(MailboxChanges {
            present_uids: changes.present_uids,
            added: changes
                .added
                .into_iter()
                .map(|(uid, flags, recent)| MessageState {
                    uid,
                    flags: protocol_flags(flags),
                    recent,
                })
                .collect(),
            flag_updates: changes
                .flag_updates
                .into_iter()
                .map(|(uid, flags)| (uid, protocol_flags(flags)))
                .collect(),
            uid_next: changes.uid_next,
            cursor: changes.cursor,
        })
    }

    async fn metadata(
        &self,
        mailbox: &ImapMailboxHandle,
        uids: &[Uid],
    ) -> Result<Vec<MessageMetadata>, MailboxError> {
        Ok(self
            .0
            .metadata(mailbox, uids)
            .await
            .map_err(mailbox_error)?
            .into_iter()
            .map(|metadata| MessageMetadata {
                uid: metadata.uid,
                size: metadata.size,
                internal_date: metadata.received_at,
            })
            .collect())
    }

    async fn content(
        &self,
        mailbox: &ImapMailboxHandle,
        uid: Uid,
    ) -> Result<Option<MessageContent>, MailboxError> {
        Ok(self
            .0
            .content(mailbox, uid)
            .await
            .map_err(mailbox_error)?
            .map(|content| MessageContent {
                octets: Arc::from(content.raw_message),
                internal_date: content.received_at,
            }))
    }

    async fn store(
        &self,
        mailbox: &ImapMailboxHandle,
        uids: &[Uid],
        change: &FlagChange,
    ) -> Result<Vec<(Uid, Flags)>, MailboxError> {
        let change = ImapFlagChange {
            operation: match change.operation {
                FlagOperation::Replace => ImapFlagOperation::Replace,
                FlagOperation::Add => ImapFlagOperation::Add,
                FlagOperation::Remove => ImapFlagOperation::Remove,
            },
            flags: stored_flags(&change.flags),
        };
        Ok(self
            .0
            .store(mailbox, uids, &change)
            .await
            .map_err(mailbox_error)?
            .into_iter()
            .map(|(uid, flags)| (uid, protocol_flags(flags)))
            .collect())
    }

    async fn expunge(
        &self,
        mailbox: &ImapMailboxHandle,
        uids: Option<&[Uid]>,
    ) -> Result<Vec<Uid>, MailboxError> {
        self.0.expunge(mailbox, uids).await.map_err(mailbox_error)
    }

    async fn copy(
        &self,
        mailbox: &ImapMailboxHandle,
        uids: &[Uid],
        destination: &str,
        remove_from_source: bool,
    ) -> Result<CopyOutcome, MailboxError> {
        let result = self
            .0
            .copy(mailbox, uids, destination, remove_from_source)
            .await
            .map_err(mailbox_error)?;
        Ok(CopyOutcome {
            uid_validity: result.uid_validity,
            source_uids: result.source_uids,
            // COPYUID describes newly assigned UIDs only (RFC 4315 section 3).
            destination_uids: if result.all_new {
                result.destination_uids
            } else {
                Vec::new()
            },
            expunged: result.removed_uids,
        })
    }

    fn watch(&self) -> WatchAdapter {
        WatchAdapter(self.0.watch())
    }
}

#[cfg(test)]
mod tests;
