//! Atomic message insertion and revision-aware deletion.
use super::parts::store_parts;
use crate::error::StorageResult;
use rusqlite::{Connection, TransactionBehavior, params};
use sandpost_core::{Message, MessageIdentifier, MessageSequence};
use sandpost_storage::StorageError;

/// Insert all child facts before the parent so its trigger enqueues exactly one complete revision.
/// An immediate transaction reserves the next AUTOINCREMENT sequence across connections; foreign
/// keys are deferred until commit, never disabled. Later child mutations retain their triggers.
pub(super) fn insert_message_blocking(
    connection: &mut Connection,
    message: &Message,
) -> Result<MessageSequence, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    transaction
        .pragma_update(None, "defer_foreign_keys", "ON")
        .storage()?;
    let previous_sequence: i64 = transaction
        .query_row(
            "SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'mail'), 0), COALESCE((SELECT MAX(sequence) FROM mail), 0))",
            [],
            |row| row.get(0),
        )
        .storage()?;
    let stored_sequence = previous_sequence
        .checked_add(1)
        .filter(|sequence| *sequence > 0)
        .ok_or(StorageError::IntegerRange)?;
    store_parts(&transaction, stored_sequence, message)?;
    let inserted = transaction
        .execute(
            "INSERT INTO mail(identifier, subject, text_body, markup_body, message_identifier, raw_message, received_at, size, sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(identifier) DO NOTHING",
            params![
                message.identifier.to_string(),
                message.facts.subject,
                message.facts.text,
                message.facts.markup_body,
                message.facts.message_identifier,
                message.raw_message,
                message.facts.received_at,
                i64::try_from(message.facts.size).map_err(|_| StorageError::IntegerRange)?,
                stored_sequence,
            ],
        )
        .storage()?;
    if inserted == 0 {
        return Err(StorageError::DuplicateMessageIdentifier(message.identifier));
    }
    let sequence =
        MessageSequence(u64::try_from(stored_sequence).map_err(|_| StorageError::IntegerRange)?);
    transaction.commit().storage()?;
    Ok(sequence)
}

/// Delete a message and its cascading child facts in one transaction.
pub(super) fn delete_message_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
) -> Result<bool, StorageError> {
    let transaction = connection.transaction().storage()?;
    let deleted = transaction
        .execute(
            "DELETE FROM mail WHERE identifier = ?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0;
    transaction.commit().storage()?;
    Ok(deleted)
}

/// Delete only the exact authorized revision, returning false for stale or missing mail.
pub(super) fn delete_message_at_revision_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
    revision: u64,
) -> Result<bool, StorageError> {
    let revision = i64::try_from(revision).map_err(|_| StorageError::IntegerRange)?;
    let transaction = connection.transaction().storage()?;
    let deleted = transaction
        .execute(
            "DELETE FROM mail WHERE identifier = ?1 AND search_revision = ?2",
            params![identifier.to_string(), revision],
        )
        .storage()?
        > 0;
    transaction.commit().storage()?;
    Ok(deleted)
}
