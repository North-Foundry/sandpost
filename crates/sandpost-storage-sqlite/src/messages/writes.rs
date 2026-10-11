//! Atomic message insertion and revision-aware deletion.
use super::parts::store_parts;
use crate::error::StorageResult;
use sandpost_core::{Message, MessageIdentifier, MessageSequence};
use sandpost_storage::StorageError;
use sqlx::SqliteConnection;

/// Insert child facts before the parent so its trigger enqueues one complete revision.
/// The caller owns an immediate transaction; deferred foreign keys are checked at commit.
pub(super) async fn insert_message(
    connection: &mut SqliteConnection,
    message: &Message,
) -> Result<MessageSequence, StorageError> {
    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut *connection)
        .await
        .storage()?;
    let previous_sequence: i64 = sqlx::query_scalar(
        "SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'mail'), 0), COALESCE((SELECT MAX(sequence) FROM mail), 0))",
    )
    .fetch_one(&mut *connection)
    .await
    .storage()?;
    let stored_sequence = previous_sequence
        .checked_add(1)
        .filter(|sequence| *sequence > 0)
        .ok_or(StorageError::IntegerRange)?;
    store_parts(connection, stored_sequence, message).await?;
    let inserted = sqlx::query(
        "INSERT INTO mail(identifier, subject, text_body, markup_body, message_identifier, raw_message, received_at, size, sequence) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(identifier) DO NOTHING",
    )
    .bind(message.identifier.to_string())
    .bind(&message.facts.subject)
    .bind(&message.facts.text)
    .bind(&message.facts.markup_body)
    .bind(&message.facts.message_identifier)
    .bind(&message.raw_message)
    .bind(message.facts.received_at)
    .bind(i64::try_from(message.facts.size).map_err(|_| StorageError::IntegerRange)?)
    .bind(stored_sequence)
    .execute(&mut *connection)
    .await
    .storage()?;
    if inserted.rows_affected() == 0 {
        return Err(StorageError::DuplicateMessageIdentifier(message.identifier));
    }
    Ok(MessageSequence(
        u64::try_from(stored_sequence).map_err(|_| StorageError::IntegerRange)?,
    ))
}

/// Delete a message and its cascading child facts inside the caller's transaction.
pub(super) async fn delete_message(
    connection: &mut SqliteConnection,
    identifier: MessageIdentifier,
) -> Result<bool, StorageError> {
    let result = sqlx::query("DELETE FROM mail WHERE identifier = ?")
        .bind(identifier.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
    Ok(result.rows_affected() > 0)
}

/// Delete only the exact authorized revision inside the caller's transaction.
pub(super) async fn delete_message_at_revision(
    connection: &mut SqliteConnection,
    identifier: MessageIdentifier,
    revision: u64,
) -> Result<bool, StorageError> {
    let revision = i64::try_from(revision).map_err(|_| StorageError::IntegerRange)?;
    let result = sqlx::query("DELETE FROM mail WHERE identifier = ? AND search_revision = ?")
        .bind(identifier.to_string())
        .bind(revision)
        .execute(&mut *connection)
        .await
        .storage()?;
    Ok(result.rows_affected() > 0)
}
