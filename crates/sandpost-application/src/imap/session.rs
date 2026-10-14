//! Mailbox operations and state for an authenticated IMAP account session.
//!
//! Every operation that exposes messages revalidates the account and reconciles the mailbox
//! against its current definition, including the owner's current authorization.
use super::catalog::{self, PlannedMailbox};
use crate::credentials::current_unix_seconds;
use crate::{ApplicationChange, ApplicationError, MessageEvent};
use sandpost_core::{
    GlobalRole, ImapAccount, ImapAccountIdentifier, ImapAccountMode, ImapFolderKind,
    ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags, UserIdentifier,
};
use sandpost_storage::{
    ImapCopyResult, ImapFlagChange, ImapLocalFolderSettings, ImapMailboxState, ImapMemberContent,
    ImapMemberMetadata, Storage, StorageError,
};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Why an IMAP session operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ImapAccessError {
    /// The account was deleted, disabled, or its password replaced; the session must end.
    #[error("the IMAP account is no longer available")]
    Revoked,
    /// No mailbox with that name is visible to the account.
    #[error("mailbox does not exist")]
    NonExistent,
    /// The operation is not possible for this mailbox.
    #[error("{0}")]
    NotPermitted(String),
    /// A temporary failure such as unavailable storage.
    #[error("temporarily unavailable: {0}")]
    Unavailable(String),
}

impl From<ApplicationError> for ImapAccessError {
    /// Treat unexpected application failures as temporary, without exposing details.
    fn from(error: ApplicationError) -> Self {
        Self::Unavailable(error.to_string())
    }
}

impl From<StorageError> for ImapAccessError {
    /// Report missing mailboxes distinctly; other storage failures remain temporary.
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::NotFound => Self::NonExistent,
            error => Self::Unavailable(error.to_string()),
        }
    }
}

/// A mailbox selected by a session: its identity and source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImapMailboxHandle {
    pub identifier: ImapMailboxIdentifier,
    pub source: ImapMailboxSource,
}

/// A live mailbox or retained subscription for LIST and LSUB responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapMailboxListing {
    pub name: String,
    pub is_trash: bool,
    pub subscribed: bool,
    /// False for a retained subscription whose mailbox is absent from the current catalog.
    pub selectable: bool,
}

/// A mailbox as SELECT sees it.
#[derive(Debug, Clone)]
pub struct ImapMailboxSnapshot {
    pub handle: ImapMailboxHandle,
    pub uid_validity: u32,
    pub uid_next: u32,
    /// Every member in ascending UID order with its flags and whether it is recent.
    pub members: Vec<(u32, ImapMessageFlags, bool)>,
    pub cursor: u64,
    /// Smart mailboxes permit clients to change only `\Seen`.
    pub seen_only: bool,
}

/// STATUS data of a mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImapMailboxStatus {
    pub messages: u32,
    pub recent: u32,
    pub uid_next: u32,
    pub uid_validity: u32,
    pub unseen: u32,
}

/// Membership and flag changes since what a session knows.
#[derive(Debug, Clone, Default)]
pub struct ImapMembershipChanges {
    /// Every UID up to the known highest that still exists, when some known member is gone.
    pub present_uids: Option<Vec<u32>>,
    /// New members above the known highest UID, with flags and recency.
    pub added: Vec<(u32, ImapMessageFlags, bool)>,
    pub flag_updates: Vec<(u32, ImapMessageFlags)>,
    pub uid_next: u32,
    pub cursor: u64,
}

/// The mailbox operations of one authenticated IMAP connection.
///
/// Intentionally lacks Debug: it retains the password hash verified at login.
pub struct ImapAccountSession {
    pub(super) storage: Arc<dyn Storage>,
    pub(super) changes: broadcast::Sender<ApplicationChange>,
    pub(super) messages: broadcast::Sender<MessageEvent>,
    pub(super) account_identifier: ImapAccountIdentifier,
    pub(super) owner_identifier: UserIdentifier,
    pub(super) verified_password_hash: String,
    pub(super) authenticated_mode_generation: u64,
}

