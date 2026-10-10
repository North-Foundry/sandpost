//! SQLite persistence for IMAP accounts and virtual mailboxes.
//!
//! Each child module owns blocking SQL operations for one responsibility. This module is the
//! single adapter from the asynchronous IMAP storage contract to those helpers.

mod accounts;
mod flags;
mod folders;
mod mailboxes;
mod members;
mod membership;
mod shared;
mod subscriptions;

use crate::SqliteStorage;
use async_trait::async_trait;
use sandpost_core::{
    ImapAccount, ImapAccountIdentifier, ImapFolderIdentifier, ImapLinkedView, ImapLocalFolder,
    ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags, UserIdentifier,
};
use sandpost_storage::{
    ImapAccountCredential, ImapAccountSettings, ImapCopyResult, ImapFlagChange, ImapFlagChanges,
    ImapLocalFolderSettings, ImapMailboxCounts, ImapMailboxState, ImapMember, ImapMemberContent,
    ImapMemberMetadata, ImapMembershipDefinition, ImapStorage, NewImapAccount, NewImapLocalFolder,
    StorageError,
};

#[async_trait]
impl ImapStorage for SqliteStorage {
    /// Create an enabled account and reject duplicate usernames atomically.
    async fn create_imap_account(
        &self,
        account: &NewImapAccount,
    ) -> Result<ImapAccount, StorageError> {
        let account = account.clone();
        self.run(move |connection| accounts::create_account_blocking(connection, &account))
            .await
    }

    /// Load one account without its password hash.
    async fn get_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<Option<ImapAccount>, StorageError> {
        self.run(move |connection| accounts::get_account_blocking(connection, identifier))
            .await
    }

    /// List an owner's accounts by name and identifier without password hashes.
    async fn list_imap_accounts(
        &self,
        owner: UserIdentifier,
    ) -> Result<Vec<ImapAccount>, StorageError> {
        self.run(move |connection| accounts::list_accounts_blocking(connection, owner))
            .await
    }

    /// Load credentials by exact username for authentication.
    async fn get_imap_account_credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<ImapAccountCredential>, StorageError> {
        let username = username.to_owned();
        self.run(move |connection| {
            accounts::get_credential_blocking(connection, "username", &username)
        })
        .await
    }

    /// Load credentials by identifier for session revalidation.
    async fn get_imap_account_credential(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<Option<ImapAccountCredential>, StorageError> {
        self.run(move |connection| {
            accounts::get_credential_blocking(connection, "identifier", &identifier.to_string())
        })
        .await
    }

    /// Replace an account's display name and enabled or mirror settings.
    async fn update_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
        settings: &ImapAccountSettings,
    ) -> Result<Option<ImapAccount>, StorageError> {
        let settings = settings.clone();
        self.run(move |connection| {
            accounts::update_account_blocking(connection, identifier, &settings)
        })
        .await
    }

    /// Replace an account password hash and return the updated public account.
    async fn replace_imap_account_password(
        &self,
        identifier: ImapAccountIdentifier,
        password_hash: &str,
    ) -> Result<Option<ImapAccount>, StorageError> {
        let password_hash = password_hash.to_owned();
        self.run(move |connection| {
            accounts::replace_password_blocking(connection, identifier, &password_hash)
        })
        .await
    }

    /// Delete the account and its cascading IMAP state without deleting mail.
    async fn delete_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<bool, StorageError> {
        self.run(move |connection| accounts::delete_account_blocking(connection, identifier))
            .await
    }

    /// Record successful account use, skipping writes within the timestamp resolution window.
    async fn record_imap_account_use(
        &self,
        identifier: ImapAccountIdentifier,
        used_at: i64,
    ) -> Result<(), StorageError> {
        self.run(move |connection| {
            accounts::record_account_use_blocking(connection, identifier, used_at)
        })
        .await
    }

