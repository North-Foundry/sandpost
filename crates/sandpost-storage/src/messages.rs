//! Atomic message ingestion and public message retrieval.
use crate::{
    MessageSummary, Storage, StorageError,
    message_indexes::index_message,
    records::{decode_summary, summary_row},
};
use rusqlite::{OptionalExtension, params};
use sandpost_core::{Message, MessageIdentifier, MessageSequence, ScopeIdentifier};

pub(crate) const MAXIMUM_PAGE_SIZE: usize = 100;

impl Storage {
    /// Insert a message once and materialize its current scope matches atomically.
    pub fn insert_message(
        &self,
        message: &Message,
        matches: &[(ScopeIdentifier, u64)],
    ) -> Result<MessageSequence, StorageError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let facts = serde_json::to_string(&message.facts)?;
        let attachments = serde_json::to_string(&message.attachments)?;
        let sender_domain = message
            .facts
            .envelope_from
            .as_ref()
            .or_else(|| message.facts.from.first())
            .map(|mailbox| mailbox.domain.as_str())
            .unwrap_or("");
        let inserted = transaction.execute(
            "INSERT INTO messages(id, facts, raw_mime, attachments, sender_domain, received_at, subject, from_json, to_json, size, attachment_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                message.identifier.to_string(), facts, message.raw_message, attachments, sender_domain,
                message.facts.received_at, message.facts.subject,
                serde_json::to_string(&message.facts.from)?, serde_json::to_string(&message.facts.to)?,
                i64::try_from(message.facts.size).map_err(|_| StorageError::IntegerRange)?,
                i64::try_from(message.facts.attachment_count).map_err(|_| StorageError::IntegerRange)?,
            ],
        );
        let inserted = match inserted {
            Ok(changes) => changes,
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(StorageError::DuplicateMessageIdentifier(message.identifier));
            }
            Err(error) => return Err(error.into()),
        };
        let stored_sequence: i64 = transaction.query_row(
            "SELECT seq FROM messages WHERE id = ?1",
            [message.identifier.to_string()],
            |row| row.get(0),
        )?;
        let sequence = MessageSequence(
            u64::try_from(stored_sequence).map_err(|_| StorageError::IntegerRange)?,
        );
        if inserted > 0 {
            index_message(&transaction, stored_sequence, message)?;
        }
        for (scope_identifier, policy_version) in matches {
            let current: Option<i64> = transaction
                .query_row(
                    "SELECT policy_version FROM scopes WHERE id = ?1",
                    [scope_identifier.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(current) = current else {
                return Err(rusqlite::Error::QueryReturnedNoRows.into());
            };
            if u64::try_from(current).ok() != Some(*policy_version) {
                return Err(StorageError::StalePolicy(*scope_identifier));
            }
            transaction.execute(
                "INSERT INTO message_scope(scope_id, message_seq, policy_version) VALUES (?1, ?2, ?3) ON CONFLICT(scope_id, message_seq) DO UPDATE SET policy_version = excluded.policy_version",
                params![scope_identifier.to_string(), stored_sequence, i64::try_from(*policy_version).map_err(|_| StorageError::IntegerRange)?],
            )?;
        }
        transaction.commit()?;
        Ok(sequence)
    }

    /// List messages newest first, optionally restricting results to sequences before a cursor.
    pub fn list_messages(
        &self,
        before: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let connection = self.connection()?;
        let limit = limit.min(MAXIMUM_PAGE_SIZE);
        let mut statement = connection.prepare_cached(
            "SELECT seq, id, subject, from_json, to_json, received_at, size, attachment_count FROM messages WHERE (?1 IS NULL OR seq < ?1) ORDER BY seq DESC LIMIT ?2",
        )?;
        let before = before
            .map(|sequence| i64::try_from(sequence.0).map_err(|_| StorageError::IntegerRange))
            .transpose()?;
        let rows = statement.query_map(params![before, limit as i64], summary_row)?;
        rows.map(|database_row| {
            database_row
                .map_err(StorageError::from)
                .and_then(decode_summary)
        })
        .collect()
    }

    /// Load a message, including its original raw message bytes, by identifier.
    pub fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError> {
        let connection = self.connection()?;
        let stored: Option<(String, Vec<u8>, String)> = connection
            .query_row(
                "SELECT facts, raw_mime, attachments FROM messages WHERE id = ?1",
                [identifier.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        stored
            .map(|(facts, raw_message, attachments)| {
                Ok(Message {
                    identifier,
                    facts: serde_json::from_str(&facts)?,
                    raw_message,
                    attachments: serde_json::from_str(&attachments)?,
                })
            })
            .transpose()
    }

    /// Read parsed message data and attachment metadata without fetching raw MIME bytes.
    pub fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(sandpost_core::MessageFacts, Vec<sandpost_core::Attachment>)>, StorageError>
    {
        let connection = self.connection()?;
        let stored: Option<(String, String)> = connection
            .query_row(
                "SELECT facts, attachments FROM messages WHERE id = ?1",
                [identifier.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(facts, attachments)| {
                Ok((
                    serde_json::from_str(&facts)?,
                    serde_json::from_str(&attachments)?,
                ))
            })
            .transpose()
    }
}
