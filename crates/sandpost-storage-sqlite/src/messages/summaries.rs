//! Bounded summary listing and hydration with keyset and revision checks.
use super::parts::load_recipients;
use crate::error::StorageResult;
use crate::filter::{SqlValue, bind_query, compile};
use crate::records::parse_identifier;
use sandpost_core::{MessageIdentifier, MessageSequence};
use sandpost_storage::{MessageListQuery, MessageSummary, StorageError};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

/// Maximum page size for summary listings.
const MAXIMUM_PAGE_SIZE: usize = 100;

const SUMMARY_SELECT: &str = "SELECT mail.sequence, mail.identifier, mail.subject, substr(CASE WHEN mail.text_body != '' THEN mail.text_body ELSE mail.markup_body END, 1, 160) AS preview, mail.received_at, mail.size, (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence) AS attachment_count FROM mail";
/// Like [SUMMARY_SELECT] with the durable revision appended for exact-revision hydration.
const SUMMARY_SELECT_WITH_REVISION: &str = "SELECT mail.sequence, mail.identifier, mail.subject, substr(CASE WHEN mail.text_body != '' THEN mail.text_body ELSE mail.markup_body END, 1, 160) AS preview, mail.received_at, mail.size, (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence) AS attachment_count, mail.search_revision FROM mail";

/// Selected scalar summary fields before domain validation and recipient hydration.
struct StoredSummary {
    sequence: i64,
    identifier: String,
    subject: String,
    preview: String,
    received_at: i64,
    size: i64,
    attachment_count: i64,
}

/// Read selected summary columns by name, including aliased computed values.
fn summary_row(database_row: &sqlx::sqlite::SqliteRow) -> Result<StoredSummary, sqlx::Error> {
    Ok(StoredSummary {
        sequence: database_row.try_get("sequence")?,
        identifier: database_row.try_get("identifier")?,
        subject: database_row.try_get("subject")?,
        preview: database_row.try_get("preview")?,
        received_at: database_row.try_get("received_at")?,
        size: database_row.try_get("size")?,
        attachment_count: database_row.try_get("attachment_count")?,
    })
}

/// Decode scalar summaries and load recipient relations in the same snapshot.
async fn decode_summaries(
    connection: &mut SqliteConnection,
    rows: Vec<StoredSummary>,
) -> Result<Vec<MessageSummary>, StorageError> {
    let sequences: Vec<_> = rows.iter().map(|row| row.sequence).collect();
    let mut recipients = load_recipients(connection, &sequences).await?;
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

/// List message summaries newest first using canonical filter and keyset bounds.
pub(super) async fn list_messages(
    connection: &mut SqliteConnection,
    query: MessageListQuery,
) -> Result<Vec<MessageSummary>, StorageError> {
    let mut parameters = Vec::new();
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
    let rows = bind_query(sqlx::query(sqlx::AssertSqlSafe(sql)), &parameters)
        .try_map(|row| summary_row(&row))
        .fetch_all(&mut *connection)
        .await
        .storage()?;
    decode_summaries(connection, rows).await
}

/// Load lightweight summaries in caller-supplied identifier order, omitting missing IDs.
pub(super) async fn hydrate_messages(
    connection: &mut SqliteConnection,
    identifiers: &[MessageIdentifier],
) -> Result<Vec<MessageSummary>, StorageError> {
    if identifiers.is_empty() {
        return Ok(Vec::new());
    }
    let values: Vec<String> = identifiers.iter().map(ToString::to_string).collect();
    let mut summaries = Vec::new();
    for chunk in values.chunks(500) {
        let mut query =
            QueryBuilder::<Sqlite>::new(format!("{SUMMARY_SELECT} WHERE identifier IN ("));
        for (index, identifier) in chunk.iter().enumerate() {
            if index != 0 {
                query.push(",");
            }
            query.push_bind(identifier);
        }
        query.push(")");
        let rows = query
            .build()
            .try_map(|row| summary_row(&row))
            .fetch_all(&mut *connection)
            .await
            .storage()?;
        summaries.extend(decode_summaries(connection, rows).await?);
    }
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
pub(super) async fn hydrate_current_messages(
    connection: &mut SqliteConnection,
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
    let mut by_revision = std::collections::HashMap::new();
    for chunk in values.chunks(250) {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "{SUMMARY_SELECT_WITH_REVISION} WHERE (identifier,search_revision) IN (VALUES "
        ));
        for (index, (identifier, revision)) in chunk.iter().enumerate() {
            if index != 0 {
                query.push(",");
            }
            query
                .push("(")
                .push_bind(identifier)
                .push(",")
                .push_bind(revision)
                .push(")");
        }
        query.push(")");
        let rows = query
            .build()
            .try_map(|row| {
                let revision = row.try_get::<i64, _>("search_revision")?;
                Ok((summary_row(&row)?, revision))
            })
            .fetch_all(&mut *connection)
            .await
            .storage()?;
        let (summaries, revisions): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
        for (summary, revision) in decode_summaries(connection, summaries)
            .await?
            .into_iter()
            .zip(revisions)
        {
            let revision = u64::try_from(revision).map_err(|_| StorageError::IntegerRange)?;
            by_revision.insert((summary.identifier, revision), summary);
        }
    }
    Ok(requested
        .iter()
        .filter_map(|key| by_revision.get(key).cloned())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::summary_row;
    use sqlx::Connection;

    /// Reordered columns keep their meaning and missing fields return a decode error.
    #[tokio::test]
    async fn summary_decoding_uses_names_and_reports_missing_columns() {
        let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        let row = sqlx::query(
            "SELECT 37 AS sequence, 'preview' AS preview, 'subject' AS subject, 'identifier' AS identifier, 4096 AS size, 1710000000 AS received_at, 2 AS attachment_count",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        let summary = summary_row(&row).unwrap();
        assert_eq!(summary.sequence, 37);
        assert_eq!(summary.identifier, "identifier");
        assert_eq!(summary.subject, "subject");
        assert_eq!(summary.preview, "preview");
        assert_eq!(summary.received_at, 1710000000);
        assert_eq!(summary.size, 4096);
        assert_eq!(summary.attachment_count, 2);

        let missing = sqlx::query("SELECT 37 AS sequence, 'identifier' AS identifier")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert!(matches!(
            summary_row(&missing),
            Err(sqlx::Error::ColumnNotFound(name)) if name == "subject"
        ));
    }
}
