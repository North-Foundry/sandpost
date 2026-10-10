//! Messages storage capability.
use crate::{
    Attachment, IndexedMessage, Message, MessageFacts, MessageIdentifier, MessageListQuery,
    MessageSequence, MessageSummary, StorageError,
};
use async_trait::async_trait;

/// Atomic message ingestion, retrieval, ordering, and deletion.
#[async_trait]
pub trait MessageStorage: Send + Sync {
    /// Insert a normalized message and all relational facts atomically.
    async fn insert_message(&self, message: &Message) -> Result<MessageSequence, StorageError>;

    /// Load a message and its original bytes from one snapshot.
    async fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError>;

    /// Load normalized facts and attachments without raw bytes.
    async fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError>;

    /// List message summaries newest first using filter and keyset bounds.
    async fn list_messages(
        &self,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Delete a message and its cascading child facts in one transaction.
    async fn delete_message(&self, identifier: MessageIdentifier) -> Result<bool, StorageError>;

    /// Delete only the exact authorized revision, returning false for stale or missing mail.
    async fn delete_message_at_revision(
        &self,
        identifier: MessageIdentifier,
        revision: u64,
    ) -> Result<bool, StorageError>;

    /// Return the highest stored mail sequence, or zero when empty.
    async fn max_message_sequence(&self) -> Result<MessageSequence, StorageError>;

    /// Load lightweight summaries in caller-supplied identifier order, omitting missing IDs.
    async fn hydrate_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Hydrate only messages whose current revision equals the requested revision.
    async fn hydrate_current_messages(
        &self,
        requested: &[(MessageIdentifier, u64)],
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Read normalized index records and revisions after an exclusive sequence, bounded by limit.
    async fn index_messages(
        &self,
        after: MessageSequence,
        through: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<IndexedMessage>, StorageError>;

    /// Load up to the backend batch maximum of normalized index records by identifier.
    async fn indexed_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<IndexedMessage>, StorageError>;
}
