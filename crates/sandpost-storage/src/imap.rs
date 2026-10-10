//! IMAP storage capability and domain-specific request and result types.

mod types;

pub use types::{
    ImapAccountCredential, ImapAccountSettings, ImapCopyResult, ImapFlagChange, ImapFlagChanges,
    ImapFlagOperation, ImapLocalFolderSettings, ImapMailboxCounts, ImapMailboxState, ImapMember,
    ImapMemberContent, ImapMemberMetadata, ImapMembershipDefinition, NewImapAccount,
    NewImapLocalFolder,
};

use crate::StorageError;
use async_trait::async_trait;
use sandpost_core::{
    ImapAccount, ImapAccountIdentifier, ImapFolderIdentifier, ImapLinkedView, ImapLocalFolder,
    ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags, UserIdentifier,
};

/// IMAP accounts, their configuration, mailbox identities, membership, and flags.
#[async_trait]
pub trait ImapStorage: Send + Sync {
    /// Create an enabled account; a taken username fails with DuplicateImapUsername.
    async fn create_imap_account(
        &self,
        account: &NewImapAccount,
    ) -> Result<ImapAccount, StorageError>;

    /// Load one account by identifier, without its password hash.
    async fn get_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<Option<ImapAccount>, StorageError>;

    /// List one user's accounts ordered by name and identifier, without password hashes.
    async fn list_imap_accounts(
        &self,
        owner: UserIdentifier,
    ) -> Result<Vec<ImapAccount>, StorageError>;

    /// Load an account and its password hash by exact username for authentication.
    async fn get_imap_account_credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<ImapAccountCredential>, StorageError>;

    /// Load an account and its password hash by identifier for session revalidation.
    async fn get_imap_account_credential(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<Option<ImapAccountCredential>, StorageError>;

    /// Replace an account's name, enabled state, and mirror setting.
    async fn update_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
        settings: &ImapAccountSettings,
    ) -> Result<Option<ImapAccount>, StorageError>;

    /// Atomically replace an account's password hash; the old hash stops verifying at once.
    async fn replace_imap_account_password(
        &self,
        identifier: ImapAccountIdentifier,
        password_hash: &str,
    ) -> Result<Option<ImapAccount>, StorageError>;

    /// Delete an account with its configuration, mailboxes, memberships, and flags; mail is
    /// untouched.
    async fn delete_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<bool, StorageError>;

    /// Record a successful login; backends may skip writes within a minute of the stored time.
    async fn record_imap_account_use(
        &self,
        identifier: ImapAccountIdentifier,
        used_at: i64,
    ) -> Result<(), StorageError>;

    /// Replace the linked View selection atomically; an unknown View fails with NotFound.
    async fn set_imap_linked_views(
        &self,
        account: ImapAccountIdentifier,
        views: &[ImapLinkedView],
    ) -> Result<(), StorageError>;

