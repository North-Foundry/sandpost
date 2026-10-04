//! Atomic message ingestion and public message retrieval.
use crate::{
    MessageSummary, Storage, StorageError,
    mail_parts::{load_attachments, load_headers, load_recipients, store_parts},
    records::{StoredSummary, decode_summaries, summary_row},
};
use rusqlite::{OptionalExtension, params};
use sandpost_core::{
    Attachment, Message, MessageFacts, MessageIdentifier, MessageSequence, ScopeIdentifier,
};

pub(crate) const MAXIMUM_PAGE_SIZE: usize = 100;

impl Storage {
    /// Insert a normalized message and materialize its current scope matches atomically.
    pub fn insert_message(
        &self,
        message: &Message,
        matches: &[(ScopeIdentifier, u64)],
    ) -> Result<MessageSequence, StorageError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let inserted = transaction.execute(
            "INSERT INTO mail(identifier, subject, text_body, markup_body, message_identifier, raw_message, received_at, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(identifier) DO NOTHING",
            params![
                message.identifier.to_string(),
                message.facts.subject,
                message.facts.text,
                message.facts.markup_body,
                message.facts.message_identifier,
                message.raw_message,
                message.facts.received_at,
                i64::try_from(message.facts.size).map_err(|_| StorageError::IntegerRange)?,
            ],
        )?;
        if inserted == 0 {
            return Err(StorageError::DuplicateMessageIdentifier(message.identifier));
        }
        let stored_sequence = transaction.last_insert_rowid();
        let sequence = MessageSequence(
            u64::try_from(stored_sequence).map_err(|_| StorageError::IntegerRange)?,
        );
        store_parts(&transaction, stored_sequence, message)?;
        for (scope_identifier, policy_version) in matches {
            let current: Option<i64> = transaction
                .query_row(
                    "SELECT policy_version FROM scopes WHERE identifier = ?1",
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
                "INSERT INTO mail_scope(scope_identifier, mail_sequence, policy_version) VALUES (?1, ?2, ?3) ON CONFLICT(scope_identifier, mail_sequence) DO UPDATE SET policy_version = excluded.policy_version",
                params![
                    scope_identifier.to_string(),
                    stored_sequence,
                    i64::try_from(*policy_version).map_err(|_| StorageError::IntegerRange)?,
                ],
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
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let limit = limit.min(MAXIMUM_PAGE_SIZE);
        let before = before
            .map(|sequence| i64::try_from(sequence.0).map_err(|_| StorageError::IntegerRange))
            .transpose()?;
        let rows: Vec<StoredSummary> = {
            let mut statement = transaction.prepare_cached(
                "SELECT mail.sequence, mail.identifier, mail.subject, mail.received_at, mail.size, (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence) FROM mail WHERE (?1 IS NULL OR mail.sequence < ?1) ORDER BY mail.sequence DESC LIMIT ?2",
            )?;
            statement
                .query_map(
                    params![
                        before,
                        i64::try_from(limit).map_err(|_| StorageError::IntegerRange)?
                    ],
                    summary_row,
                )?
                .collect::<Result<_, _>>()?
        };
        let summaries = decode_summaries(&transaction, rows)?;
        transaction.commit()?;
        Ok(summaries)
    }

    /// Load a message, including its original raw message bytes, by identifier.
    pub fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let stored: Option<(StoredMail, Vec<u8>)> = transaction
            .query_row(
                "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size, raw_message FROM mail WHERE identifier = ?1",
                [identifier.to_string()],
                |row| Ok((mail_row(row)?, row.get(7)?)),
            )
            .optional()?;
        let result = stored
            .map(|(mail, raw_message)| -> Result<_, StorageError> {
                let (facts, attachments) = load_message_parts(&transaction, mail)?;
                Ok(Message {
                    identifier,
                    facts,
                    raw_message,
                    attachments,
                })
            })
            .transpose()?;
        transaction.commit()?;
        Ok(result)
    }

    /// Read parsed message data and attachment metadata without fetching raw message bytes.
    pub fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let stored = transaction
            .query_row(
                "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size FROM mail WHERE identifier = ?1",
                [identifier.to_string()],
                mail_row,
            )
            .optional()?;
        let result = stored
            .map(|mail| load_message_parts(&transaction, mail))
            .transpose()?;
        transaction.commit()?;
        Ok(result)
    }
}

/// Scalar mail metadata shared by full-message and metadata-only reads.
struct StoredMail {
    sequence: i64,
    subject: String,
    text_body: String,
    markup_body: String,
    message_identifier: Option<String>,
    received_at: i64,
    size: i64,
}

/// Decode the scalar mail columns in their shared selection order.
fn mail_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMail> {
    Ok(StoredMail {
        sequence: row.get(0)?,
        subject: row.get(1)?,
        text_body: row.get(2)?,
        markup_body: row.get(3)?,
        message_identifier: row.get(4)?,
        received_at: row.get(5)?,
        size: row.get(6)?,
    })
}

/// Rebuild normalized facts and ordered attachments from one consistent read snapshot.
fn load_message_parts(
    connection: &rusqlite::Connection,
    mail: StoredMail,
) -> Result<(MessageFacts, Vec<Attachment>), StorageError> {
    let recipients = load_recipients(connection, &[mail.sequence])?
        .remove(&mail.sequence)
        .unwrap_or_default();
    let headers = load_headers(connection, mail.sequence)?;
    let attachments = load_attachments(connection, mail.sequence)?;
    let attachment_count =
        u64::try_from(attachments.len()).map_err(|_| StorageError::IntegerRange)?;
    Ok((
        MessageFacts {
            envelope_from: recipients.envelope_from,
            envelope_to: recipients.envelope_to,
            from: recipients.from,
            to: recipients.to,
            carbon_copy: recipients.carbon_copy,
            subject: mail.subject,
            text: mail.text_body,
            markup_body: mail.markup_body,
            message_identifier: mail.message_identifier,
            received_at: mail.received_at,
            size: u64::try_from(mail.size).map_err(|_| StorageError::IntegerRange)?,
            attachment_count,
            headers,
        },
        attachments,
    ))
}