    /// Replace an account's linked View selection atomically.
    async fn set_imap_linked_views(
        &self,
        account: ImapAccountIdentifier,
        views: &[ImapLinkedView],
    ) -> Result<(), StorageError> {
        let views = views.to_vec();
        self.run(move |connection| accounts::set_linked_views_blocking(connection, account, &views))
            .await
    }

    /// List linked Views ordered by View identifier.
    async fn list_imap_linked_views(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<ImapLinkedView>, StorageError> {
        self.run(move |connection| accounts::list_linked_views_blocking(connection, account))
            .await
    }

    /// Create a local folder after checking the account and name uniqueness.
    async fn create_imap_local_folder(
        &self,
        folder: &NewImapLocalFolder,
    ) -> Result<ImapLocalFolder, StorageError> {
        let folder = folder.clone();
        self.run(move |connection| folders::create_folder_blocking(connection, &folder))
            .await
    }

    /// Load a local folder by identifier.
    async fn get_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<Option<ImapLocalFolder>, StorageError> {
        self.run(move |connection| folders::get_folder_blocking(connection, identifier))
            .await
    }

    /// List an account's local folders by name and identifier.
    async fn list_imap_local_folders(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<ImapLocalFolder>, StorageError> {
        self.run(move |connection| folders::list_folders_blocking(connection, account))
            .await
    }

    /// Replace a local folder's settings and update its modification time.
    async fn update_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
        settings: &ImapLocalFolderSettings,
    ) -> Result<Option<ImapLocalFolder>, StorageError> {
        let settings = settings.clone();
        self.run(move |connection| {
            folders::update_folder_blocking(connection, identifier, &settings)
        })
        .await
    }

    /// Delete a local folder and its cascading mailbox state.
    async fn delete_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<bool, StorageError> {
        self.run(move |connection| folders::delete_folder_blocking(connection, identifier))
            .await
    }

    /// Reconcile mailbox identities to the supplied ordered source list.
    async fn reconcile_imap_mailboxes(
        &self,
        account: ImapAccountIdentifier,
        sources: &[ImapMailboxSource],
    ) -> Result<Vec<ImapMailboxState>, StorageError> {
        let sources = sources.to_vec();
        self.run(move |connection| {
            mailboxes::reconcile_mailboxes_blocking(connection, account, &sources)
        })
        .await
    }

    /// Load one mailbox identity.
    async fn get_imap_mailbox(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<ImapMailboxState>, StorageError> {
        self.run(move |connection| mailboxes::get_mailbox_blocking(connection, identifier))
            .await
    }

    /// Synchronize a mailbox's materialized membership with its definition.
    async fn synchronize_imap_mailbox(
        &self,
        identifier: ImapMailboxIdentifier,
        definition: &ImapMembershipDefinition,
    ) -> Result<Option<ImapMailboxState>, StorageError> {
        let definition = definition.clone();
        self.run(move |connection| {
            mailboxes::synchronize_mailbox_blocking(connection, identifier, &definition)
        })
        .await
    }

    /// Load members above a UID in ascending order with their current flags.
    async fn imap_mailbox_members(
        &self,
        identifier: ImapMailboxIdentifier,
        after_uid: u32,
    ) -> Result<Vec<ImapMember>, StorageError> {
        self.run(move |connection| members::members_blocking(connection, identifier, after_uid))
            .await
    }

    /// Count members at or below the UID watermark.
    async fn count_imap_mailbox_members(
        &self,
        identifier: ImapMailboxIdentifier,
        through_uid: u32,
    ) -> Result<u64, StorageError> {
        self.run(move |connection| {
            members::count_members_blocking(connection, identifier, through_uid)
        })
        .await
    }

    /// List member UIDs at or below the watermark in ascending order.
    async fn imap_mailbox_uids(
        &self,
        identifier: ImapMailboxIdentifier,
        through_uid: u32,
    ) -> Result<Vec<u32>, StorageError> {
        self.run(move |connection| {
            members::member_uids_blocking(connection, identifier, through_uid)
        })
        .await
    }

    /// Load changed flags for members after the account modification cursor.
    async fn imap_flag_changes(
        &self,
        identifier: ImapMailboxIdentifier,
        after_cursor: u64,
        through_uid: u32,
    ) -> Result<ImapFlagChanges, StorageError> {
        self.run(move |connection| {
            flags::flag_changes_blocking(connection, identifier, after_cursor, through_uid)
        })
        .await
    }

    /// Read the current flag modification cursor for an account.
    async fn imap_flag_cursor(&self, account: ImapAccountIdentifier) -> Result<u64, StorageError> {
        self.run(move |connection| flags::flag_cursor_blocking(connection, account))
            .await
    }

    /// Claim all current members as recent and return the previous watermark.
    async fn claim_imap_recent(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<u32>, StorageError> {
        self.run(move |connection| members::claim_recent_blocking(connection, identifier))
            .await
    }

    /// Count total, unseen, and recent mailbox members.
    async fn imap_mailbox_counts(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<ImapMailboxCounts>, StorageError> {
        self.run(move |connection| members::counts_blocking(connection, identifier))
            .await
    }

    /// Load metadata for requested member UIDs, omitting UIDs that are not members.
    async fn imap_member_metadata(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: &[u32],
    ) -> Result<Vec<ImapMemberMetadata>, StorageError> {
        let uids = uids.to_vec();
        self.run(move |connection| members::metadata_blocking(connection, identifier, &uids))
            .await
    }

    /// Load the stored content for one member UID.
    async fn imap_member_content(
        &self,
        identifier: ImapMailboxIdentifier,
        uid: u32,
    ) -> Result<Option<ImapMemberContent>, StorageError> {
        self.run(move |connection| members::content_blocking(connection, identifier, uid))
            .await
    }

    /// Apply a flag change to selected members and return resulting flags.
    async fn store_imap_flags(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: &[u32],
        change: &ImapFlagChange,
    ) -> Result<Vec<(u32, ImapMessageFlags)>, StorageError> {
        let uids = uids.to_vec();
        let change = change.clone();
        self.run(move |connection| {
            flags::store_flags_blocking(connection, identifier, &uids, &change)
        })
        .await
    }

    /// Remove deleted members, recording exclusions for dynamic mailboxes.
    async fn expunge_imap_members(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: Option<&[u32]>,
    ) -> Result<Vec<u32>, StorageError> {
        let uids = uids.map(<[u32]>::to_vec);
        self.run(move |connection| {
            membership::expunge_blocking(connection, identifier, uids.as_deref())
        })
        .await
    }

    /// Copy or move members between mailboxes belonging to the same account.
    async fn copy_imap_members(
        &self,
        source: ImapMailboxIdentifier,
        uids: &[u32],
        destination: ImapMailboxIdentifier,
        destination_definition: Option<&ImapMembershipDefinition>,
        remove_from_source: bool,
    ) -> Result<ImapCopyResult, StorageError> {
        let uids = uids.to_vec();
        let definition = destination_definition.cloned();
        self.run(move |connection| {
            membership::copy_blocking(
                connection,
                source,
                &uids,
                destination,
                definition.as_ref(),
                remove_from_source,
            )
        })
        .await
    }

    /// Set the subscription state for a mailbox name.
    async fn set_imap_subscription(
        &self,
        account: ImapAccountIdentifier,
        name: &str,
        subscribed: bool,
    ) -> Result<(), StorageError> {
        let name = name.to_owned();
        self.run(move |connection| {
            subscriptions::set_subscription_blocking(connection, account, &name, subscribed)
        })
        .await
    }

    /// List mailbox names the account has unsubscribed from.
    async fn list_imap_unsubscribed(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<String>, StorageError> {
        self.run(move |connection| subscriptions::list_unsubscribed_blocking(connection, account))
            .await
    }

    /// Clear recorded expunges for an account and force full mailbox reconciliation.
    async fn clear_imap_exclusions(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<u64, StorageError> {
        self.run(move |connection| membership::clear_exclusions_blocking(connection, account))
            .await
    }
}
