//! Persistence and reconstruction for normalized mail child records.
use crate::StorageError;
use crate::error::StorageResult;
use sandpost_core::{Attachment, Mailbox, Message};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use std::collections::BTreeMap;

/// Mailbox addresses grouped by their original semantic role and order.
#[derive(Default)]
pub(super) struct MailRecipients {
    pub(super) envelope_from: Option<Mailbox>,
    pub(super) envelope_to: Vec<Mailbox>,
    pub(super) from: Vec<Mailbox>,
    pub(super) to: Vec<Mailbox>,
    pub(super) carbon_copy: Vec<Mailbox>,
}

/// Persist each ordered recipient, header value, and attachment inside the insert transaction.
pub(super) async fn store_parts(
    connection: &mut SqliteConnection,
    sequence: i64,
    message: &Message,
) -> Result<(), StorageError> {
    const MAXIMUM_BATCH_ROWS: usize = 128;

    let mut recipients: Vec<(&str, i64, &Mailbox)> = Vec::new();
    if let Some(mailbox) = &message.facts.envelope_from {
        recipients.push(("envelope_from", 0, mailbox));
    }
    for (recipient_type, mailboxes) in [
        ("envelope_to", &message.facts.envelope_to),
        ("from", &message.facts.from),
        ("to", &message.facts.to),
        ("carbon_copy", &message.facts.carbon_copy),
    ] {
        for (ordinal, mailbox) in mailboxes.iter().enumerate() {
            recipients.push((
                recipient_type,
                i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?,
                mailbox,
            ));
        }
    }
    for batch in recipients.chunks(MAXIMUM_BATCH_ROWS) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO mail_recipients(mail_sequence, recipient_type, ordinal, address, domain) ",
        );
        query.push_values(batch, |mut row, (recipient_type, ordinal, mailbox)| {
            row.push_bind(sequence)
                .push_bind(*recipient_type)
                .push_bind(*ordinal)
                .push_bind(&mailbox.address)
                .push_bind(&mailbox.domain);
        });
        query.build().execute(&mut *connection).await.storage()?;
    }

    let mut headers = Vec::new();
    for (name, values) in &message.facts.headers {
        for (ordinal, value) in values.iter().enumerate() {
            headers.push((
                name.as_str(),
                value.as_str(),
                i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?,
            ));
        }
    }
    for batch in headers.chunks(MAXIMUM_BATCH_ROWS) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO mail_headers(mail_sequence, name, value, ordinal) ",
        );
        query.push_values(batch, |mut row, (name, value, ordinal)| {
            row.push_bind(sequence)
                .push_bind(*name)
                .push_bind(*value)
                .push_bind(*ordinal);
        });
        query.build().execute(&mut *connection).await.storage()?;
    }

    let attachments = message
        .attachments
        .iter()
        .enumerate()
        .map(|(ordinal, attachment)| {
            Ok((
                i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?,
                i64::try_from(attachment.size).map_err(|_| StorageError::IntegerRange)?,
                attachment,
            ))
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    for batch in attachments.chunks(MAXIMUM_BATCH_ROWS) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO mail_attachments(mail_sequence, ordinal, filename, content_type, size, content_hash) ",
        );
        query.push_values(batch, |mut row, (ordinal, size, attachment)| {
            row.push_bind(sequence)
                .push_bind(*ordinal)
                .push_bind(&attachment.filename)
                .push_bind(&attachment.content_type)
                .push_bind(*size)
                .push_bind(&attachment.content_hash);
        });
        query.build().execute(&mut *connection).await.storage()?;
    }
    Ok(())
}

/// Load ordered recipient roles for a bounded page, transferring one JSON row per message.
/// The transient projection reduces SQLite worker row/value copies without changing stored facts.
pub(super) async fn load_recipients(
    connection: &mut SqliteConnection,
    sequences: &[i64],
) -> Result<BTreeMap<i64, MailRecipients>, StorageError> {
    let mut recipients: BTreeMap<i64, MailRecipients> = sequences
        .iter()
        .copied()
        .map(|sequence| (sequence, MailRecipients::default()))
        .collect();
    if sequences.is_empty() {
        return Ok(recipients);
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT mail_sequence, json_group_array(json_array(recipient_type, ordinal, address, domain)) AS recipients FROM (SELECT mail_sequence, recipient_type, ordinal, address, domain FROM mail_recipients WHERE mail_sequence IN (",
    );
    for (index, sequence) in sequences.iter().enumerate() {
        if index != 0 {
            query.push(",");
        }
        query.push_bind(sequence);
    }
    query.push(") ORDER BY mail_sequence, recipient_type, ordinal) GROUP BY mail_sequence");
    let rows = query.build().fetch_all(&mut *connection).await.storage()?;
    for row in rows {
        let sequence: i64 = row.try_get("mail_sequence").storage()?;
        let encoded: String = row.try_get("recipients").storage()?;
        let records: Vec<(String, i64, String, String)> = serde_json::from_str(&encoded)
            .map_err(|_| StorageError::InvalidData("invalid stored recipients".into()))?;
        let Some(group) = recipients.get_mut(&sequence) else {
            return Err(StorageError::InvalidData(format!(
                "recipient references unrequested message sequence {sequence}"
            )));
        };
        for (recipient_type, ordinal, address, domain) in records {
            let mailbox = Mailbox { address, domain };
            match recipient_type.as_str() {
                "envelope_from" if ordinal == 0 && group.envelope_from.is_none() => {
                    group.envelope_from = Some(mailbox);
                }
                "envelope_from" => {
                    return Err(StorageError::InvalidData(
                        "envelope_from must have ordinal zero and one mailbox".into(),
                    ));
                }
                "envelope_to" => group.envelope_to.push(mailbox),
                "from" => group.from.push(mailbox),
                "to" => group.to.push(mailbox),
                "carbon_copy" => group.carbon_copy.push(mailbox),
                _ => {
                    return Err(StorageError::InvalidData(format!(
                        "unknown recipient type {recipient_type}"
                    )));
                }
            }
        }
    }
    Ok(recipients)
}

/// Load one mail's duplicate header values in their original order for each name.
pub(super) async fn load_headers(
    connection: &mut SqliteConnection,
    sequence: i64,
) -> Result<BTreeMap<String, Vec<String>>, StorageError> {
    let rows = sqlx::query(
        "SELECT name, value FROM mail_headers WHERE mail_sequence = ? ORDER BY name, ordinal",
    )
    .bind(sequence)
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        headers
            .entry(row.try_get("name").storage()?)
            .or_default()
            .push(row.try_get("value").storage()?);
    }
    Ok(headers)
}

/// Load one mail's attachment metadata in order and validate unsigned sizes.
pub(super) async fn load_attachments(
    connection: &mut SqliteConnection,
    sequence: i64,
) -> Result<Vec<Attachment>, StorageError> {
    let rows = sqlx::query(
        "SELECT filename, content_type, size, content_hash FROM mail_attachments WHERE mail_sequence = ? ORDER BY ordinal",
    )
    .bind(sequence)
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    rows.into_iter()
        .map(|row| {
            let size: i64 = row.try_get("size").storage()?;
            Ok(Attachment {
                filename: row.try_get("filename").storage()?,
                content_type: row.try_get("content_type").storage()?,
                size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
                content_hash: row.try_get("content_hash").storage()?,
            })
        })
        .collect()
}
