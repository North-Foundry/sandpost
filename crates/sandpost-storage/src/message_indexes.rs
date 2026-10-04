//! Recipient and header projections maintained during message ingestion.
use crate::StorageError;
use rusqlite::{Transaction, params};
use sandpost_core::Message;

/// Index a message's recipient addresses and ordered headers inside its insert transaction.
pub(crate) fn index_message(
    transaction: &Transaction<'_>,
    sequence: i64,
    message: &Message,
) -> Result<(), StorageError> {
    let mut recipients = transaction.prepare_cached("INSERT OR IGNORE INTO message_recipients(message_seq, address, domain) VALUES (?1, ?2, ?3)")?;
    for mailbox in message
        .facts
        .envelope_to
        .iter()
        .chain(&message.facts.to)
        .chain(&message.facts.carbon_copy)
    {
        recipients.execute(params![sequence, mailbox.address, mailbox.domain])?;
    }
    drop(recipients);
    let mut headers = transaction.prepare_cached(
        "INSERT INTO message_headers(message_seq, name, value, ordinal) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (name, values) in &message.facts.headers {
        for (ordinal, value) in values.iter().enumerate() {
            headers.execute(params![
                sequence,
                name,
                value,
                i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?
            ])?;
        }
    }
    Ok(())
}