/// Notifications relevant to one IMAP account, used while its sessions idle.
pub struct ImapChangeWatch {
    account: ImapAccountIdentifier,
    changes: broadcast::Receiver<ApplicationChange>,
    messages: broadcast::Receiver<MessageEvent>,
}

impl ImapChangeWatch {
    /// Wait for the next notification that may concern the account.
    ///
    /// Cancel-safe. Lagging counts as a change; `false` means notifications ended.
    pub async fn changed(&mut self) -> bool {
        loop {
            tokio::select! {
                change = self.changes.recv() => match change {
                    Ok(ApplicationChange::ImapAccount(account)) if account != self.account => {}
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => return true,
                    Err(broadcast::error::RecvError::Closed) => return false,
                },
                message = self.messages.recv() => match message {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => return true,
                    Err(broadcast::error::RecvError::Closed) => return false,
                },
            }
        }
    }
}

impl ImapAccountSession {
    /// Validate names in the supported flat namespace without silently changing client input.
    fn private_mailbox_name(name: &str) -> Result<String, ImapAccessError> {
        if name.is_empty()
            || name.trim() != name
            || name.chars().count() > 128
            || name.chars().any(char::is_control)
            || name
                .split('/')
                .any(|part| part.is_empty() || part.trim() != part)
        {
            return Err(ImapAccessError::NotPermitted(
                "mailbox names must contain 1 to 128 characters with nonempty hierarchy segments"
                    .into(),
            ));
        }
        Ok(name.to_owned())
    }

    /// Create an empty manual folder belonging only to the authenticated IMAP account.
    pub async fn create_mailbox(&self, name: &str) -> Result<(), ImapAccessError> {
        let name = Self::private_mailbox_name(name)?;
        let (account, catalog) = self.catalog().await?;
        if catalog
            .iter()
            .any(|(mailbox, _)| mailbox.name.eq_ignore_ascii_case(&name))
        {
            return Err(ImapAccessError::NotPermitted(
                "mailbox already exists".into(),
            ));
        }
        if catalog.len() >= 302 {
            return Err(ImapAccessError::NotPermitted(
                "mailbox limit reached".into(),
            ));
        }
        let result = self
            .storage
            .create_imap_private_folder(
                self.account_identifier,
                &name,
                current_unix_seconds(),
                Some(self.authenticated_mode_generation),
            )
            .await;
        if result.is_ok() {
            self.notify_mailbox_change(account.mode);
        }
        self.current_account().await?;
        result?;
        Ok(())
    }

    /// Remove a private folder, restoring baseline placement without deleting its messages.
    pub async fn delete_mailbox(&self, name: &str) -> Result<(), ImapAccessError> {
        let owner = self
            .storage
            .get_user(self.owner_identifier)
            .await?
            .ok_or(ImapAccessError::Revoked)?;
        if owner.global_role == GlobalRole::Viewer {
            return Err(ImapAccessError::NotPermitted(
                "this account cannot delete mail folders".into(),
            ));
        }
        let (account, catalog) = self.catalog().await?;
        let index = Self::find(&catalog, name)?;
        let folder = catalog[index]
            .0
            .folder
            .as_ref()
            .filter(|folder| {
                matches!(
                    folder.kind,
                    ImapFolderKind::Manual | ImapFolderKind::Private
                )
            })
            .ok_or_else(|| {
                ImapAccessError::NotPermitted("this folder is managed through Sandpost Web".into())
            })?;
        let result = self
            .storage
            .delete_imap_local_folder(folder.identifier, Some(self.authenticated_mode_generation))
            .await;
        if result.is_ok() {
            self.notify_mailbox_change(account.mode);
        }
        self.current_account().await?;
        result?;
        Ok(())
    }

