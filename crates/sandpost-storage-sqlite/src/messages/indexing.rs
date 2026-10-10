//! Bounded normalized projections for the disposable search index.
use super::parts::load_recipients;
use crate::error::StorageResult;
use crate::records::parse_identifier;
use rusqlite::{Connection, params, params_from_iter};
use sandpost_core::{MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_storage::{IndexedMessage, StorageError};

const MAXIMUM_INDEX_BATCH_SIZE: usize = 64;

/// Scalar index input selected together with its durable revision.
struct StoredIndexedMessage {
    sequence: i64,
    identifier: String,
    subject: String,
    text_body: String,
    markup_body: String,
    message_identifier: Option<String>,
    received_at: i64,
    size: i64,
    revision: i64,
}

/// Decode scalar index columns in their shared selection order.
fn indexed_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredIndexedMessage> {
    Ok(StoredIndexedMessage {
        sequence: row.get(0)?,
        identifier: row.get(1)?,
        subject: row.get(2)?,
        text_body: row.get(3)?,
        markup_body: row.get(4)?,
        message_identifier: row.get(5)?,
        received_at: row.get(6)?,
        size: row.get(7)?,
        revision: row.get(8)?,
    })
}

/// Return the highest stored mail sequence, or zero when the database is empty.
pub(super) fn max_message_sequence_blocking(
    connection: &Connection,
) -> Result<MessageSequence, StorageError> {
    let value: i64 = connection
        .query_row(
            "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='mail'),0)",
            [],
            |row| row.get(0),
        )
        .storage()?;
    Ok(MessageSequence(
        u64::try_from(value).map_err(|_| StorageError::IntegerRange)?,
    ))
}

/// Read normalized index records and revisions after an exclusive sequence, bounded by limit.
pub(super) fn index_messages_blocking(
    connection: &mut Connection,
    after: MessageSequence,
    through: Option<MessageSequence>,
    limit: usize,
) -> Result<Vec<IndexedMessage>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let after = i64::try_from(after.0).map_err(|_| StorageError::IntegerRange)?;
    let through = through
        .map(|value| i64::try_from(value.0).map_err(|_| StorageError::IntegerRange))
        .transpose()?;
    let limit = i64::try_from(limit.min(MAXIMUM_INDEX_BATCH_SIZE))
        .map_err(|_| StorageError::IntegerRange)?;
    let mut statement = transaction
        .prepare("SELECT sequence,identifier,subject,text_body,markup_body,message_identifier,received_at,size,search_revision FROM mail WHERE sequence>?1 AND (?2 IS NULL OR sequence<=?2) ORDER BY sequence LIMIT ?3")
        .storage()?;
    let rows = statement
        .query_map(params![after, through, limit], indexed_message_row)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    drop(statement);
    let indexed = load_indexed_rows(&transaction, rows)?;
    transaction.commit().storage()?;
    Ok(indexed)
}

/// Load up to the backend batch maximum of normalized index records by identifier.
pub(super) fn indexed_messages_blocking(
    connection: &mut Connection,
    identifiers: &[MessageIdentifier],
) -> Result<Vec<IndexedMessage>, StorageError> {
    if identifiers.is_empty() {
        return Ok(Vec::new());
    }
    let mut seen = std::collections::HashSet::with_capacity(identifiers.len());
    let identifiers: Vec<_> = identifiers
        .iter()
        .copied()
        .filter(|identifier| seen.insert(*identifier))
        .collect();
    if identifiers.len() > MAXIMUM_INDEX_BATCH_SIZE {
        return Err(StorageError::BatchLimitExceeded(MAXIMUM_INDEX_BATCH_SIZE));
    }
    let transaction = connection.transaction().storage()?;
    let values: Vec<String> = identifiers.iter().map(ToString::to_string).collect();
    let placeholders = (1..=values.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT sequence,identifier,subject,text_body,markup_body,message_identifier,received_at,size,search_revision FROM mail WHERE identifier IN ({placeholders})"
    );
    let mut statement = transaction.prepare(&sql).storage()?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), indexed_message_row)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    drop(statement);
    let indexed = load_indexed_rows(&transaction, rows)?;
    transaction.commit().storage()?;
    let by_identifier: std::collections::HashMap<_, _> = indexed
        .into_iter()
        .map(|record| (record.identifier, record))
        .collect();
    Ok(identifiers
        .iter()
        .filter_map(|identifier| by_identifier.get(identifier).cloned())
        .collect())
}

