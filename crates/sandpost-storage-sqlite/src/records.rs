//! Validated decoding of stored rows into backend-neutral values.
use crate::StorageError;
use crate::mail_parts::load_recipients;
use rusqlite::Connection;
use sandpost_core::MessageSequence;
use sandpost_storage::MessageSummary;

/// Selected scalar summary columns in their stable query order.
pub(crate) type StoredSummary = (i64, String, String, String, i64, i64, i64, String);

/// Read the selected summary columns in their stable query order.
pub(crate) fn summary_row(database_row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSummary> {
    Ok((
        database_row.get(0)?,
        database_row.get(1)?,
        database_row.get(2)?,
        database_row.get(3)?,
        database_row.get(4)?,
        database_row.get(5)?,
        database_row.get(6)?,
        database_row.get(7)?,
    ))
}

/// Decode scalar summaries and read their mailbox relations together without fetching bodies.
pub(crate) fn decode_summaries(
    connection: &Connection,
    rows: Vec<StoredSummary>,
) -> Result<Vec<MessageSummary>, StorageError> {
    let sequences: Vec<_> = rows.iter().map(|row| row.0).collect();
    let mut recipients = load_recipients(connection, &sequences)?;
    rows.into_iter()
        .map(
            |(
                sequence,
                identifier,
                subject,
                preview,
                received_at,
                size,
                attachment_count,
                endpoint_identifier,
            )| {
                let mailboxes = recipients.remove(&sequence).unwrap_or_default();
                Ok(MessageSummary {
                    sequence: MessageSequence(
                        u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
                    ),
                    identifier: parse_identifier(&identifier)?,
                    endpoint_identifier: parse_identifier(&endpoint_identifier)?,
                    subject,
                    from: mailboxes.from,
                    to: mailboxes.to,
                    envelope_to: mailboxes.envelope_to,
                    preview,
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

/// Build a rusqlite decode failure for an invalid stored column value.
pub(crate) fn invalid_column(index: usize, name: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(index, name.to_owned(), rusqlite::types::Type::Text)
}
