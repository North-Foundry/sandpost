//! Full-message and metadata reads from authoritative SQL snapshots.
use super::parts::{load_attachments, load_headers, load_recipients};
use crate::error::StorageResult;
use rusqlite::{Connection, OptionalExtension};
use sandpost_core::{Attachment, Message, MessageFacts, MessageIdentifier};
use sandpost_storage::StorageError;

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

/// Load normalized facts, attachments, and raw bytes in one read transaction.
pub(super) fn get_message_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
) -> Result<Option<Message>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let stored = transaction
        .query_row(
            "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size, raw_message FROM mail WHERE identifier = ?1",
            [identifier.to_string()],
            |row| Ok((mail_row(row)?, row.get::<_, Vec<u8>>(7)?)),
        )
        .optional()
        .storage()?;
    let result = stored
        .map(|(mail, raw_message)| -> Result<_, StorageError> {
            let (facts, attachments) = load_message_parts(&transaction, mail)?;
            Ok(Message {
                identifier,
                facts,
                raw_message,
                attachments,
            })
        })
        .transpose()?;
    transaction.commit().storage()?;
    Ok(result)
}

/// Load normalized facts and attachments without raw bytes.
pub(super) fn get_message_metadata_blocking(
    connection: &mut Connection,
    identifier: MessageIdentifier,
) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let stored = transaction
        .query_row(
            "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size FROM mail WHERE identifier = ?1",
            [identifier.to_string()],
            mail_row,
        )
        .optional()
        .storage()?;
    let result = stored
        .map(|mail| load_message_parts(&transaction, mail))
        .transpose()?;
    transaction.commit().storage()?;
    Ok(result)
}
