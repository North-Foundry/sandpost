//! SQLx implementation of atomic mail writes, snapshot reads, summaries, and index projections.
mod indexing;
mod parts;
mod reads;
pub(crate) mod summaries;
mod tombstones;
mod writes;

#[cfg(test)]
mod concurrency_tests;

use crate::SqliteStorage;
use crate::error::StorageResult;
use async_trait::async_trait;
use sandpost_core::{
    Attachment, Message, MessageFacts, MessageIdentifier, MessageSequence, UserIdentifier,
};
use sandpost_storage::{
    FilterExpression, IndexedMessage, MessageListQuery, MessageStorage, MessageSummary,
    StorageError,
};

#[async_trait]
impl MessageStorage for SqliteStorage {
    /// Persist mail and child facts atomically with one search revision.
    async fn insert_message(&self, message: &Message) -> Result<MessageSequence, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let sequence = writes::insert_message(&mut transaction, message).await?;
        transaction.commit().await.storage()?;
        Ok(sequence)
    }

    /// Read mail facts, attachments, and raw bytes from one SQL snapshot.
    async fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let message = reads::get_message(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(message)
    }

    /// Read normalized facts and attachments without fetching raw bytes.
    async fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let metadata = reads::get_message_metadata(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(metadata)
    }

    /// Read message facts and bytes from one snapshot when this user has not deleted it.
    async fn get_user_message(
        &self,
        user: UserIdentifier,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let message = reads::get_user_message(&mut transaction, user, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(message)
    }

    /// Read message facts and attachments from one snapshot unless tombstoned for this user.
    async fn get_user_message_metadata(
        &self,
        user: UserIdentifier,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let metadata = reads::get_user_message_metadata(&mut transaction, user, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(metadata)
    }

    /// List bounded summaries newest first with canonical filters and a keyset cursor.
    async fn list_messages(
        &self,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let summaries = summaries::list_messages(&mut transaction, query).await?;
        transaction.commit().await.storage()?;
        Ok(summaries)
    }

    /// List authorized summaries while excluding this user's tombstoned messages.
    async fn list_user_messages(
        &self,
        user: UserIdentifier,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let summaries = summaries::list_user_messages(&mut transaction, user, query).await?;
        transaction.commit().await.storage()?;
        Ok(summaries)
    }

    /// Delete mail and dependent facts atomically with the search operation.
    async fn delete_message(&self, identifier: MessageIdentifier) -> Result<bool, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let deleted = writes::delete_message(&mut transaction, identifier).await?;
        transaction.commit().await.storage()?;
        Ok(deleted)
    }

    /// Delete only the requested current revision, returning false for stale or missing mail.
    async fn delete_message_at_revision(
        &self,
        identifier: MessageIdentifier,
        revision: u64,
    ) -> Result<bool, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let deleted =
            writes::delete_message_at_revision(&mut transaction, identifier, revision).await?;
        transaction.commit().await.storage()?;
        Ok(deleted)
    }

    /// Atomically tombstone matching messages for this user and return newly deleted count.
    async fn delete_user_messages(
        &self,
        user: UserIdentifier,
        identifiers: Option<&[MessageIdentifier]>,
        filter: &FilterExpression,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let deleted =
            tombstones::delete_user_messages(&mut transaction, user, identifiers, filter).await?;
        crate::imap::organization::collect_unreferenced_messages(&mut transaction).await?;
        transaction.commit().await.storage()?;
        Ok(deleted)
    }

    /// Read the insertion high-water mark, including previously deleted mail.
    async fn max_message_sequence(&self) -> Result<MessageSequence, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let sequence = indexing::max_message_sequence(&mut transaction).await?;
        transaction.commit().await.storage()?;
        Ok(sequence)
    }

    /// Load summaries in request order, omitting missing identifiers.
    async fn hydrate_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let summaries = summaries::hydrate_messages(&mut transaction, identifiers).await?;
        transaction.commit().await.storage()?;
        Ok(summaries)
    }

    /// Load summaries only for current revisions, omitting stale or deleted hits.
    async fn hydrate_current_messages(
        &self,
        requested: &[(MessageIdentifier, u64)],
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let summaries = summaries::hydrate_current_messages(&mut transaction, requested).await?;
        transaction.commit().await.storage()?;
        Ok(summaries)
    }

    /// Hydrate authorized current revisions, omitting stale, user-deleted, and trashed messages.
    async fn hydrate_current_user_messages(
        &self,
        user: UserIdentifier,
        requested: &[(MessageIdentifier, u64)],
        filter: &FilterExpression,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let summaries =
            summaries::hydrate_current_user_messages(&mut transaction, user, requested, filter)
                .await?;
        transaction.commit().await.storage()?;
        Ok(summaries)
    }

    /// Read a bounded ascending sequence range of current index facts and revisions.
    async fn index_messages(
        &self,
        after: MessageSequence,
        through: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<IndexedMessage>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let messages = indexing::index_messages(&mut transaction, after, through, limit).await?;
        transaction.commit().await.storage()?;
        Ok(messages)
    }

    /// Read a bounded set of index facts and revisions in identifier request order.
    async fn indexed_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<IndexedMessage>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let messages = indexing::indexed_messages(&mut transaction, identifiers).await?;
        transaction.commit().await.storage()?;
        Ok(messages)
    }
}
