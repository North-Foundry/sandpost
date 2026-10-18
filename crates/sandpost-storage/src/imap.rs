//! IMAP storage capability and domain-specific request and result types.

mod types;

pub use types::{
    ImapAccountCredential, ImapAccountSettings, ImapCopyResult, ImapFlagChange, ImapFlagChanges,
    ImapFlagOperation, ImapLocalFolderSettings, ImapMailboxCounts, ImapMailboxState, ImapMember,
    ImapMemberContent, ImapMemberMetadata, ImapMembershipDefinition, NewImapAccount,
    NewImapLocalFolder, WebFolderSettings,
};

use crate::{FilterExpression, MessageListQuery, MessageSummary, StorageError};
use async_trait::async_trait;
use sandpost_core::{
    ImapAccount, ImapAccountIdentifier, ImapFolderIdentifier, ImapLinkedView, ImapLocalFolder,
    ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags, MessageIdentifier, UserIdentifier,
    WebFolder,
};

/// IMAP accounts, their configuration, mailbox identities, membership, and flags.
/// Session generation fences require an enabled credential at the captured generation,
/// checked in the same snapshot or transaction as the operation. A rejected fence returns
/// NotFound without changes. None preserves trusted, unfenced backend-call behavior;
/// mailbox and folder operations still refuse dormant-mode handles.
/// Folder definition configuration requires the captured generation and active mode,
/// but remains available to the Web owner while the IMAP credential is disabled.
#[async_trait]
pub trait ImapStorage: Send + Sync {
    /// Create an enabled account and snapshot authorized current primary history and organization.
    /// A taken username fails with DuplicateImapUsername.
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

    /// Replace name/status/mode, preserving dormant state and advancing the mode generation.
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

    /// Delete private configuration, both mode catalogs, flags and retained references.
    /// Primary state is untouched; content with no remaining view may be collected.
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

    /// Create an account-local Independent smart definition, or a legacy primary Mirror definition.
    /// A name used in that namespace fails with DuplicateImapFolderName.
    async fn create_imap_local_folder(
        &self,
        folder: &NewImapLocalFolder,
    ) -> Result<ImapLocalFolder, StorageError>;

    /// Load an account folder projection with its current canonical owner definition.
    async fn get_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<Option<ImapLocalFolder>, StorageError>;

    /// List the active mode's folder definitions, ordered by name and identifier.
    async fn list_imap_local_folders(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<ImapLocalFolder>, StorageError>;

    /// Replace a folder definition in its organization namespace; filter edits reconcile dynamically.
    async fn update_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
        settings: &ImapLocalFolderSettings,
    ) -> Result<Option<ImapLocalFolder>, StorageError>;