    /// List the linked View selection ordered by View identifier.
    async fn list_imap_linked_views(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<ImapLinkedView>, StorageError>;

    /// Create a local folder; a name already used in the account fails with
    /// DuplicateImapFolderName.
    async fn create_imap_local_folder(
        &self,
        folder: &NewImapLocalFolder,
    ) -> Result<ImapLocalFolder, StorageError>;

    /// Load one local folder.
    async fn get_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<Option<ImapLocalFolder>, StorageError>;

    /// List an account's local folders ordered by name and identifier.
    async fn list_imap_local_folders(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<ImapLocalFolder>, StorageError>;

    /// Replace a local folder's settings; a filter change triggers full reconciliation of its
    /// mailbox at the next synchronization.
    async fn update_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
        settings: &ImapLocalFolderSettings,
    ) -> Result<Option<ImapLocalFolder>, StorageError>;

    /// Delete a local folder and its mailbox identity.
    async fn delete_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<bool, StorageError>;

    /// Ensure exactly the given sources have mailbox identities: create missing ones with a fresh
    /// UIDVALIDITY and delete those not listed. Returns the states in the order given.
    async fn reconcile_imap_mailboxes(
        &self,
        account: ImapAccountIdentifier,
        sources: &[ImapMailboxSource],
    ) -> Result<Vec<ImapMailboxState>, StorageError>;

    /// Load one mailbox identity.
    async fn get_imap_mailbox(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<ImapMailboxState>, StorageError>;

    /// Atomically bring membership in line with the definition and return the new state.
    ///
    /// Dynamic mailboxes gain every matching message (except those expunged from this mailbox)
    /// in ascending message order with new UIDs, and lose members that no longer match. The
    /// Trash only loses members that no longer match. Returns None for an unknown mailbox.
    async fn synchronize_imap_mailbox(
        &self,
        identifier: ImapMailboxIdentifier,
        definition: &ImapMembershipDefinition,
    ) -> Result<Option<ImapMailboxState>, StorageError>;

    /// Members with a UID above `after_uid`, ascending, with their current flags.
    async fn imap_mailbox_members(
        &self,
        identifier: ImapMailboxIdentifier,
        after_uid: u32,
    ) -> Result<Vec<ImapMember>, StorageError>;

    /// Count members with a UID at or below `through_uid`.
    async fn count_imap_mailbox_members(
        &self,
        identifier: ImapMailboxIdentifier,
        through_uid: u32,
    ) -> Result<u64, StorageError>;

    /// UIDs of members at or below `through_uid`, ascending.
    async fn imap_mailbox_uids(
        &self,
        identifier: ImapMailboxIdentifier,
        through_uid: u32,
    ) -> Result<Vec<u32>, StorageError>;

    /// Flags of members at or below `through_uid` that changed after `after_cursor`.
    async fn imap_flag_changes(
        &self,
        identifier: ImapMailboxIdentifier,
        after_cursor: u64,
        through_uid: u32,
    ) -> Result<ImapFlagChanges, StorageError>;

    /// The account's current flag cursor.
    async fn imap_flag_cursor(&self, account: ImapAccountIdentifier) -> Result<u64, StorageError>;

    /// Atomically mark every current member as reported recent and return the previous
    /// watermark; None for an unknown mailbox.
    async fn claim_imap_recent(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<u32>, StorageError>;

    /// Message, unseen, and recent counts.
    async fn imap_mailbox_counts(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<ImapMailboxCounts>, StorageError>;

    /// Sizes and dates of members; UIDs that are not members are omitted.
    async fn imap_member_metadata(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: &[u32],
    ) -> Result<Vec<ImapMemberMetadata>, StorageError>;

    /// The stored octets of one member, or None when it is not a member.
    async fn imap_member_content(
        &self,
        identifier: ImapMailboxIdentifier,
        uid: u32,
    ) -> Result<Option<ImapMemberContent>, StorageError>;

    /// Apply a flag change to members and return each existing member's resulting flags.
    async fn store_imap_flags(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: &[u32],
        change: &ImapFlagChange,
    ) -> Result<Vec<(u32, ImapMessageFlags)>, StorageError>;

    /// Remove members flagged deleted, restricted to `uids` when given, and return their UIDs.
    ///
    /// A removed member of a dynamic mailbox is remembered as expunged so its filter does not
    /// bring it back. The message itself is never deleted.
    async fn expunge_imap_members(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: Option<&[u32]>,
    ) -> Result<Vec<u32>, StorageError>;

    /// Copy members to another mailbox of the same account, removing them from the source when
    /// `remove_from_source`.
    ///
    /// A dynamic destination accepts only messages matching `destination_definition` (otherwise
    /// ConstraintViolation and nothing changes) and forgets earlier expunges of them. The Trash
    /// accepts any member. Messages already present keep their UID.
    async fn copy_imap_members(
        &self,
        source: ImapMailboxIdentifier,
        uids: &[u32],
        destination: ImapMailboxIdentifier,
        destination_definition: Option<&ImapMembershipDefinition>,
        remove_from_source: bool,
    ) -> Result<ImapCopyResult, StorageError>;

    /// Subscribe to or unsubscribe from a mailbox name; every name starts subscribed.
    async fn set_imap_subscription(
        &self,
        account: ImapAccountIdentifier,
        name: &str,
        subscribed: bool,
    ) -> Result<(), StorageError>;

    /// The names the account unsubscribed from.
    async fn list_imap_unsubscribed(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<String>, StorageError>;

    /// Forget every expunge recorded in the account's mailboxes so their filters may show those
    /// messages again; returns how many were forgotten.
    async fn clear_imap_exclusions(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<u64, StorageError>;
}