    /// Rename an account-private definition without changing its mailbox identity or UIDs.
    pub async fn rename_mailbox(
        &self,
        source: &str,
        destination: &str,
    ) -> Result<(), ImapAccessError> {
        let name = Self::private_mailbox_name(destination)?;
        let (account, catalog) = self.catalog().await?;
        let index = Self::find(&catalog, source)?;
        let folder = catalog[index]
            .0
            .folder
            .as_ref()
            .filter(|folder| {
                matches!(
                    folder.kind,
                    ImapFolderKind::Manual | ImapFolderKind::Private
                )
            })
            .ok_or_else(|| {
                ImapAccessError::NotPermitted("this folder is managed through Sandpost Web".into())
            })?;
        if catalog
            .iter()
            .any(|(mailbox, _)| mailbox.name.eq_ignore_ascii_case(&name))
        {
            return Err(ImapAccessError::NotPermitted(
                "mailbox already exists".into(),
            ));
        }
        let result = self
            .storage
            .update_imap_local_folder(
                folder.identifier,
                &ImapLocalFolderSettings {
                    expected_mode_generation: Some(self.authenticated_mode_generation),
                    name,
                    filter: folder.filter.clone(),
                    include_in_inbox: folder.include_in_inbox,
                },
            )
            .await;
        if result.is_ok() {
            self.notify_mailbox_change(account.mode);
        }
        self.current_account().await?;
        result?;
        Ok(())
    }

    /// The account this session authenticated as.
    pub fn account_identifier(&self) -> ImapAccountIdentifier {
        self.account_identifier
    }

    /// Subscribe to notifications relevant to this account.
    pub fn watch(&self) -> ImapChangeWatch {
        ImapChangeWatch {
            account: self.account_identifier,
            changes: self.changes.subscribe(),
            messages: self.messages.subscribe(),
        }
    }

    /// Load the account, requiring it to be enabled, owned by an existing user, and still
    /// authenticating with the password hash verified at login.
    async fn current_account(&self) -> Result<ImapAccount, ImapAccessError> {
        let credential = self
            .storage
            .get_imap_account_credential(self.account_identifier)
            .await?
            .ok_or(ImapAccessError::Revoked)?;
        if !credential.account.enabled
            || credential.password_hash != self.verified_password_hash
            || credential.account.owner_identifier != self.owner_identifier
            || credential.account.mode_generation != self.authenticated_mode_generation
        {
            return Err(ImapAccessError::Revoked);
        }
        if self
            .storage
            .get_user(self.owner_identifier)
            .await?
            .is_none()
        {
            return Err(ImapAccessError::Revoked);
        }
        Ok(credential.account)
    }

    /// Publish shared mirror changes to every affected mail view, or private changes to this
    /// account's sessions only.
    fn notify_mailbox_change(&self, mode: ImapAccountMode) {
        let change = if mode == ImapAccountMode::Mirror {
            ApplicationChange::Visibility
        } else {
            ApplicationChange::ImapAccount(self.account_identifier)
        };
        let _ = self.changes.send(change);
    }

    /// Revalidate a mailbox and report whether its membership is smart-folder driven.
    async fn reconcile_handle_kind(
        &self,
        handle: &ImapMailboxHandle,
    ) -> Result<(ImapAccount, ImapMailboxState, bool), ImapAccessError> {
        let (account, catalog) = self.catalog().await?;
        let index = Self::find_handle(&catalog, handle)?;
        let mailbox = &catalog[index].0;
        let smart = mailbox.view.is_some()
            || mailbox
                .folder
                .as_ref()
                .is_some_and(|folder| folder.kind == ImapFolderKind::Smart);
        let state = self.reconcile(&account, &catalog, index).await?;
        Ok((account, state, smart))
    }

    /// Confirm the session may continue.
    pub async fn validate(&self) -> Result<(), ImapAccessError> {
        self.current_account().await.map(|_| ())
    }

