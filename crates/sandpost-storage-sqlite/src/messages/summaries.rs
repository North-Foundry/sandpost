//! Bounded summary listing and hydration with keyset and revision checks.
use super::parts::load_recipients;
use crate::error::StorageResult;
use crate::filter::compile;
use crate::records::parse_identifier;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params_from_iter};
use sandpost_core::{MessageIdentifier, MessageSequence};
use sandpost_storage::{MessageListQuery, MessageSummary, StorageError};

/// Maximum page size for summary listings.
const MAXIMUM_PAGE_SIZE: usize = 100;

const SUMMARY_SELECT: &str = "SELECT mail.sequence, mail.identifier, mail.subject, substr(CASE WHEN mail.text_body != '' THEN mail.text_body ELSE mail.markup_body END, 1, 160), mail.received_at, mail.size, (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence) FROM mail";
/// Like [SUMMARY_SELECT] but with the durable revision appended for exact-revision hydration.
const SUMMARY_SELECT_WITH_REVISION: &str = "SELECT mail.sequence, mail.identifier, mail.subject, substr(CASE WHEN mail.text_body != '' THEN mail.text_body ELSE mail.markup_body END, 1, 160), mail.received_at, mail.size, (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence), mail.search_revision FROM mail";

/// Selected scalar summary columns in their stable query order.
struct StoredSummary {
    sequence: i64,
    identifier: String,
    subject: String,
    preview: String,
    received_at: i64,
    size: i64,
    attachment_count: i64,
}

/// Read the selected summary columns in their stable query order.
fn summary_row(database_row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSummary> {
    Ok(StoredSummary {
        sequence: database_row.get(0)?,
        identifier: database_row.get(1)?,
        subject: database_row.get(2)?,
        preview: database_row.get(3)?,
        received_at: database_row.get(4)?,
        size: database_row.get(5)?,
        attachment_count: database_row.get(6)?,
    })
}

/// Decode scalar summaries and read their mailbox relations together without fetching bodies.
fn decode_summaries(
    connection: &Connection,
    rows: Vec<StoredSummary>,
) -> Result<Vec<MessageSummary>, StorageError> {
    let sequences: Vec<_> = rows.iter().map(|row| row.sequence).collect();
    let mut recipients = load_recipients(connection, &sequences)?;
    rows.into_iter()
        .map(
            |StoredSummary {
                 sequence,
                 identifier,
                 subject,
                 preview,
                 received_at,
                 size,
                 attachment_count,
             }| {
                let mailboxes = recipients.remove(&sequence).unwrap_or_default();
                Ok(MessageSummary {
                    sequence: MessageSequence(
                        u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
                    ),
                    identifier: parse_identifier(&identifier)?,
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

/// List message summaries newest first using filter and keyset bounds.
pub(super) fn list_messages_blocking(
    connection: &mut Connection,
    query: MessageListQuery,
) -> Result<Vec<MessageSummary>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let mut parameters: Vec<SqlValue> = Vec::new();
    let filter_clause = match &query.filter {
        Some(expression) => {
            let compiled = compile(expression);
            parameters.extend(compiled.parameters);
            compiled.sql
        }
        None => "1".to_owned(),
    };
    let before_clause = match query.before {
        Some(sequence) => {
            parameters.push(SqlValue::Integer(
                i64::try_from(sequence.0).map_err(|_| StorageError::IntegerRange)?,
            ));
            "mail.sequence < ?"
        }
        None => "1",
    };
    let limit = query.limit.min(MAXIMUM_PAGE_SIZE);
    parameters.push(SqlValue::Integer(
        i64::try_from(limit).map_err(|_| StorageError::IntegerRange)?,
    ));
    let sql = format!(
        "{SUMMARY_SELECT} WHERE ({filter_clause}) AND ({before_clause}) ORDER BY mail.sequence DESC LIMIT ?"
    );
    let rows: Vec<StoredSummary> = {
        let mut statement = transaction.prepare(&sql).storage()?;
        statement
            .query_map(params_from_iter(parameters), summary_row)
            .storage()?
            .collect::<Result<_, _>>()
            .storage()?
    };
    let summaries = decode_summaries(&transaction, rows)?;
    transaction.commit().storage()?;
    Ok(summaries)
}

/// Load lightweight summaries in caller-supplied identifier order, omitting missing IDs.
pub(super) fn hydrate_messages_blocking(
    connection: &mut Connection,
    identifiers: &[MessageIdentifier],
) -> Result<Vec<MessageSummary>, StorageError> {
    if identifiers.is_empty() {
        return Ok(Vec::new());
    }
    let transaction = connection.transaction().storage()?;
    let values: Vec<String> = identifiers.iter().map(ToString::to_string).collect();
    let mut summaries = Vec::new();
    for chunk in values.chunks(500) {
        let placeholders = (1..=chunk.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("{SUMMARY_SELECT} WHERE identifier IN ({placeholders})");
        let mut statement = transaction.prepare(&sql).storage()?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter()), summary_row)
            .storage()?
            .collect::<Result<Vec<_>, _>>()
            .storage()?;
        summaries.extend(decode_summaries(&transaction, rows)?);
    }
    transaction.commit().storage()?;
    let by_identifier: std::collections::HashMap<_, _> = summaries
        .into_iter()
        .map(|summary| (summary.identifier, summary))
        .collect();
    Ok(identifiers
        .iter()
        .filter_map(|identifier| by_identifier.get(identifier).cloned())
        .collect())
}

/// Hydrate exact current revisions in request order, omitting stale or deleted search hits.
pub(super) fn hydrate_current_messages_blocking(
    connection: &mut Connection,
    requested: &[(MessageIdentifier, u64)],
) -> Result<Vec<MessageSummary>, StorageError> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let values = requested
        .iter()
        .map(|(identifier, revision)| {
            Ok((
                identifier.to_string(),
                i64::try_from(*revision).map_err(|_| StorageError::IntegerRange)?,
            ))
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    let transaction = connection.transaction().storage()?;
    let mut by_revision = std::collections::HashMap::new();
    for chunk in values.chunks(250) {
        let placeholders = (0..chunk.len())
            .map(|index| format!("(?{},?{})", index * 2 + 1, index * 2 + 2))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "{SUMMARY_SELECT_WITH_REVISION} WHERE (identifier,search_revision) IN (VALUES {placeholders})"
        );
        let parameters = chunk.iter().flat_map(|(identifier, revision)| {
            [
                SqlValue::Text(identifier.clone()),
                SqlValue::Integer(*revision),
            ]
        });
        let rows = transaction
            .prepare(&sql)
            .storage()?
            .query_map(params_from_iter(parameters), |row| {
                Ok((summary_row(row)?, row.get::<_, i64>(7)?))
            })
            .storage()?
            .collect::<Result<Vec<_>, _>>()
            .storage()?;
        let (summaries, revisions): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
        for (summary, revision) in decode_summaries(&transaction, summaries)?
            .into_iter()
            .zip(revisions)
        {
            let revision = u64::try_from(revision).map_err(|_| StorageError::IntegerRange)?;
            by_revision.insert((summary.identifier, revision), summary);
        }
    }
    transaction.commit().storage()?;
    Ok(requested
        .iter()
        .filter_map(|key| by_revision.get(key).cloned())
        .collect())
}