    /// Delete the folder in its owning view; normal occurrences retain copies or enter Trash.
    async fn delete_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
        expected_mode_generation: Option<u64>,
    ) -> Result<bool, StorageError>;

    /// Create an Independent private folder, or a shared primary Web folder in Mirror.
    async fn create_imap_private_folder(
        &self,
        account: ImapAccountIdentifier,
        name: &str,
        created_at: i64,
        expected_mode_generation: Option<u64>,
    ) -> Result<ImapLocalFolder, StorageError>;

    /// Create a Web manual folder and expose it to the listed accounts atomically.
    async fn create_web_folder(
        &self,
        owner: UserIdentifier,
        name: &str,
        icon: Option<&str>,
        accounts: &[ImapAccountIdentifier],
        created_at: i64,
    ) -> Result<WebFolder, StorageError>;

    /// Load a Web folder and its selected account exposures.
    async fn get_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<Option<WebFolder>, StorageError>;

    /// List a user's Web folders in stable name and identifier order.
    async fn list_web_folders(&self, owner: UserIdentifier)
    -> Result<Vec<WebFolder>, StorageError>;

    /// Replace a Web folder's name and exposures.
    async fn update_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
        name: &str,
        icon: Option<&str>,
        accounts: &[ImapAccountIdentifier],
    ) -> Result<Option<WebFolder>, StorageError>;

    /// Delete a primary folder and its Mirror projections, trashing otherwise-unfiled occurrences.
    async fn delete_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<bool, StorageError>;

    /// Place or unfile up to 256 authorized messages in one owner's Web mailbox.
    ///
    /// Placing a message in a folder or the general mailbox also restores it from the owner's
    /// Web Trash.
    async fn place_user_messages(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        folder: Option<ImapFolderIdentifier>,
        authorization: &FilterExpression,
    ) -> Result<u64, StorageError>;

    /// Copy up to 256 distinct authorized, untombstoned messages into Inbox or an owned folder.
    /// Preserve primary placement and Trash; reject any existing destination occurrence atomically.
    /// Unknown owners, destinations, or inaccessible messages return NotFound.
    async fn copy_user_messages(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        folder: Option<ImapFolderIdentifier>,
        authorization: &FilterExpression,
    ) -> Result<u64, StorageError>;

    /// Atomically change shared primary flags on up to 256 currently authorized messages.
    /// Deleted is occurrence-only and refused; keywords must be bounded valid IMAP atoms.
    /// Return the number of distinct messages whose supported flags actually changed.
    async fn set_user_message_flags(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        change: &ImapFlagChange,
        authorization: &FilterExpression,
    ) -> Result<u64, StorageError>;

    /// List authorized messages in the Web general mailbox (`None`) or a manual folder.
    ///
    /// Messages in the owner's Web Trash are excluded from every such listing.
    async fn list_web_mailbox_messages(
        &self,
        owner: UserIdentifier,
        folder: Option<ImapFolderIdentifier>,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Move up to 256 authorized, undeleted messages into one owner's Web Trash.
    ///
    /// Trash is exclusive with Web folder placement: a trashed message loses its folder placement,
    /// leaves every Web mailbox listing and user search, and projects into the Trash of each of
    /// the owner's IMAP accounts that did not place it elsewhere. Already trashed messages keep
    /// their original time. Return the number of authorized messages processed.
    async fn trash_user_messages(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        authorization: &FilterExpression,
        trashed_at: i64,
    ) -> Result<u64, StorageError>;

    /// List authorized, undeleted messages in one owner's Web Trash, newest first with keysets.
    async fn list_web_trash_messages(
        &self,
        owner: UserIdentifier,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Set shared Seen for up to 256 currently authorized, undeleted messages.
    async fn set_user_message_seen(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        seen: bool,
        authorization: &FilterExpression,
    ) -> Result<u64, StorageError>;

    /// Tombstone every authorized message in one owner's Web Trash and return new tombstones.
    async fn empty_user_trash(
        &self,
        owner: UserIdentifier,
        authorization: &FilterExpression,
    ) -> Result<u64, StorageError>;

    /// Reconcile active-mode identities, preserving all dormant-mode identities and counters.
    /// Missing active sources receive fresh UIDVALIDITY; unlisted live identities are retained.
    /// Return states in request order.
    /// Local folders must belong to the account; otherwise NotFound and nothing changes.
    async fn reconcile_imap_mailboxes(
        &self,
        account: ImapAccountIdentifier,
        sources: &[ImapMailboxSource],
        expected_mode_generation: Option<u64>,
    ) -> Result<Vec<ImapMailboxState>, StorageError>;

    /// Load one mailbox identity.
    async fn get_imap_mailbox(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<ImapMailboxState>, StorageError>;

    /// Atomically bring membership in line with the definition and return the new state.
    ///
    /// Dynamic mailboxes gain matching retained and currently authorized messages,
    /// in ascending message order with new UIDs, and lose members that no longer match. The
    /// Trash and COPY targets restore authorized membership from durable placement state.
    /// Returns None for an unknown mailbox.
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
        expected_mode_generation: Option<u64>,
    ) -> Result<Vec<(u32, ImapMessageFlags)>, StorageError>;

    /// Remove members flagged deleted, restricted to `uids` when given, and return their UIDs.
    ///
    /// Remove only the active view's flagged occurrences. Promote surviving copies, move a final
    /// normal occurrence into its Trash, and permanently release a final Trash occurrence.
    /// Preserve primary/peer state for Independent; collect physical content only after all views
    /// release it. COPY then source EXPUNGE transfers without duplicating physical mail.
    async fn expunge_imap_members(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: Option<&[u32]>,
        expected_mode_generation: Option<u64>,
    ) -> Result<Vec<u32>, StorageError>;

    /// Copy members to another mailbox of the same account, removing them from the source when
    /// `remove_from_source`.
    ///
    /// Smart destinations are refused. Writable destinations require a current authorization
    /// definition when source members exist. Refused operations change nothing. Successful copies
    /// allocate fresh destination UIDs and preserve flags. Cross-mailbox Trash transfers coalesce
    /// existing occurrences, preserving their UIDs and setting `all_new` false so no COPYUID is
    /// advertised for a coalesced result. Other existing destination occurrences are refused with
    /// ConstraintViolation. MOVE changes only the source account organization; moving
    /// an additional COPY occurrence preserves the unrelated primary placement and other copies.
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

    /// Retain default subscriptions for live names without overriding explicit unsubscribes.
    async fn retain_imap_subscriptions(
        &self,
        account: ImapAccountIdentifier,
        names: &[String],
    ) -> Result<(), StorageError>;

    /// The names the account unsubscribed from.
    async fn list_imap_unsubscribed(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<String>, StorageError>;

    /// List retained subscribed names (explicit and observed defaults), including absent names.
    async fn list_imap_subscribed(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<String>, StorageError>;

    /// Forget mailbox exclusions from membership-only moves so filters may show those
    /// messages again; returns how many were forgotten. User tombstones are never cleared.
    async fn clear_imap_exclusions(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<u64, StorageError>;
}
