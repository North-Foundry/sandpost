//! Messages storage capability.
use crate::{
    Attachment, FilterExpression, IndexedMessage, Message, MessageFacts, MessageIdentifier,
    MessageListQuery, MessageSequence, MessageSummary, StorageError, UserIdentifier,
};
use async_trait::async_trait;

/// Atomic message ingestion, retrieval, ordering, and deletion.
#[async_trait]
pub trait MessageStorage: Send + Sync {
    /// Insert a normalized message and all relational facts atomically.
    ///
    /// The same atomic operation schedules pending Persona executions for the message's SMTP
    /// envelope recipients, as described by [`crate::PersonaStorage`].
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

    /// Load a message and its original bytes unless this user has deleted it.
    /// This checks deletion state only; callers must authorize the message separately.
    async fn get_user_message(
        &self,
        user: UserIdentifier,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError>;

    /// Load normalized facts and attachments unless this user has deleted the message.
    /// This checks deletion state only; callers must authorize the message separately.
    async fn get_user_message_metadata(
        &self,
        user: UserIdentifier,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError>;

    /// List message summaries newest first using filter and keyset bounds.
    async fn list_messages(
        &self,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// List summaries matching the query and excluding this user's deleted messages.
    /// The query filter must contain the caller's fully composed authorization filter.
    async fn list_user_messages(
        &self,
        user: UserIdentifier,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Physically delete a message and its cascading child facts for privileged maintenance.
    async fn delete_message(&self, identifier: MessageIdentifier) -> Result<bool, StorageError>;

    /// Physically delete the exact revision for privileged maintenance.
    async fn delete_message_at_revision(
        &self,
        identifier: MessageIdentifier,
        revision: u64,
    ) -> Result<bool, StorageError>;

    /// Return the persistent message-sequence high-water mark, including deleted messages, or
    /// zero when no message has ever been inserted.
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

    /// Hydrate current revisions that satisfy the fully composed authorization filter and deletion
    /// state, omitting messages in the user's Web Trash.
    async fn hydrate_current_user_messages(
        &self,
        user: UserIdentifier,
        requested: &[(MessageIdentifier, u64)],
        filter: &FilterExpression,
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Permanently release matching primary messages, preserving other primary/Independent views.
    /// Content released by every retaining view may be garbage-collected atomically.
    /// The filter must contain the caller's fully composed authorization filter; an explicit
    /// identifier batch is limited to 256 entries, and the return value counts new tombstones.
    async fn delete_user_messages(
        &self,
        user: UserIdentifier,
        identifiers: Option<&[MessageIdentifier]>,
        filter: &FilterExpression,
    ) -> Result<u64, StorageError>;

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
