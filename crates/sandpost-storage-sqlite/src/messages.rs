//! Atomic message ingestion and public message retrieval on SQLite.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::filter::compile;
use crate::mail_parts::{load_attachments, load_headers, load_recipients, store_parts};
use crate::records::{StoredSummary, decode_summaries, parse_identifier, summary_row};
use async_trait::async_trait;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use sandpost_core::{
    Attachment, EndpointIdentifier, Message, MessageFacts, MessageIdentifier, MessageSequence,
};
use sandpost_storage::{
    IndexedMessage, MessageListQuery, MessageStorage, MessageSummary, StorageError,
};

/// Public maximum page size for summary listings.
pub(crate) const MAXIMUM_PAGE_SIZE: usize = 100;
const MAXIMUM_INDEX_BATCH_SIZE: usize = 64;

const SUMMARY_SELECT: &str = "SELECT mail.sequence, mail.identifier, mail.subject, substr(CASE WHEN mail.text_body != '' THEN mail.text_body ELSE mail.markup_body END, 1, 160), mail.received_at, mail.size, (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence), mail.endpoint_identifier FROM mail";
/// Like [SUMMARY_SELECT] but with the durable revision appended for exact-revision hydration.
const SUMMARY_SELECT_WITH_REVISION: &str = "SELECT mail.sequence, mail.identifier, mail.subject, substr(CASE WHEN mail.text_body != '' THEN mail.text_body ELSE mail.markup_body END, 1, 160), mail.received_at, mail.size, (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence), mail.endpoint_identifier, mail.search_revision FROM mail";

/// Scalar index input selected together with its durable revision.
struct StoredIndexedMessage {
    sequence: i64,
    identifier: String,
    endpoint_identifier: String,
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
        endpoint_identifier: row.get(2)?,
        subject: row.get(3)?,
        text_body: row.get(4)?,
        markup_body: row.get(5)?,
        message_identifier: row.get(6)?,
        received_at: row.get(7)?,
        size: row.get(8)?,
        revision: row.get(9)?,
    })
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
    connection: &Connection,
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

/// Insert a normalized message and all relational facts atomically for one endpoint.
fn insert_message_blocking(
    connection: &mut Connection,
    message: &Message,
    endpoint: EndpointIdentifier,
) -> Result<MessageSequence, StorageError> {
    let transaction = connection.transaction().storage()?;
    let inserted = transaction
        .execute(
            "INSERT INTO mail(identifier, subject, text_body, markup_body, message_identifier, raw_message, received_at, size, endpoint_identifier) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(identifier) DO NOTHING",
            params![
                message.identifier.to_string(),
                message.facts.subject,
                message.facts.text,
                message.facts.markup_body,
                message.facts.message_identifier,
                message.raw_message,
                message.facts.received_at,
                i64::try_from(message.facts.size).map_err(|_| StorageError::IntegerRange)?,
                endpoint.to_string(),
            ],
        )
        .storage()?;
    if inserted == 0 {
        return Err(StorageError::DuplicateMessageIdentifier(message.identifier));
    }
    let stored_sequence = transaction.last_insert_rowid();
    let sequence =
        MessageSequence(u64::try_from(stored_sequence).map_err(|_| StorageError::IntegerRange)?);
    store_parts(&transaction, stored_sequence, message)?;
    transaction.commit().storage()?;
    Ok(sequence)
}

/// Load endpoint, normalized facts, attachments, and raw bytes in one read transaction.
fn get_message_with_endpoint_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
) -> Result<Option<(EndpointIdentifier, Message)>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let stored = transaction
        .query_row(
            "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size, endpoint_identifier, raw_message FROM mail WHERE identifier = ?1",
            [identifier.to_string()],
            |row| {
                Ok((
                    mail_row(row)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                ))
            },
        )
        .optional()
        .storage()?;
    let result = stored
        .map(|(mail, endpoint, raw_message)| -> Result<_, StorageError> {
            let (facts, attachments) = load_message_parts(&transaction, mail)?;
            Ok((
                parse_identifier(&endpoint)?,
                Message {
                    identifier,
                    facts,
                    raw_message,
                    attachments,
                },
            ))
        })
        .transpose()?;
    transaction.commit().storage()?;
    Ok(result)
}

