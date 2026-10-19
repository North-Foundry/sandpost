//! Link check result storage capability.
use crate::{MessageIdentifier, MessageLinkCheck, StorageError};
use async_trait::async_trait;

/// The most recent completed link check of each message, shared by every authorized reader.
///
/// Results belong to the physical message and disappear with it. Reads are not user-scoped:
/// callers must authorize the message before exposing its results.
#[async_trait]
pub trait LinkCheckStorage: Send + Sync {
    /**
     * Load the stored check of a message with its links in message order, if one completed.
     */
    async fn get_message_link_check(
        &self,
        message: MessageIdentifier,
    ) -> Result<Option<MessageLinkCheck>, StorageError>;

    /**
     * Atomically replace the stored check of a message with a newer complete check.
     *
     * Readers observe either the previous check or the replacement, never a mixture. Each URL
     * appears at most once; a duplicate URL is a ConstraintViolation and leaves the previous
     * check unchanged. A message that no longer exists fails with NotFound. A replacement whose
     * `checked_at` is older than the stored check is ignored and returns false.
     */
    async fn replace_message_link_check(
        &self,
        check: &MessageLinkCheck,
    ) -> Result<bool, StorageError>;
}
