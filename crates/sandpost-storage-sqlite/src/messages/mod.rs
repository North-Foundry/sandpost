//! SQLite adapter for atomic mail writes, snapshot reads, summaries, and index projections.
mod indexing;
mod parts;
mod reads;
mod summaries;
mod writes;

use crate::SqliteStorage;
use async_trait::async_trait;
use indexing::{index_messages_blocking, indexed_messages_blocking, max_message_sequence_blocking};
use reads::{get_message_blocking, get_message_metadata_blocking};
use sandpost_core::{Attachment, Message, MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_storage::{
    IndexedMessage, MessageListQuery, MessageStorage, MessageSummary, StorageError,
};
use summaries::{
    hydrate_current_messages_blocking, hydrate_messages_blocking, list_messages_blocking,
};
use writes::{
    delete_message_at_revision_blocking, delete_message_blocking, insert_message_blocking,
};

#[async_trait]
impl MessageStorage for SqliteStorage {
    /// Persist mail and child facts atomically with one search revision.
    async fn insert_message(&self, message: &Message) -> Result<MessageSequence, StorageError> {
        let message = message.clone();
        self.run(move |connection| insert_message_blocking(connection, &message))
            .await
    }

    /// Read mail facts, attachments, and raw bytes from one SQL snapshot.
    async fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError> {
        self.run(move |connection| get_message_blocking(connection, identifier))
            .await
    }

    /// Read normalized facts and attachments without fetching raw bytes.
    async fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError> {
        self.run(move |connection| get_message_metadata_blocking(connection, identifier))
            .await
    }

    /// List bounded summaries newest first with canonical filters and a keyset cursor.
    async fn list_messages(
        &self,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        self.run(move |connection| list_messages_blocking(connection, query))
            .await
    }

    /// Delete mail and dependent facts atomically with the search operation.
    async fn delete_message(&self, identifier: MessageIdentifier) -> Result<bool, StorageError> {
        self.run(move |connection| delete_message_blocking(connection, identifier))
            .await
    }

    /// Delete only the requested current revision, returning false for stale or missing mail.
    async fn delete_message_at_revision(
        &self,
        identifier: MessageIdentifier,
        revision: u64,
    ) -> Result<bool, StorageError> {
        self.run(move |connection| {
            delete_message_at_revision_blocking(connection, identifier, revision)
        })
        .await
    }

    /// Read the insertion high-water mark, including previously deleted mail.
    async fn max_message_sequence(&self) -> Result<MessageSequence, StorageError> {
        self.run(|connection| max_message_sequence_blocking(connection))
            .await
    }

    /// Load summaries in request order, omitting missing identifiers.
    async fn hydrate_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let identifiers = identifiers.to_vec();
        self.run(move |connection| hydrate_messages_blocking(connection, &identifiers))
            .await
    }

    /// Load summaries only for current revisions, omitting stale or deleted hits.
    async fn hydrate_current_messages(
        &self,
        requested: &[(MessageIdentifier, u64)],
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let requested = requested.to_vec();
        self.run(move |connection| hydrate_current_messages_blocking(connection, &requested))
            .await
    }

    /// Read a bounded ascending sequence range of current index facts and revisions.
    async fn index_messages(
        &self,
        after: MessageSequence,
        through: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<IndexedMessage>, StorageError> {
        self.run(move |connection| index_messages_blocking(connection, after, through, limit))
            .await
    }

    /// Read a bounded set of index facts and revisions in identifier request order.
    async fn indexed_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<IndexedMessage>, StorageError> {
        let identifiers = identifiers.to_vec();
        self.run(move |connection| indexed_messages_blocking(connection, &identifiers))
            .await
    }
}