    /// Revalidate the account, plan its mailboxes, and ensure their identities exist.
    async fn catalog(
        &self,
    ) -> Result<(ImapAccount, Vec<(PlannedMailbox, ImapMailboxState)>), ImapAccessError> {
        let account = self.current_account().await?;
        let planned = catalog::load_plan(&self.storage, &account).await?;
        let sources: Vec<ImapMailboxSource> =
            planned.iter().map(|mailbox| mailbox.source).collect();
        let states = match self
            .storage
            .reconcile_imap_mailboxes(
                account.identifier,
                &sources,
                Some(self.authenticated_mode_generation),
            )
            .await
        {
            Ok(states) => states,
            // A View deleted between planning and reconciliation; plan again once.
            Err(StorageError::NotFound) => {
                self.current_account().await?;
                let planned = catalog::load_plan(&self.storage, &account).await?;
                let sources: Vec<_> = planned.iter().map(|mailbox| mailbox.source).collect();
                let result = self
                    .storage
                    .reconcile_imap_mailboxes(
                        account.identifier,
                        &sources,
                        Some(self.authenticated_mode_generation),
                    )
                    .await;
                self.current_account().await?;
                let states = result?;
                return Ok((account, planned.into_iter().zip(states).collect()));
            }
            Err(error) => {
                self.current_account().await?;
                return Err(error.into());
            }
        };
        self.current_account().await?;
        Ok((account, planned.into_iter().zip(states).collect()))
    }

    /// Reconcile one planned mailbox's membership against its current definition.
    async fn reconcile(
        &self,
        account: &ImapAccount,
        catalog: &[(PlannedMailbox, ImapMailboxState)],
        index: usize,
    ) -> Result<ImapMailboxState, ImapAccessError> {
        let planned: Vec<PlannedMailbox> =
            catalog.iter().map(|(mailbox, _)| mailbox.clone()).collect();
        let authorization =
            catalog::owner_authorization(&self.storage, self.owner_identifier).await?;
        let definition = catalog::definition(account, &planned, &catalog[index].0, &authorization);
        let result = self
            .storage
            .synchronize_imap_mailbox(catalog[index].1.identifier, &definition)
            .await;
        self.current_account().await?;
        result?.ok_or(ImapAccessError::NonExistent)
    }

    /// Find a mailbox by name; INBOX matches case-insensitively.
    fn find(
        catalog: &[(PlannedMailbox, ImapMailboxState)],
        name: &str,
    ) -> Result<usize, ImapAccessError> {
        catalog
            .iter()
            .position(|(mailbox, _)| {
                mailbox.name == name
                    || (mailbox.source == ImapMailboxSource::Inbox
                        && name.eq_ignore_ascii_case(catalog::INBOX_NAME))
            })
            .ok_or(ImapAccessError::NonExistent)
    }

    /// Find a session-owned mailbox with the handle's identity and source, or fail closed.
    fn find_handle(
        catalog: &[(PlannedMailbox, ImapMailboxState)],
        handle: &ImapMailboxHandle,
    ) -> Result<usize, ImapAccessError> {
        catalog
            .iter()
            .position(|(_, state)| {
                state.identifier == handle.identifier && state.source == handle.source
            })
            .ok_or(ImapAccessError::NonExistent)
    }

    /// Revalidate credentials and reconcile a session-owned handle against current authorization.
    async fn reconcile_handle(
        &self,
        handle: &ImapMailboxHandle,
    ) -> Result<ImapMailboxState, ImapAccessError> {
        let (account, catalog) = self.catalog().await?;
        let index = Self::find_handle(&catalog, handle)?;
        self.reconcile(&account, &catalog, index).await
    }