/// Batch-load recipient roles, headers, and attachment counts for selected mail rows.
fn load_indexed_rows(
    connection: &Connection,
    rows: Vec<StoredIndexedMessage>,
) -> Result<Vec<IndexedMessage>, StorageError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let sequences: Vec<i64> = rows.iter().map(|row| row.sequence).collect();
    let recipients = load_recipients(connection, &sequences)?;
    let placeholders = (1..=sequences.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut headers_by_sequence =
        std::collections::HashMap::<i64, std::collections::BTreeMap<String, Vec<String>>>::new();
    let query = format!(
        "SELECT mail_sequence,name,value FROM mail_headers WHERE mail_sequence IN ({placeholders}) ORDER BY mail_sequence,name,ordinal"
    );
    {
        let mut statement = connection.prepare(&query).storage()?;
        let mut result = statement
            .query(params_from_iter(sequences.iter()))
            .storage()?;
        while let Some(row) = result.next().storage()? {
            headers_by_sequence
                .entry(row.get(0).storage()?)
                .or_default()
                .entry(row.get(1).storage()?)
                .or_default()
                .push(row.get(2).storage()?);
        }
    }
    let mut attachment_counts_by_sequence = std::collections::HashMap::<i64, i64>::new();
    let query = format!(
        "SELECT mail_sequence,COUNT(*) FROM mail_attachments WHERE mail_sequence IN ({placeholders}) GROUP BY mail_sequence"
    );
    {
        let mut statement = connection.prepare(&query).storage()?;
        let mut result = statement
            .query(params_from_iter(sequences.iter()))
            .storage()?;
        while let Some(row) = result.next().storage()? {
            let sequence: i64 = row.get(0).storage()?;
            attachment_counts_by_sequence.insert(sequence, row.get(1).storage()?);
        }
    }
    rows.into_iter()
        .map(
            |StoredIndexedMessage {
                 sequence,
                 identifier,
                 subject,
                 text_body,
                 markup_body,
                 message_identifier,
                 received_at,
                 size,
                 revision,
             }| {
                let mailbox = recipients.get(&sequence);
                let facts = MessageFacts {
                    envelope_from: mailbox.and_then(|value| value.envelope_from.clone()),
                    envelope_to: mailbox
                        .map(|value| value.envelope_to.clone())
                        .unwrap_or_default(),
                    from: mailbox.map(|value| value.from.clone()).unwrap_or_default(),
                    to: mailbox.map(|value| value.to.clone()).unwrap_or_default(),
                    carbon_copy: mailbox
                        .map(|value| value.carbon_copy.clone())
                        .unwrap_or_default(),
                    subject,
                    text: text_body,
                    markup_body,
                    message_identifier,
                    received_at,
                    size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
                    attachment_count: u64::try_from(
                        attachment_counts_by_sequence
                            .remove(&sequence)
                            .unwrap_or_default(),
                    )
                    .map_err(|_| StorageError::IntegerRange)?,
                    headers: headers_by_sequence.remove(&sequence).unwrap_or_default(),
                };
                Ok(IndexedMessage {
                    revision: u64::try_from(revision).map_err(|_| StorageError::IntegerRange)?,
                    identifier: parse_identifier(&identifier)?,
                    sequence: MessageSequence(
                        u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
                    ),
                    facts,
                })
            },
        )
        .collect()
}