/// Load endpoint, normalized facts, and attachments without raw bytes.
fn get_message_metadata_with_endpoint_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
) -> Result<Option<(EndpointIdentifier, MessageFacts, Vec<Attachment>)>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let stored = transaction
        .query_row(
            "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size, endpoint_identifier FROM mail WHERE identifier = ?1",
            [identifier.to_string()],
            |row| Ok((mail_row(row)?, row.get::<_, String>(7)?)),
        )
        .optional()
        .storage()?;
    let result = stored
        .map(|(mail, endpoint)| -> Result<_, StorageError> {
            let (facts, attachments) = load_message_parts(&transaction, mail)?;
            Ok((parse_identifier(&endpoint)?, facts, attachments))
        })
        .transpose()?;
    transaction.commit().storage()?;
    Ok(result)
}

/// List message summaries newest first using endpoint, filter, and keyset bounds.
fn list_messages_blocking(
    connection: &mut Connection,
    query: MessageListQuery,
) -> Result<Vec<MessageSummary>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let mut parameters: Vec<SqlValue> = Vec::new();
    let endpoint_clause = match query.endpoint {
        Some(endpoint) => {
            parameters.push(SqlValue::Text(endpoint.to_string()));
            "mail.endpoint_identifier = ?"
        }
        None => "1",
    };
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
        "{SUMMARY_SELECT} WHERE ({endpoint_clause}) AND ({filter_clause}) AND ({before_clause}) ORDER BY mail.sequence DESC LIMIT ?"
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

/// Delete a message and its cascading child facts in one transaction.
fn delete_message_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
) -> Result<bool, StorageError> {
    let transaction = connection.transaction().storage()?;
    let deleted = transaction
        .execute(
            "DELETE FROM mail WHERE identifier = ?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0;
    transaction.commit().storage()?;
    Ok(deleted)
}

/// Delete only the exact authorized revision, returning false for stale or missing mail.
fn delete_message_at_revision_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
    revision: u64,
) -> Result<bool, StorageError> {
    let revision = i64::try_from(revision).map_err(|_| StorageError::IntegerRange)?;
    let transaction = connection.transaction().storage()?;
    let deleted = transaction
        .execute(
            "DELETE FROM mail WHERE identifier = ?1 AND search_revision = ?2",
            params![identifier.to_string(), revision],
        )
        .storage()?
        > 0;
    transaction.commit().storage()?;
    Ok(deleted)
}

/// Return the endpoint currently owning a stored message.
fn message_endpoint_blocking(
    connection: &Connection,
    identifier: MessageIdentifier,
) -> Result<Option<EndpointIdentifier>, StorageError> {
    let endpoint: Option<String> = connection
        .query_row(
            "SELECT endpoint_identifier FROM mail WHERE identifier=?1",
            [identifier.to_string()],
            |row| row.get(0),
        )
        .optional()
        .storage()?;
    endpoint.map(|value| parse_identifier(&value)).transpose()
}