    /// List live mailboxes and retained subscriptions, persisting newly observed default names.
    pub async fn mailboxes(&self) -> Result<Vec<ImapMailboxListing>, ImapAccessError> {
        let (_, catalog) = self.catalog().await?;
        let unsubscribed: std::collections::HashSet<_> = self
            .storage
            .list_imap_unsubscribed(self.account_identifier)
            .await?
            .into_iter()
            .collect();
        let mut subscribed: std::collections::BTreeSet<_> = self
            .storage
            .list_imap_subscribed(self.account_identifier)
            .await?
            .into_iter()
            .collect();
        let default_subscriptions: Vec<_> = catalog
            .iter()
            .map(|(mailbox, _)| &mailbox.name)
            .filter(|name| !unsubscribed.contains(*name) && !subscribed.contains(*name))
            .cloned()
            .collect();
        if !default_subscriptions.is_empty() {
            self.storage
                .retain_imap_subscriptions(self.account_identifier, &default_subscriptions)
                .await?;
        }
        let mut listings: Vec<_> = catalog
            .into_iter()
            .map(|(mailbox, _)| {
                subscribed.remove(&mailbox.name);
                ImapMailboxListing {
                    subscribed: !unsubscribed.contains(&mailbox.name),
                    is_trash: mailbox.source == ImapMailboxSource::Trash,
                    name: mailbox.name,
                    selectable: true,
                }
            })
            .collect();
        listings.extend(
            subscribed
                .into_iter()
                .filter(|name| !unsubscribed.contains(name))
                .map(|name| ImapMailboxListing {
                    name,
                    is_trash: false,
                    subscribed: true,
                    selectable: false,
                }),
        );
        self.current_account().await?;
        Ok(listings)
    }

    /// Subscribe to or unsubscribe from a mailbox name.
    pub async fn set_subscription(
        &self,
        name: &str,
        subscribed: bool,
    ) -> Result<(), ImapAccessError> {
        self.current_account().await?;
        self.storage
            .set_imap_subscription(self.account_identifier, name, subscribed)
            .await?;
        self.current_account().await?;
        Ok(())
    }

    /// Select a mailbox: reconcile it, claim recent members for read-write sessions, and return
    /// every member.
    pub async fn open(
        &self,
        name: &str,
        read_only: bool,
    ) -> Result<ImapMailboxSnapshot, ImapAccessError> {
        let (account, catalog) = self.catalog().await?;
        let index = Self::find(&catalog, name)?;
        let state = self.reconcile(&account, &catalog, index).await?;
        let previous_recent = if read_only {
            state.recent_through
        } else {
            self.storage
                .claim_imap_recent(state.identifier)
                .await?
                .ok_or(ImapAccessError::NonExistent)?
        };
        let cursor = self
            .storage
            .imap_flag_cursor(self.account_identifier)
            .await?;
        let members = self
            .storage
            .imap_mailbox_members(state.identifier, 0)
            .await?
            .into_iter()
            .map(|member| (member.uid, member.flags, member.uid > previous_recent))
            .collect();
        self.current_account().await?;
        Ok(ImapMailboxSnapshot {
            handle: ImapMailboxHandle {
                identifier: state.identifier,
                source: state.source,
            },
            uid_validity: state.uid_validity,
            uid_next: state.uid_next,
            members,
            cursor,
            seen_only: catalog[index].0.view.is_some()
                || catalog[index]
                    .0
                    .folder
                    .as_ref()
                    .is_some_and(|folder| folder.kind == ImapFolderKind::Smart),
        })
    }

    /// Report STATUS data of a mailbox after reconciling it.
    pub async fn status(&self, name: &str) -> Result<ImapMailboxStatus, ImapAccessError> {
        let (account, catalog) = self.catalog().await?;
        let index = Self::find(&catalog, name)?;
        let state = self.reconcile(&account, &catalog, index).await?;
        let counts = self
            .storage
            .imap_mailbox_counts(state.identifier)
            .await?
            .ok_or(ImapAccessError::NonExistent)?;
        self.current_account().await?;
        Ok(ImapMailboxStatus {
            messages: counts.messages,
            recent: counts.recent,
            uid_next: state.uid_next,
            uid_validity: state.uid_validity,
            unseen: counts.unseen,
        })
    }

