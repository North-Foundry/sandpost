//! IMAP mailbox subscription preferences.

use crate::error::StorageResult;
use rusqlite::{Connection, params};
use sandpost_core::ImapAccountIdentifier;
use sandpost_storage::StorageError;

/// Set whether an account subscribes to a mailbox name.
pub(super) fn set_subscription_blocking(
    connection: &Connection,
    account: ImapAccountIdentifier,
    name: &str,
    subscribed: bool,
) -> Result<(), StorageError> {
    if subscribed {
        connection.execute(
            "DELETE FROM imap_unsubscribed_mailboxes WHERE account_identifier = ?1 AND name = ?2",
            params![account.to_string(), name],
        ).storage()?;
    } else {
        connection.execute(
            "INSERT OR IGNORE INTO imap_unsubscribed_mailboxes(account_identifier, name) VALUES (?1, ?2)",
            params![account.to_string(), name],
        ).storage()?;
    }
    Ok(())
}

/// List mailbox names an account has unsubscribed from.
pub(super) fn list_unsubscribed_blocking(
    connection: &Connection,
    account: ImapAccountIdentifier,
) -> Result<Vec<String>, StorageError> {
    let mut statement = connection
        .prepare("SELECT name FROM imap_unsubscribed_mailboxes WHERE account_identifier = ?1 ORDER BY name")
        .storage()?;
    statement
        .query_map([account.to_string()], |row| row.get(0))
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}