/// Return the highest stored mail sequence, or zero when the database is empty.
fn max_message_sequence_blocking(connection: &Connection) -> Result<MessageSequence, StorageError> {
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

/// Load lightweight summaries in caller-supplied identifier order, omitting missing IDs.
fn hydrate_messages_blocking(
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
fn hydrate_current_messages_blocking(
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
                Ok((summary_row(row)?, row.get::<_, i64>(8)?))
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

/// Read normalized index records and revisions after an exclusive sequence, bounded by limit.
fn index_messages_blocking(
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
        .prepare("SELECT sequence,identifier,endpoint_identifier,subject,text_body,markup_body,message_identifier,received_at,size,search_revision FROM mail WHERE sequence>?1 AND (?2 IS NULL OR sequence<=?2) ORDER BY sequence LIMIT ?3")
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
fn indexed_messages_blocking(
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
        "SELECT sequence,identifier,endpoint_identifier,subject,text_body,markup_body,message_identifier,received_at,size,search_revision FROM mail WHERE identifier IN ({placeholders})"
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

/// Batch-load recipient roles, headers, and attachments for selected mail rows.
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
    let mut attachments_by_sequence = std::collections::HashMap::<i64, Vec<Attachment>>::new();
    let query = format!(
        "SELECT mail_sequence,filename,content_type,size,content_hash FROM mail_attachments WHERE mail_sequence IN ({placeholders}) ORDER BY mail_sequence,ordinal"
    );
    {
        let mut statement = connection.prepare(&query).storage()?;
        let mut result = statement
            .query(params_from_iter(sequences.iter()))
            .storage()?;
        while let Some(row) = result.next().storage()? {
            let sequence: i64 = row.get(0).storage()?;
            let size: i64 = row.get(3).storage()?;
            attachments_by_sequence
                .entry(sequence)
                .or_default()
                .push(Attachment {
                    filename: row.get(1).storage()?,
                    content_type: row.get(2).storage()?,
                    size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
                    content_hash: row.get(4).storage()?,
                });
        }
    }
    rows.into_iter()
        .map(
            |StoredIndexedMessage {
                 sequence,
                 identifier,
                 endpoint_identifier,
                 subject,
                 text_body,
                 markup_body,
                 message_identifier,
                 received_at,
                 size,
                 revision,
             }| {
                let mailbox = recipients.get(&sequence);
                let attachments = attachments_by_sequence
                    .remove(&sequence)
                    .unwrap_or_default();
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
                    attachment_count: attachments.len() as u64,
                    headers: headers_by_sequence.remove(&sequence).unwrap_or_default(),
                };
                Ok(IndexedMessage {
                    revision: u64::try_from(revision).map_err(|_| StorageError::IntegerRange)?,
                    identifier: parse_identifier(&identifier)?,
                    sequence: MessageSequence(
                        u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
                    ),
                    endpoint_identifier: parse_identifier(&endpoint_identifier)?,
                    facts,
                    attachments,
                })
            },
        )
        .collect()
}

#[async_trait]
impl MessageStorage for SqliteStorage {
    async fn insert_message(
        &self,
        message: &Message,
        endpoint: EndpointIdentifier,
    ) -> Result<MessageSequence, StorageError> {
        let message = message.clone();
        self.run(move |connection| insert_message_blocking(connection, &message, endpoint))
            .await
    }

    async fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError> {
        self.run(move |connection| {
            Ok(get_message_with_endpoint_blocking(connection, identifier)?
                .map(|(_, message)| message))
        })
        .await
    }

    async fn get_message_with_endpoint(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(EndpointIdentifier, Message)>, StorageError> {
        self.run(move |connection| get_message_with_endpoint_blocking(connection, identifier))
            .await
    }

    async fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError> {
        self.run(move |connection| {
            Ok(
                get_message_metadata_with_endpoint_blocking(connection, identifier)?
                    .map(|(_, facts, attachments)| (facts, attachments)),
            )
        })
        .await
    }

    async fn get_message_metadata_with_endpoint(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(EndpointIdentifier, MessageFacts, Vec<Attachment>)>, StorageError> {
        self.run(move |connection| {
            get_message_metadata_with_endpoint_blocking(connection, identifier)
        })
        .await
    }

    async fn list_messages(
        &self,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        self.run(move |connection| list_messages_blocking(connection, query))
            .await
    }

    async fn delete_message(&self, identifier: MessageIdentifier) -> Result<bool, StorageError> {
        self.run(move |connection| delete_message_blocking(connection, identifier))
            .await
    }

    async fn delete_message_at_revision(
        &self,
        identifier: MessageIdentifier,
        revision: u64,
    ) -> Result<bool, StorageError> {
        self.run(move |connection| {
            delete_message_at_revision_blocking(connection, identifier, revision)
        })
        .await
    }

    async fn message_endpoint(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<EndpointIdentifier>, StorageError> {
        self.run(move |connection| message_endpoint_blocking(connection, identifier))
            .await
    }

    async fn max_message_sequence(&self) -> Result<MessageSequence, StorageError> {
        self.run(|connection| max_message_sequence_blocking(connection))
            .await
    }

    async fn hydrate_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let identifiers = identifiers.to_vec();
        self.run(move |connection| hydrate_messages_blocking(connection, &identifiers))
            .await
    }

    async fn hydrate_current_messages(
        &self,
        requested: &[(MessageIdentifier, u64)],
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let requested = requested.to_vec();
        self.run(move |connection| hydrate_current_messages_blocking(connection, &requested))
            .await
    }

    async fn index_messages(
        &self,
        after: MessageSequence,
        through: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<IndexedMessage>, StorageError> {
        self.run(move |connection| index_messages_blocking(connection, after, through, limit))
            .await
    }

    async fn indexed_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<IndexedMessage>, StorageError> {
        let identifiers = identifiers.to_vec();
        self.run(move |connection| indexed_messages_blocking(connection, &identifiers))
            .await
    }
}
