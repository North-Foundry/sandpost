//! Public message summaries and validated database row decoding.
use crate::StorageError;
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

pub(crate) type StoredSummary = (i64, String, String, String, String, i64, i64, i64);

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

/// Convert a stored summary row to domain values, validating numeric and serialized fields.
pub(crate) fn decode_summary(
    (sequence, identifier, subject, from, to, received_at, size, attachment_count): StoredSummary,
) -> Result<MessageSummary, StorageError> {
    Ok(MessageSummary {
        sequence: MessageSequence(u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?),
        identifier: parse_identifier(&identifier)?,
        subject,
        from: serde_json::from_str(&from)?,
        to: serde_json::from_str(&to)?,
        received_at,
        size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
        attachment_count: u64::try_from(attachment_count)
            .map_err(|_| StorageError::IntegerRange)?,
    })
}

/// Parse a stored identifier and preserve the invalid source value in the error.
pub(crate) fn parse_identifier<IdentifierType: std::str::FromStr>(
    value: &str,
) -> Result<IdentifierType, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidData(value.to_owned()))
}