    /// Reconcile a selected mailbox and report what changed since the session's knowledge.
    pub async fn synchronize(
        &self,
        handle: &ImapMailboxHandle,
        highest_uid: u32,
        known_count: u32,
        cursor: u64,
        claim_recent: bool,
    ) -> Result<ImapMembershipChanges, ImapAccessError> {
        let (account, catalog) = self.catalog().await?;
        let index = Self::find_handle(&catalog, handle)?;
        let state = self.reconcile(&account, &catalog, index).await?;
        let count = self
            .storage
            .count_imap_mailbox_members(state.identifier, highest_uid)
            .await?;
        let present_uids = if count != u64::from(known_count) {
            Some(
                self.storage
                    .imap_mailbox_uids(state.identifier, highest_uid)
                    .await?,
            )
        } else {
            None
        };
        let flag_changes = self
            .storage
            .imap_flag_changes(state.identifier, cursor, highest_uid)
            .await?;
        let new_members = self
            .storage
            .imap_mailbox_members(state.identifier, highest_uid)
            .await?;
        let previous_recent = if claim_recent && !new_members.is_empty() {
            self.storage
                .claim_imap_recent(state.identifier)
                .await?
                .unwrap_or(u32::MAX)
        } else {
            u32::MAX
        };
        let uid_next = new_members.last().map_or(state.uid_next, |member| {
            state.uid_next.max(member.uid.saturating_add(1))
        });
        self.current_account().await?;
        Ok(ImapMembershipChanges {
            present_uids,
            added: new_members
                .into_iter()
                .map(|member| (member.uid, member.flags, member.uid > previous_recent))
                .collect(),
            flag_updates: flag_changes.changes,
            uid_next,
            cursor: flag_changes.cursor,
        })
    }

    /// Load sizes and dates of members.
    pub async fn metadata(
        &self,
        handle: &ImapMailboxHandle,
        uids: &[u32],
    ) -> Result<Vec<ImapMemberMetadata>, ImapAccessError> {
        let state = self.reconcile_handle(handle).await?;
        let result = self
            .storage
            .imap_member_metadata(state.identifier, uids)
            .await;
        self.current_account().await?;
        Ok(result?)
    }

    /// Load one member's octets.
    pub async fn content(
        &self,
        handle: &ImapMailboxHandle,
        uid: u32,
    ) -> Result<Option<ImapMemberContent>, ImapAccessError> {
        let state = self.reconcile_handle(handle).await?;
        let result = self
            .storage
            .imap_member_content(state.identifier, uid)
            .await;
        self.current_account().await?;
        Ok(result?)
    }

    /// Change flags of members; other sessions of the account are notified.
    pub async fn store(
        &self,
        handle: &ImapMailboxHandle,
        uids: &[u32],
        change: &ImapFlagChange,
    ) -> Result<Vec<(u32, ImapMessageFlags)>, ImapAccessError> {
        let (account, state, smart) = self.reconcile_handle_kind(handle).await?;
        if smart
            && (change.flags.answered
                || change.flags.flagged
                || change.flags.draft
                || change.flags.deleted
                || !change.flags.keywords.is_empty())
        {
            return Err(ImapAccessError::NotPermitted(
                "smart folders allow changes to \\Seen only".into(),
            ));
        }
        let seen_change = ImapFlagChange {
            operation: if change.operation == sandpost_storage::ImapFlagOperation::Replace {
                if change.flags.seen {
                    sandpost_storage::ImapFlagOperation::Add
                } else {
                    sandpost_storage::ImapFlagOperation::Remove
                }
            } else {
                change.operation
            },
            flags: ImapMessageFlags {
                seen: change.operation == sandpost_storage::ImapFlagOperation::Replace
                    || change.flags.seen,
                ..ImapMessageFlags::default()
            },
        };
        let change = if smart { &seen_change } else { change };
        let updated = self
            .storage
            .store_imap_flags(
                state.identifier,
                uids,
                change,
                Some(self.authenticated_mode_generation),
            )
            .await;
        if updated.is_ok() {
            self.notify_mailbox_change(account.mode);
        }
        self.current_account().await?;
        Ok(updated?)
    }

