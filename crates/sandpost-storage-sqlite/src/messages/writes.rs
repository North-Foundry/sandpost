//! Atomic message insertion and revision-aware deletion.
use super::parts::store_parts;
use crate::error::StorageResult;
use sandpost_core::{Message, MessageIdentifier, MessageSequence, PersonaExecutionIdentifier};
use sandpost_storage::{PERSONA_PENDING_EXECUTION_LIMIT, StorageError};
use sqlx::{QueryBuilder, Sqlite, SqliteConnection};

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
    schedule_persona_executions(connection, message).await?;
    Ok(MessageSequence(
        u64::try_from(stored_sequence).map_err(|_| StorageError::IntegerRange)?,
    ))
}

/**
 * Schedule one pending execution per enabled Persona addressed by the message's envelope.
 *
 * Persona addresses are stored ASCII-lowercased, so envelope recipients are compared the same
 * way. A Persona already holding the pending limit receives nothing, and the unique
 * Persona/message pair keeps a recipient listed twice from scheduling twice. Runs inside the
 * caller's message transaction, so scheduling commits or rolls back with the message.
 */
async fn schedule_persona_executions(
    connection: &mut SqliteConnection,
    message: &Message,
) -> Result<(), StorageError> {
    let mut addresses: Vec<String> = message
        .facts
        .envelope_to
        .iter()
        .map(|recipient| recipient.address.to_ascii_lowercase())
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        return Ok(());
    }
    let pending_limit =
        i64::try_from(PERSONA_PENDING_EXECUTION_LIMIT).map_err(|_| StorageError::IntegerRange)?;
    let mut personas: Vec<String> = Vec::new();
    for chunk in addresses.chunks(128) {
        let mut statement = QueryBuilder::<Sqlite>::new(
            "SELECT persona.identifier FROM personas AS persona WHERE persona.enabled = 1 AND (SELECT count(*) FROM persona_executions AS execution WHERE execution.persona_identifier = persona.identifier AND execution.status = 'pending') < ",
        );
        statement.push_bind(pending_limit);
        statement.push(" AND persona.address IN (");
        let mut separated = statement.separated(", ");
        for address in chunk {
            separated.push_bind(address);
        }
        statement.push(")");
        personas.extend(
            statement
                .build_query_scalar::<String>()
                .fetch_all(&mut *connection)
                .await
                .storage()?,
        );
    }
    for persona in personas {
        sqlx::query(
            "INSERT INTO persona_executions(identifier, persona_identifier, message_identifier, status, scheduled_at) VALUES (?, ?, ?, 'pending', ?) ON CONFLICT(persona_identifier, message_identifier) DO NOTHING",
        )
        .bind(PersonaExecutionIdentifier::new().to_string())
        .bind(persona)
        .bind(message.identifier.to_string())
        .bind(message.facts.received_at)
        .execute(&mut *connection)
        .await
        .storage()?;
    }
    Ok(())
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
