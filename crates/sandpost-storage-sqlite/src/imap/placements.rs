//! Authorized Web mailbox placement writes.

use crate::error::StorageResult;
use crate::filter::{bind_query, compile};
use sandpost_core::{ImapFolderIdentifier, MessageIdentifier, UserIdentifier};
use sandpost_storage::{FilterExpression, StorageError};
use sqlx::{Row, SqliteConnection};
use std::collections::HashSet;

/// Atomically place only requested messages still authorized and not tombstoned for the owner.
pub(super) async fn place_user_messages(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    identifiers: &[MessageIdentifier],
    folder: Option<ImapFolderIdentifier>,
    authorization: &FilterExpression,
) -> Result<u64, StorageError> {
    if identifiers.len() > 256 {
        return Err(StorageError::ConstraintViolation(
            "a placement batch may contain at most 256 messages".into(),
        ));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE identifier = ?1)")
            .bind(owner.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    if !exists {
        return Err(StorageError::NotFound);
    }
    if let Some(folder) = folder {
        let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM web_folders WHERE identifier = ?1 AND owner_identifier = ?2)")
            .bind(folder.to_string()).bind(owner.to_string()).fetch_one(&mut *connection).await.storage()?;
        if !owned {
            return Err(StorageError::NotFound);
        }
    }
    let compiled = compile(authorization);
    let sql = format!(
        "SELECT mail.sequence FROM mail WHERE mail.identifier = ? AND ({}) AND NOT EXISTS (SELECT 1 FROM user_message_deletions WHERE user_identifier = ? AND mail_sequence = mail.sequence)",
        compiled.sql
    );
    let mut changed = 0_u64;
    let mut unique = HashSet::new();
    for identifier in identifiers {
        if !unique.insert(identifier) {
            continue;
        }
        let row = bind_query(
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(identifier.to_string()),
            &compiled.parameters,
        )
        .bind(owner.to_string())
        .fetch_optional(&mut *connection)
        .await
        .storage()?;
        let Some(row) = row else {
            continue;
        };
        let sequence: i64 = row.try_get("sequence").storage()?;
        sqlx::query("INSERT INTO user_message_placements(owner_identifier, mail_sequence, folder_identifier) VALUES (?1, ?2, ?3) ON CONFLICT(owner_identifier, mail_sequence) DO UPDATE SET folder_identifier = excluded.folder_identifier")
            .bind(owner.to_string()).bind(sequence).bind(folder.map(|value| value.to_string()))
            .execute(&mut *connection).await.storage()?;
        changed += 1;
    }
    if changed > 0 {
        sqlx::query("UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier IN (SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1)")
            .bind(owner.to_string()).execute(connection).await.storage()?;
    }
    Ok(changed)
}
