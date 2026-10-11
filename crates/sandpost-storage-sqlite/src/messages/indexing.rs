//! Bounded normalized projections for the disposable search index.
use super::parts::load_recipients;
use crate::error::StorageResult;
use crate::records::parse_identifier;
use sandpost_core::{MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_storage::{IndexedMessage, StorageError};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

/// Maximum number of messages in one index projection batch.
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

/// Decode scalar index columns by their explicit result names.
fn indexed_message_row(row: &sqlx::sqlite::SqliteRow) -> Result<StoredIndexedMessage, sqlx::Error> {
    Ok(StoredIndexedMessage {
        sequence: row.try_get("sequence")?,
        identifier: row.try_get("identifier")?,
        subject: row.try_get("subject")?,
        text_body: row.try_get("text_body")?,
        markup_body: row.try_get("markup_body")?,
        message_identifier: row.try_get("message_identifier")?,
        received_at: row.try_get("received_at")?,
        size: row.try_get("size")?,
        revision: row.try_get("search_revision")?,
    })
}

/// Return the highest stored mail sequence, or zero when the database is empty.
pub(super) async fn max_message_sequence(
    connection: &mut SqliteConnection,
) -> Result<MessageSequence, StorageError> {
    let value: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='mail'),0)",
    )
    .fetch_one(&mut *connection)
    .await
    .storage()?;
    Ok(MessageSequence(
        u64::try_from(value).map_err(|_| StorageError::IntegerRange)?,
    ))
}

/// Read normalized index records and revisions after an exclusive sequence, bounded by limit.
pub(super) async fn index_messages(
    connection: &mut SqliteConnection,
    after: MessageSequence,
    through: Option<MessageSequence>,
    limit: usize,
) -> Result<Vec<IndexedMessage>, StorageError> {
    let after = i64::try_from(after.0).map_err(|_| StorageError::IntegerRange)?;
    let through = through
        .map(|value| i64::try_from(value.0).map_err(|_| StorageError::IntegerRange))
        .transpose()?;
    let limit = i64::try_from(limit.min(MAXIMUM_INDEX_BATCH_SIZE))
        .map_err(|_| StorageError::IntegerRange)?;
    let rows = sqlx::query(
        "SELECT sequence,identifier,subject,text_body,markup_body,message_identifier,received_at,size,search_revision FROM mail WHERE sequence>? AND (? IS NULL OR sequence<=?) ORDER BY sequence LIMIT ?",
    )
    .bind(after)
    .bind(through)
    .bind(through)
    .bind(limit)
    .try_map(|row| indexed_message_row(&row))
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    load_indexed_rows(connection, rows).await
}

/// Load index records by identifier while preserving caller order and enforcing the batch bound.
pub(super) async fn indexed_messages(
    connection: &mut SqliteConnection,
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
    let values: Vec<_> = identifiers.iter().map(ToString::to_string).collect();
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT sequence,identifier,subject,text_body,markup_body,message_identifier,received_at,size,search_revision FROM mail WHERE identifier IN (",
    );
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            query.push(",");
        }
        query.push_bind(value);
    }
    query.push(")");
    let rows = query
        .build()
        .try_map(|row| indexed_message_row(&row))
        .fetch_all(&mut *connection)
        .await
        .storage()?;
    let indexed = load_indexed_rows(connection, rows).await?;
    let by_identifier: std::collections::HashMap<_, _> = indexed
        .into_iter()
        .map(|record| (record.identifier, record))
        .collect();
    Ok(identifiers
        .iter()
        .filter_map(|identifier| by_identifier.get(identifier).cloned())
        .collect())
}

/// Batch-load recipients, headers, and attachment counts for the selected mail rows.
async fn load_indexed_rows(
    connection: &mut SqliteConnection,
    rows: Vec<StoredIndexedMessage>,
) -> Result<Vec<IndexedMessage>, StorageError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let sequences: Vec<i64> = rows.iter().map(|row| row.sequence).collect();
    let mut recipients = load_recipients(connection, &sequences).await?;
    let mut headers_by_sequence =
        std::collections::HashMap::<i64, std::collections::BTreeMap<String, Vec<String>>>::new();
    let mut header_query = QueryBuilder::<Sqlite>::new(
        "SELECT mail_sequence,name,value FROM mail_headers WHERE mail_sequence IN (",
    );
    for (index, sequence) in sequences.iter().enumerate() {
        if index != 0 {
            header_query.push(",");
        }
        header_query.push_bind(sequence);
    }
    header_query.push(") ORDER BY mail_sequence,name,ordinal");
    for row in header_query
        .build()
        .fetch_all(&mut *connection)
        .await
        .storage()?
    {
        headers_by_sequence
            .entry(row.try_get("mail_sequence").storage()?)
            .or_default()
            .entry(row.try_get("name").storage()?)
            .or_default()
            .push(row.try_get("value").storage()?);
    }
    let mut counts_by_sequence = std::collections::HashMap::<i64, i64>::new();
    let mut count_query = QueryBuilder::<Sqlite>::new(
        "SELECT mail_sequence,COUNT(*) AS attachment_count FROM mail_attachments WHERE mail_sequence IN (",
    );
    for (index, sequence) in sequences.iter().enumerate() {
        if index != 0 {
            count_query.push(",");
        }
        count_query.push_bind(sequence);
    }
    count_query.push(") GROUP BY mail_sequence");
    for row in count_query
        .build()
        .fetch_all(&mut *connection)
        .await
        .storage()?
    {
        counts_by_sequence.insert(
            row.try_get("mail_sequence").storage()?,
            row.try_get("attachment_count").storage()?,
        );
    }
    rows.into_iter()
        .map(|row| {
            let sequence = row.sequence;
            let mailboxes = recipients.remove(&sequence).unwrap_or_default();
            let attachment_count = counts_by_sequence.remove(&sequence).unwrap_or_default();
            let facts = MessageFacts {
                envelope_from: mailboxes.envelope_from,
                envelope_to: mailboxes.envelope_to,
                from: mailboxes.from,
                to: mailboxes.to,
                carbon_copy: mailboxes.carbon_copy,
                subject: row.subject,
                text: row.text_body,
                markup_body: row.markup_body,
                message_identifier: row.message_identifier,
                received_at: row.received_at,
                size: u64::try_from(row.size).map_err(|_| StorageError::IntegerRange)?,
                attachment_count: u64::try_from(attachment_count)
                    .map_err(|_| StorageError::IntegerRange)?,
                headers: headers_by_sequence.remove(&sequence).unwrap_or_default(),
            };
            Ok(IndexedMessage {
                revision: u64::try_from(row.revision).map_err(|_| StorageError::IntegerRange)?,
                identifier: parse_identifier(&row.identifier)?,
                sequence: MessageSequence(
                    u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
                ),
                facts,
            })
        })
        .collect()
}
