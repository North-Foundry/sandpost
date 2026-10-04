//! Public message summaries and validated database row decoding.
use crate::{StorageError, mail_parts::load_recipients};
use rusqlite::Connection;
use sandpost_core::{MessageIdentifier, MessageSequence};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    #[serde(rename = "seq")]
    pub sequence: MessageSequence,
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    pub subject: String,
    pub from: Vec<sandpost_core::Mailbox>,
    pub to: Vec<sandpost_core::Mailbox>,
    pub received_at: i64,
    pub size: u64,
    pub attachment_count: u64,
}

pub(crate) type StoredSummary = (i64, String, String, i64, i64, i64);

/// Read the selected summary columns in their stable query order.
pub(crate) fn summary_row(database_row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSummary> {
    Ok((
        database_row.get(0)?,
        database_row.get(1)?,
        database_row.get(2)?,
        database_row.get(3)?,
        database_row.get(4)?,
        database_row.get(5)?,
    ))
}

/// Decode scalar summaries and read their mailbox relations together without fetching mail bodies.
pub(crate) fn decode_summaries(
    connection: &Connection,
    rows: Vec<StoredSummary>,
) -> Result<Vec<MessageSummary>, StorageError> {
    let sequences: Vec<_> = rows.iter().map(|row| row.0).collect();
    let mut recipients = load_recipients(connection, &sequences)?;
    rows.into_iter()
        .map(
            |(sequence, identifier, subject, received_at, size, attachment_count)| {
                let mailboxes = recipients.remove(&sequence).unwrap_or_default();
                Ok(MessageSummary {
                    sequence: MessageSequence(
                        u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
                    ),
                    identifier: parse_identifier(&identifier)?,
                    subject,
                    from: mailboxes.from,
                    to: mailboxes.to,
                    received_at,
                    size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
                    attachment_count: u64::try_from(attachment_count)
                        .map_err(|_| StorageError::IntegerRange)?,
                })
            },
        )
        .collect()
}

/// Parse a stored identifier and preserve the invalid source value in the error.
pub(crate) fn parse_identifier<IdentifierType: std::str::FromStr>(
    value: &str,
) -> Result<IdentifierType, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidData(value.to_owned()))
}
