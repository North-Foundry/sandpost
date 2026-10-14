//! Asynchronous SQLite persistence for IMAP accounts and virtual mailboxes.

mod accounts;
mod flags;
mod folders;
mod mailboxes;
mod members;
mod membership;
pub(crate) mod organization;
mod placements;
mod shared;
mod subscriptions;
mod web_folders;

#[cfg(test)]
mod read_status_tests;

use crate::SqliteStorage;
use crate::error::StorageResult;
use async_trait::async_trait;
use sandpost_core::{
    ImapAccount, ImapAccountIdentifier, ImapFolderIdentifier, ImapLinkedView, ImapLocalFolder,
    ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags, MessageIdentifier, UserIdentifier,
};
use sandpost_storage::{
    ImapAccountCredential, ImapAccountSettings, ImapCopyResult, ImapFlagChange, ImapFlagChanges,
    ImapLocalFolderSettings, ImapMailboxCounts, ImapMailboxState, ImapMember, ImapMemberContent,
    ImapMemberMetadata, ImapMembershipDefinition, ImapStorage, MessageListQuery, MessageSummary,
    NewImapAccount, NewImapLocalFolder, StorageError,
};

#[async_trait]
impl ImapStorage for SqliteStorage {
    /// Create an enabled account and reject duplicate usernames atomically.
    async fn create_imap_account(
        &self,
        account: &NewImapAccount,
    ) -> Result<ImapAccount, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = accounts::create_account(&mut transaction, account).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Load one account without its password hash.
    async fn get_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<Option<ImapAccount>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        accounts::get_account(&mut connection, identifier).await
    }

    /// List an owner's accounts by name and identifier without password hashes.
    async fn list_imap_accounts(
        &self,
        owner: UserIdentifier,
    ) -> Result<Vec<ImapAccount>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        accounts::list_accounts(&mut connection, owner).await
    }

    /// Load credentials by exact username for authentication.
    async fn get_imap_account_credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<ImapAccountCredential>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        accounts::get_credential(&mut connection, "username", username).await
    }

    /// Load credentials by identifier for session revalidation.
    async fn get_imap_account_credential(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<Option<ImapAccountCredential>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        accounts::get_credential(&mut connection, "identifier", &identifier.to_string()).await
    }

    /// Replace an account's display name and enabled or mirror settings.
    async fn update_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
        settings: &ImapAccountSettings,
    ) -> Result<Option<ImapAccount>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = accounts::update_account(&mut transaction, identifier, settings).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Replace an account password hash and return the updated public account.
    async fn replace_imap_account_password(
        &self,
        identifier: ImapAccountIdentifier,
        password_hash: &str,
    ) -> Result<Option<ImapAccount>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result =
            accounts::replace_password(&mut transaction, identifier, password_hash).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Delete the account and its cascading IMAP state without deleting mail.
    async fn delete_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<bool, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = accounts::delete_account(&mut transaction, identifier).await?;
        organization::collect_unreferenced_messages(&mut transaction).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Record successful account use, skipping writes within the timestamp resolution window.
    async fn record_imap_account_use(
        &self,
        identifier: ImapAccountIdentifier,
        used_at: i64,
    ) -> Result<(), StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        accounts::record_account_use(&mut transaction, identifier, used_at).await?;
        transaction.commit().await.storage()
    }

    /// Replace an account's linked View selection atomically.
    async fn set_imap_linked_views(
        &self,
        account: ImapAccountIdentifier,
        views: &[ImapLinkedView],
    ) -> Result<(), StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        accounts::set_linked_views(&mut transaction, account, views).await?;
        transaction.commit().await.storage()
    }

    /// List linked Views ordered by View identifier.
    async fn list_imap_linked_views(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<ImapLinkedView>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        accounts::list_linked_views(&mut connection, account).await
    }

    /// Create a local folder after checking the account and name uniqueness.
    async fn create_imap_local_folder(
        &self,
        folder: &NewImapLocalFolder,
    ) -> Result<ImapLocalFolder, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_account_configuration_generation(
            &mut transaction,
            folder.account_identifier,
            folder.expected_mode_generation,
        )
        .await?;
        let result = folders::create_folder(&mut transaction, folder).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Load a local folder by identifier.
    async fn get_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<Option<ImapLocalFolder>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        folders::get_folder(&mut connection, identifier).await
    }

    /// List an account's local folders by name and identifier.
    async fn list_imap_local_folders(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<ImapLocalFolder>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        folders::list_folders(&mut connection, account).await
    }

    /// Replace a local folder's settings and update its modification time.
    async fn update_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
        settings: &ImapLocalFolderSettings,
    ) -> Result<Option<ImapLocalFolder>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_folder_configuration_generation(
            &mut transaction,
            identifier,
            settings.expected_mode_generation,
        )
        .await?;
        let result = folders::update_folder(&mut transaction, identifier, settings).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Delete a local folder and its cascading mailbox state.
    async fn delete_imap_local_folder(
        &self,
        identifier: ImapFolderIdentifier,
        expected_mode_generation: Option<u64>,
    ) -> Result<bool, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_folder_configuration_generation(
            &mut transaction,
            identifier,
            expected_mode_generation,
        )
        .await?;
        let result = folders::delete_folder(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Create a private folder owned by one IMAP account.
    async fn create_imap_private_folder(
        &self,
        account: ImapAccountIdentifier,
        name: &str,
        created_at: i64,
        expected_mode_generation: Option<u64>,
    ) -> Result<ImapLocalFolder, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_account_generation(
            &mut transaction,
            account,
            expected_mode_generation,
        )
        .await?;
        let result =
            folders::create_private_folder(&mut transaction, account, name, created_at).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Create a Web manual folder with its selected account projections.
    async fn create_web_folder(
        &self,
        owner: UserIdentifier,
        name: &str,
        accounts: &[ImapAccountIdentifier],
        created_at: i64,
    ) -> Result<sandpost_core::WebFolder, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result =
            web_folders::create_folder(&mut transaction, owner, name, accounts, created_at).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Read one Web folder and its account exposures.
    async fn get_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<Option<sandpost_core::WebFolder>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let result = web_folders::get_folder(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// List Web manual folders owned by one user.
    async fn list_web_folders(
        &self,
        owner: UserIdentifier,
    ) -> Result<Vec<sandpost_core::WebFolder>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let result = web_folders::list_folders(&mut transaction, owner).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Update one Web folder's name and exposed accounts.
    async fn update_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
        name: &str,
        accounts: &[ImapAccountIdentifier],
    ) -> Result<Option<sandpost_core::WebFolder>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result =
            web_folders::update_folder(&mut transaction, identifier, name, accounts).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Delete a Web folder and its account projections.
    async fn delete_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<bool, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = web_folders::delete_folder(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Place authorized, non-deleted mail in one owner's Web mailbox.
    async fn place_user_messages(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        folder: Option<ImapFolderIdentifier>,
        authorization: &sandpost_storage::FilterExpression,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = placements::place_user_messages(
            &mut transaction,
            owner,
            identifiers,
            folder,
            authorization,
        )
        .await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Atomically copy primary messages without changing their source occurrences.
    async fn copy_user_messages(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        folder: Option<ImapFolderIdentifier>,
        authorization: &sandpost_storage::FilterExpression,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let copied = placements::copy_user_messages(
            &mut transaction,
            owner,
            identifiers,
            folder,
            authorization,
        )
        .await?;
        transaction.commit().await.storage()?;
        Ok(copied)
    }

    /// Atomically update supported primary flags and every Mirror modification cursor.
    async fn set_user_message_flags(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        change: &ImapFlagChange,
        authorization: &sandpost_storage::FilterExpression,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let changed =
            flags::set_user_flags(&mut transaction, owner, identifiers, change, authorization)
                .await?;
        transaction.commit().await.storage()?;
        Ok(changed)
    }

    /// List one Web mailbox in the caller's authorized, non-deleted message set.
    async fn list_web_mailbox_messages(
        &self,
        owner: UserIdentifier,
        folder: Option<ImapFolderIdentifier>,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        if let Some(identifier) = folder {
            let definition = web_folders::get_folder(&mut transaction, identifier)
                .await?
                .ok_or(StorageError::NotFound)?;
            if definition.owner_identifier != owner {
                return Err(StorageError::NotFound);
            }
        }
        let result = crate::messages::summaries::list_web_mailbox_messages(
            &mut transaction,
            owner,
            folder,
            query,
        )
        .await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Set shared Seen for currently authorized, undeleted messages in one atomic batch.
    async fn set_user_message_seen(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        seen: bool,
        authorization: &sandpost_storage::FilterExpression,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let changed =
            flags::set_user_seen(&mut transaction, owner, identifiers, seen, authorization).await?;
        transaction.commit().await.storage()?;
        Ok(changed)
    }

    /// Move authorized, non-deleted mail into one owner's Web Trash.
    async fn trash_user_messages(
        &self,
        owner: UserIdentifier,
        identifiers: &[MessageIdentifier],
        authorization: &sandpost_storage::FilterExpression,
        trashed_at: i64,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = placements::trash_user_messages(
            &mut transaction,
            owner,
            identifiers,
            authorization,
            trashed_at,
        )
        .await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// List one owner's Web Trash with the authorization filter and tombstones applied in SQL.
    async fn list_web_trash_messages(
        &self,
        owner: UserIdentifier,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let result =
            crate::messages::summaries::list_web_trash_messages(&mut transaction, owner, query)
                .await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Permanently delete one owner's authorized Web Trash in one transaction.
    async fn empty_user_trash(
        &self,
        owner: UserIdentifier,
        authorization: &sandpost_storage::FilterExpression,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = placements::empty_user_trash(&mut transaction, owner, authorization).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Reconcile mailbox identities to the supplied ordered source list.
    async fn reconcile_imap_mailboxes(
        &self,
        account: ImapAccountIdentifier,
        sources: &[ImapMailboxSource],
        expected_mode_generation: Option<u64>,
    ) -> Result<Vec<ImapMailboxState>, StorageError> {
        {
            let mut transaction = self.read_pool.begin().await.storage()?;
            organization::require_account_generation(
                &mut transaction,
                account,
                expected_mode_generation,
            )
            .await?;
            let current = accounts::get_account(&mut transaction, account)
                .await?
                .ok_or(StorageError::NotFound)?;
            let sql = format!(
                "SELECT {} FROM imap_mailboxes WHERE account_identifier=?1 AND organization_mode=?2",
                shared::MAILBOX_COLUMNS
            );
            let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(account.to_string())
                .bind(current.mode.as_str())
                .fetch_all(&mut *transaction)
                .await
                .storage()?;
            let existing = rows
                .iter()
                .map(shared::decode_mailbox)
                .collect::<Result<Vec<_>, _>>()
                .storage()?;
            let unchanged = sources
                .iter()
                .map(|source| {
                    existing
                        .iter()
                        .find(|state| state.source == *source)
                        .copied()
                })
                .collect::<Option<Vec<_>>>();
            transaction.commit().await.storage()?;
            if let Some(states) = unchanged {
                return Ok(states);
            }
        }
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_account_generation(
            &mut transaction,
            account,
            expected_mode_generation,
        )
        .await?;
        let result = mailboxes::reconcile_mailboxes(&mut transaction, account, sources).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Load one mailbox identity.
    async fn get_imap_mailbox(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<ImapMailboxState>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        mailboxes::get_mailbox(&mut connection, identifier).await
    }

    /// Synchronize a mailbox's materialized membership with its definition.
    async fn synchronize_imap_mailbox(
        &self,
        identifier: ImapMailboxIdentifier,
        definition: &ImapMembershipDefinition,
    ) -> Result<Option<ImapMailboxState>, StorageError> {
        let expected = definition.expected_mode_generation;
        {
            let mut transaction = self.read_pool.begin().await.storage()?;
            organization::require_mailbox_generation(&mut transaction, identifier, expected)
                .await?;
            let state = mailboxes::get_mailbox(&mut transaction, identifier).await?;
            let unchanged: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM imap_mailboxes WHERE identifier=?1 AND definition_fingerprint=?2 AND examined_sequence=(SELECT coalesce(max(sequence),0) FROM mail))")
                .bind(identifier.to_string()).bind(&definition.fingerprint).fetch_one(&mut *transaction).await.storage()?;
            transaction.commit().await.storage()?;
            if unchanged || state.is_none() {
                return Ok(state);
            }
        }
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_mailbox_generation(&mut transaction, identifier, expected).await?;
        let result =
            mailboxes::synchronize_mailbox(&mut transaction, identifier, definition).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Load members above a UID in ascending order with their current flags.
    async fn imap_mailbox_members(
        &self,
        identifier: ImapMailboxIdentifier,
        after_uid: u32,
    ) -> Result<Vec<ImapMember>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        members::members(&mut connection, identifier, after_uid).await
    }

    /// Count members at or below the UID watermark.
    async fn count_imap_mailbox_members(
        &self,
        identifier: ImapMailboxIdentifier,
        through_uid: u32,
    ) -> Result<u64, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        members::count_members(&mut connection, identifier, through_uid).await
    }

    /// List member UIDs at or below the watermark in ascending order.
    async fn imap_mailbox_uids(
        &self,
        identifier: ImapMailboxIdentifier,
        through_uid: u32,
    ) -> Result<Vec<u32>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        members::member_uids(&mut connection, identifier, through_uid).await
    }

    /// Load changed flags for members after the account modification cursor.
    async fn imap_flag_changes(
        &self,
        identifier: ImapMailboxIdentifier,
        after_cursor: u64,
        through_uid: u32,
    ) -> Result<ImapFlagChanges, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let result =
            flags::flag_changes(&mut transaction, identifier, after_cursor, through_uid).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Read the current flag modification cursor for an account.
    async fn imap_flag_cursor(&self, account: ImapAccountIdentifier) -> Result<u64, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        flags::flag_cursor(&mut connection, account).await
    }

    /// Claim all current members as recent and return the previous watermark.
    async fn claim_imap_recent(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<u32>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = members::claim_recent(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Count total, unseen, and recent mailbox members from one read snapshot.
    async fn imap_mailbox_counts(
        &self,
        identifier: ImapMailboxIdentifier,
    ) -> Result<Option<ImapMailboxCounts>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let result = members::counts(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Load metadata for requested member UIDs, omitting UIDs that are not members.
    async fn imap_member_metadata(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: &[u32],
    ) -> Result<Vec<ImapMemberMetadata>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let result = members::metadata(&mut transaction, identifier, uids).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Load the stored content for one member UID.
    async fn imap_member_content(
        &self,
        identifier: ImapMailboxIdentifier,
        uid: u32,
    ) -> Result<Option<ImapMemberContent>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        members::content(&mut connection, identifier, uid).await
    }

    /// Apply a flag change to selected members and return resulting flags.
    async fn store_imap_flags(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: &[u32],
        change: &ImapFlagChange,
        expected_mode_generation: Option<u64>,
    ) -> Result<Vec<(u32, ImapMessageFlags)>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_mailbox_generation(
            &mut transaction,
            identifier,
            expected_mode_generation,
        )
        .await?;
        let result = flags::store_flags(&mut transaction, identifier, uids, change).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Tombstone flagged members for the owner, or transfer primary occurrences that survive as copies.
    async fn expunge_imap_members(
        &self,
        identifier: ImapMailboxIdentifier,
        uids: Option<&[u32]>,
        expected_mode_generation: Option<u64>,
    ) -> Result<Vec<u32>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_mailbox_generation(
            &mut transaction,
            identifier,
            expected_mode_generation,
        )
        .await?;
        let result = membership::expunge(&mut transaction, identifier, uids).await?;
        transaction.commit().await.storage()?;
        Ok(result)
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
        let expected =
            destination_definition.and_then(|definition| definition.expected_mode_generation);
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        organization::require_mailbox_generation(&mut transaction, source, expected).await?;
        organization::require_mailbox_generation(&mut transaction, destination, expected).await?;
        let result = membership::copy(
            &mut transaction,
            source,
            uids,
            destination,
            destination_definition,
            remove_from_source,
        )
        .await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }

    /// Set the subscription state for a mailbox name.
    async fn set_imap_subscription(
        &self,
        account: ImapAccountIdentifier,
        name: &str,
        subscribed: bool,
    ) -> Result<(), StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        subscriptions::set_subscription(&mut transaction, account, name, subscribed).await?;
        transaction.commit().await.storage()
    }

    /// Retain default subscriptions for live names without overriding explicit unsubscribes.
    async fn retain_imap_subscriptions(
        &self,
        account: ImapAccountIdentifier,
        names: &[String],
    ) -> Result<(), StorageError> {
        if names.is_empty() {
            return Ok(());
        }
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        subscriptions::retain_subscriptions(&mut transaction, account, names).await?;
        transaction.commit().await.storage()
    }

    /// List mailbox names the account has unsubscribed from.
    async fn list_imap_unsubscribed(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<String>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        subscriptions::list_unsubscribed(&mut connection, account).await
    }

    /// List retained subscribed names (explicit and observed defaults), including absent names.
    async fn list_imap_subscribed(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<Vec<String>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        subscriptions::list_subscribed(&mut connection, account).await
    }

    /// Clear recorded expunges for an account and force full mailbox reconciliation.
    async fn clear_imap_exclusions(
        &self,
        account: ImapAccountIdentifier,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let result = membership::clear_exclusions(&mut transaction, account).await?;
        transaction.commit().await.storage()?;
        Ok(result)
    }
}