    /// Permanently delete flagged messages for the owning Sandpost user.
    pub async fn expunge(
        &self,
        handle: &ImapMailboxHandle,
        uids: Option<&[u32]>,
    ) -> Result<Vec<u32>, ImapAccessError> {
        let (account, state, smart) = self.reconcile_handle_kind(handle).await?;
        if smart {
            return Err(ImapAccessError::NotPermitted(
                "smart folders cannot be expunged".into(),
            ));
        }
        let owner = self
            .storage
            .get_user(self.owner_identifier)
            .await?
            .ok_or(ImapAccessError::Revoked)?;
        if owner.global_role == GlobalRole::Viewer {
            return Err(ImapAccessError::NotPermitted(
                "this account cannot delete mail".into(),
            ));
        }
        let removed = self
            .storage
            .expunge_imap_members(
                state.identifier,
                uids,
                Some(self.authenticated_mode_generation),
            )
            .await;
        if removed.as_ref().is_ok_and(|removed| !removed.is_empty()) {
            self.notify_mailbox_change(account.mode);
        }
        self.current_account().await?;
        Ok(removed?)
    }

    /// Copy or move members to another mailbox of the account.
    ///
    /// Manual, private, and general mailboxes accept only currently authorized members.
    /// Smart folders and Views refuse insertion.
    pub async fn copy(
        &self,
        handle: &ImapMailboxHandle,
        uids: &[u32],
        destination: &str,
        remove_from_source: bool,
    ) -> Result<ImapCopyResult, ImapAccessError> {
        let (account, catalog) = self.catalog().await?;
        let source_index = Self::find_handle(&catalog, handle)?;
        let source = &catalog[source_index].0;
        let source_smart = source.view.is_some()
            || source
                .folder
                .as_ref()
                .is_some_and(|folder| folder.kind == ImapFolderKind::Smart);
        if remove_from_source && source_smart {
            return Err(ImapAccessError::NotPermitted(
                "messages cannot be moved out of smart folders".into(),
            ));
        }
        self.reconcile(&account, &catalog, source_index).await?;
        let index = Self::find(&catalog, destination)?;
        let (target, _) = &catalog[index];
        if target.view.is_some()
            || target
                .folder
                .as_ref()
                .is_some_and(|folder| folder.kind == ImapFolderKind::Smart)
        {
            return Err(ImapAccessError::NotPermitted(
                "smart folders accept only messages selected by their filters".into(),
            ));
        }
        if remove_from_source && target.source == ImapMailboxSource::Trash {
            let owner = self
                .storage
                .get_user(self.owner_identifier)
                .await?
                .ok_or(ImapAccessError::Revoked)?;
            if owner.global_role == GlobalRole::Viewer {
                return Err(ImapAccessError::NotPermitted(
                    "this account cannot move mail to Trash".into(),
                ));
            }
        }
        let target_state = self.reconcile(&account, &catalog, index).await?;
        let definition = {
            let planned: Vec<PlannedMailbox> =
                catalog.iter().map(|(mailbox, _)| mailbox.clone()).collect();
            let authorization =
                catalog::owner_authorization(&self.storage, self.owner_identifier).await?;
            Some(catalog::definition(
                &account,
                &planned,
                target,
                &authorization,
            ))
        };
        let result = self
            .storage
            .copy_imap_members(
                handle.identifier,
                uids,
                target_state.identifier,
                definition.as_ref(),
                remove_from_source,
            )
            .await;
        if result.is_ok() {
            self.notify_mailbox_change(account.mode);
        }
        self.current_account().await?;
        result.map_err(|error| match error {
            StorageError::ConstraintViolation(message) => ImapAccessError::NotPermitted(message),
            StorageError::NotFound => ImapAccessError::NonExistent,
            error => error.into(),
        })
    }
}
