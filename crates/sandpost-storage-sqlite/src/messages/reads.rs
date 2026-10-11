//! Full-message and metadata reads from authoritative SQL snapshots.
use super::parts::{load_attachments, load_headers, load_recipients};
use crate::error::StorageResult;
use sandpost_core::{Attachment, Message, MessageFacts, MessageIdentifier};
use sandpost_storage::StorageError;
use sqlx::{Row, SqliteConnection};

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

/// Decode scalar mail columns by their explicit result names.
fn mail_row(row: &sqlx::sqlite::SqliteRow) -> Result<StoredMail, sqlx::Error> {
    Ok(StoredMail {
        sequence: row.try_get("sequence")?,
        subject: row.try_get("subject")?,
        text_body: row.try_get("text_body")?,
        markup_body: row.try_get("markup_body")?,
        message_identifier: row.try_get("message_identifier")?,
        received_at: row.try_get("received_at")?,
        size: row.try_get("size")?,
    })
}

/// Rebuild normalized facts and ordered attachments from one read transaction.
async fn load_message_parts(
    connection: &mut SqliteConnection,
    mail: StoredMail,
) -> Result<(MessageFacts, Vec<Attachment>), StorageError> {
    let recipients = load_recipients(connection, &[mail.sequence])
        .await?
        .remove(&mail.sequence)
        .unwrap_or_default();
    let headers = load_headers(connection, mail.sequence).await?;
    let attachments = load_attachments(connection, mail.sequence).await?;
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

/// Load normalized facts, attachments, and raw bytes from one caller-owned read snapshot.
pub(super) async fn get_message(
    connection: &mut SqliteConnection,
    identifier: MessageIdentifier,
) -> Result<Option<Message>, StorageError> {
    let stored = sqlx::query(
        "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size, raw_message FROM mail WHERE identifier = ?",
    )
    .bind(identifier.to_string())
    .fetch_optional(&mut *connection)
    .await
    .storage()?;
    match stored {
        Some(row) => {
            let mail = mail_row(&row).storage()?;
            let raw_message = row.try_get("raw_message").storage()?;
            let (facts, attachments) = load_message_parts(connection, mail).await?;
            Ok(Some(Message {
                identifier,
                facts,
                raw_message,
                attachments,
            }))
        }
        None => Ok(None),
    }
}

/// Load normalized facts and attachments without selecting raw bytes.
pub(super) async fn get_message_metadata(
    connection: &mut SqliteConnection,
    identifier: MessageIdentifier,
) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError> {
    let stored = sqlx::query(
        "SELECT sequence, subject, text_body, markup_body, message_identifier, received_at, size FROM mail WHERE identifier = ?",
    )
    .bind(identifier.to_string())
    .fetch_optional(&mut *connection)
    .await
    .storage()?;
    match stored {
        Some(row) => Ok(Some(
            load_message_parts(connection, mail_row(&row).storage()?).await?,
        )),
        None => Ok(None),
    }
}
