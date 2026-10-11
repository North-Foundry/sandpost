//! IMAP mailbox subscription preferences.

use crate::error::StorageResult;
use sandpost_core::ImapAccountIdentifier;
use sandpost_storage::StorageError;
use sqlx::SqliteConnection;

/// Set whether an account subscribes to a mailbox name.
pub(super) async fn set_subscription(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    name: &str,
    subscribed: bool,
) -> Result<(), StorageError> {
    if subscribed {
        sqlx::query(
            "DELETE FROM imap_unsubscribed_mailboxes WHERE account_identifier = ?1 AND name = ?2",
        )
        .bind(account.to_string())
        .bind(name)
        .execute(connection)
        .await
        .storage()?;
    } else {
        sqlx::query("INSERT OR IGNORE INTO imap_unsubscribed_mailboxes(account_identifier, name) VALUES (?1, ?2)")
            .bind(account.to_string()).bind(name).execute(connection).await.storage()?;
    }
    Ok(())
}

/// List mailbox names an account has unsubscribed from.
pub(super) async fn list_unsubscribed(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<Vec<String>, StorageError> {
    sqlx::query_scalar(
        "SELECT name FROM imap_unsubscribed_mailboxes WHERE account_identifier = ?1 ORDER BY name",
    )
    .bind(account.to_string())
    .fetch_all(connection)
    .await
    .storage()
}
