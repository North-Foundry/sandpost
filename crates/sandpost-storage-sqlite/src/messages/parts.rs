//! Persistence and reconstruction for normalized mail child records.
use crate::StorageError;
use crate::error::StorageResult;
use rusqlite::{Connection, Transaction, params, params_from_iter};
use sandpost_core::{Attachment, Mailbox, Message};
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
pub(super) fn store_parts(
    transaction: &Transaction<'_>,
    sequence: i64,
    message: &Message,
) -> Result<(), StorageError> {
    let mut recipient_statement = transaction
        .prepare_cached(
            "INSERT INTO mail_recipients(mail_sequence, recipient_type, ordinal, address, domain) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .storage()?;
    if let Some(mailbox) = &message.facts.envelope_from {
        recipient_statement
            .execute(params![
                sequence,
                "envelope_from",
                0_i64,
                mailbox.address,
                mailbox.domain
            ])
            .storage()?;
    }
    for (recipient_type, mailboxes) in [
        ("envelope_to", &message.facts.envelope_to),
        ("from", &message.facts.from),
        ("to", &message.facts.to),
        ("carbon_copy", &message.facts.carbon_copy),
    ] {
        for (ordinal, mailbox) in mailboxes.iter().enumerate() {
            recipient_statement
                .execute(params![
                    sequence,
                    recipient_type,
                    i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?,
                    mailbox.address,
                    mailbox.domain,
                ])
                .storage()?;
        }
    }
    drop(recipient_statement);

    let mut header_statement = transaction
        .prepare_cached(
            "INSERT INTO mail_headers(mail_sequence, name, value, ordinal) VALUES (?1, ?2, ?3, ?4)",
        )
        .storage()?;
    for (name, values) in &message.facts.headers {
        for (ordinal, value) in values.iter().enumerate() {
            header_statement
                .execute(params![
                    sequence,
                    name,
                    value,
                    i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?,
                ])
                .storage()?;
        }
    }
    drop(header_statement);

    let mut attachment_statement = transaction
        .prepare_cached(
            "INSERT INTO mail_attachments(mail_sequence, ordinal, filename, content_type, size, content_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .storage()?;
    for (ordinal, attachment) in message.attachments.iter().enumerate() {
        attachment_statement
            .execute(params![
                sequence,
                i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?,
                attachment.filename,
                attachment.content_type,
                i64::try_from(attachment.size).map_err(|_| StorageError::IntegerRange)?,
                attachment.content_hash,
            ])
            .storage()?;
    }
    Ok(())
}

/// Load the semantic recipient roles for a page using one bounded IN query.
pub(super) fn load_recipients(
    connection: &Connection,
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
    let placeholders = placeholders(sequences.len());
    let query_statement = format!(
        "SELECT mail_sequence, recipient_type, ordinal, address, domain FROM mail_recipients WHERE mail_sequence IN ({placeholders}) ORDER BY mail_sequence, recipient_type, ordinal"
    );
    let mut statement = connection.prepare(&query_statement).storage()?;
    let rows = statement
        .query_map(params_from_iter(sequences.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                Mailbox {
                    address: row.get(3)?,
                    domain: row.get(4)?,
                },
            ))
        })
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    for (sequence, recipient_type, ordinal, mailbox) in rows {
        let Some(group) = recipients.get_mut(&sequence) else {
            return Err(StorageError::InvalidData(format!(
                "recipient references unrequested message sequence {sequence}"
            )));
        };
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
    Ok(recipients)
}

/// Load one mail's duplicate header values in their original order for each name.
pub(super) fn load_headers(
    connection: &Connection,
    sequence: i64,
) -> Result<BTreeMap<String, Vec<String>>, StorageError> {
    let mut statement = connection
        .prepare_cached(
            "SELECT name, value FROM mail_headers WHERE mail_sequence = ?1 ORDER BY name, ordinal",
        )
        .storage()?;
    let mut rows = statement.query([sequence]).storage()?;
    let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    while let Some(row) = rows.next().storage()? {
        headers
            .entry(row.get(0).storage()?)
            .or_default()
            .push(row.get(1).storage()?);
    }
    Ok(headers)
}

/// Load one mail's attachment metadata in its original order and validate unsigned sizes.
pub(super) fn load_attachments(
    connection: &Connection,
    sequence: i64,
) -> Result<Vec<Attachment>, StorageError> {
    let mut statement = connection
        .prepare_cached(
            "SELECT filename, content_type, size, content_hash FROM mail_attachments WHERE mail_sequence = ?1 ORDER BY ordinal",
        )
        .storage()?;
    let mut rows = statement.query([sequence]).storage()?;
    let mut attachments = Vec::new();
    while let Some(row) = rows.next().storage()? {
        let size: i64 = row.get(2).storage()?;
        attachments.push(Attachment {
            filename: row.get(0).storage()?,
            content_type: row.get(1).storage()?,
            size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
            content_hash: row.get(3).storage()?,
        });
    }
    Ok(attachments)
}

/// Build positional parameters for an IN query using only generated placeholders.
fn placeholders(count: usize) -> String {
    (0..count).map(|_| "?").collect::<Vec<_>>().join(",")
}
