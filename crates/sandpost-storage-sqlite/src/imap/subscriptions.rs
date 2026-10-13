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
        .execute(&mut *connection)
        .await
        .storage()?;
        sqlx::query(
            "INSERT OR IGNORE INTO imap_subscribed_mailboxes(account_identifier, name) VALUES (?1, ?2)",
        )
        .bind(account.to_string())
        .bind(name)
        .execute(&mut *connection)
        .await
        .storage()?;
    } else {
        sqlx::query(
            "DELETE FROM imap_subscribed_mailboxes WHERE account_identifier = ?1 AND name = ?2",
        )
        .bind(account.to_string())
        .bind(name)
        .execute(&mut *connection)
        .await
        .storage()?;
        sqlx::query("INSERT OR IGNORE INTO imap_unsubscribed_mailboxes(account_identifier, name) VALUES (?1, ?2)")
            .bind(account.to_string()).bind(name).execute(&mut *connection).await.storage()?;
    }
    Ok(())
}

/// Insert retained defaults unless an explicit unsubscribe exists for that name.
pub(super) async fn retain_subscriptions(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    names: &[String],
) -> Result<(), StorageError> {
    for name in names {
        sqlx::query(
            "INSERT OR IGNORE INTO imap_subscribed_mailboxes(account_identifier, name) \
             SELECT ?1, ?2 WHERE NOT EXISTS (\
                 SELECT 1 FROM imap_unsubscribed_mailboxes \
                 WHERE account_identifier = ?1 AND name = ?2\
             )",
        )
        .bind(account.to_string())
        .bind(name)
        .execute(&mut *connection)
        .await
        .storage()?;
    }
    Ok(())
}

/// List retained subscribed names (explicit and observed defaults), including absent names.
pub(super) async fn list_subscribed(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<Vec<String>, StorageError> {
    sqlx::query_scalar(
        "SELECT name FROM imap_subscribed_mailboxes WHERE account_identifier = ?1 ORDER BY name",
    )
    .bind(account.to_string())
    .fetch_all(connection)
    .await
    .storage()
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
