//! Authorized Web mailbox placement and Trash writes.

use crate::error::StorageResult;
use crate::filter::{bind_query, compile};
use sandpost_core::{ImapFolderIdentifier, MessageIdentifier, UserIdentifier};
use sandpost_storage::{FilterExpression, StorageError};
use sqlx::{Row, SqliteConnection};
use std::collections::HashSet;

/// The largest explicit message batch one placement or Trash request may contain.
const MAXIMUM_PLACEMENT_BATCH_SIZE: usize = 256;

/// Atomically place only requested messages still authorized and not tombstoned for the owner.
///
/// Placement also restores the message from the owner's Web Trash.
pub(super) async fn place_user_messages(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    identifiers: &[MessageIdentifier],
    folder: Option<ImapFolderIdentifier>,
    authorization: &FilterExpression,
) -> Result<u64, StorageError> {
    if identifiers.len() > MAXIMUM_PLACEMENT_BATCH_SIZE {
        return Err(StorageError::ConstraintViolation(
            "a placement batch may contain at most 256 messages".into(),
        ));
    }
    require_owner(connection, owner).await?;
    if let Some(folder) = folder {
        let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM web_folders WHERE identifier = ?1 AND owner_identifier = ?2)")
            .bind(folder.to_string()).bind(owner.to_string()).fetch_one(&mut *connection).await.storage()?;
        if !owned {
            return Err(StorageError::NotFound);
        }
    }
    let sequences = authorized_sequences(connection, owner, identifiers, authorization).await?;
    for sequence in &sequences {
        sqlx::query(
            "DELETE FROM user_message_copies WHERE owner_identifier = ?1 AND mail_sequence = ?2",
        )
        .bind(owner.to_string())
        .bind(sequence)
        .execute(&mut *connection)
        .await
        .storage()?;
        sqlx::query("INSERT INTO user_message_placements(owner_identifier, mail_sequence, folder_identifier) VALUES (?1, ?2, ?3) ON CONFLICT(owner_identifier, mail_sequence) DO UPDATE SET folder_identifier = excluded.folder_identifier")
            .bind(owner.to_string()).bind(sequence).bind(folder.map(|value| value.to_string()))
            .execute(&mut *connection).await.storage()?;
        sqlx::query(
            "DELETE FROM user_message_trash WHERE owner_identifier = ?1 AND mail_sequence = ?2",
        )
        .bind(owner.to_string())
        .bind(sequence)
        .execute(&mut *connection)
        .await
        .storage()?;
    }
    invalidate_owner_mailboxes(connection, owner, sequences.len()).await
}

/// Add normal primary copies while preserving the primary occurrence, including Trash.
pub(super) async fn copy_user_messages(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    identifiers: &[MessageIdentifier],
    folder: Option<ImapFolderIdentifier>,
    authorization: &FilterExpression,
) -> Result<u64, StorageError> {
    if identifiers.len() > MAXIMUM_PLACEMENT_BATCH_SIZE {
        return Err(StorageError::BatchLimitExceeded(
            MAXIMUM_PLACEMENT_BATCH_SIZE,
        ));
    }
    require_owner(connection, owner).await?;
    if let Some(folder) = folder {
        let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM web_folders WHERE identifier = ?1 AND owner_identifier = ?2)")
            .bind(folder.to_string()).bind(owner.to_string()).fetch_one(&mut *connection).await.storage()?;
        if !owned {
            return Err(StorageError::NotFound);
        }
    }
    let sequences = authorized_sequences(connection, owner, identifiers, authorization).await?;
    if sequences.len() != identifiers.iter().collect::<HashSet<_>>().len() {
        return Err(StorageError::NotFound);
    }
    let owner_text = owner.to_string();
    let folder_text = folder.map(|identifier| identifier.to_string());
    let kind = if folder.is_some() { "folder" } else { "inbox" };
    for sequence in &sequences {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM user_message_copies WHERE owner_identifier = ?1 AND mail_sequence = ?2 AND target_kind = ?3 AND folder_identifier IS ?4) OR \
            (NOT EXISTS(SELECT 1 FROM user_message_trash WHERE owner_identifier = ?1 AND mail_sequence = ?2) AND \
            CASE WHEN ?4 IS NULL THEN NOT EXISTS(SELECT 1 FROM user_message_placements WHERE owner_identifier = ?1 AND mail_sequence = ?2 AND folder_identifier IS NOT NULL) \
            ELSE EXISTS(SELECT 1 FROM user_message_placements WHERE owner_identifier = ?1 AND mail_sequence = ?2 AND folder_identifier = ?4) END)")
            .bind(&owner_text).bind(sequence).bind(kind).bind(&folder_text)
            .fetch_one(&mut *connection).await.storage()?;
        if exists {
            return Err(StorageError::ConstraintViolation(
                "destination already contains the message".into(),
            ));
        }
        sqlx::query("INSERT INTO user_message_copies(owner_identifier, mail_sequence, target_kind, folder_identifier) VALUES (?1, ?2, ?3, ?4)")
            .bind(&owner_text).bind(sequence).bind(kind).bind(&folder_text)
            .execute(&mut *connection).await.storage()?;
    }
    invalidate_owner_mailboxes(connection, owner, sequences.len()).await
}

/// Atomically move requested authorized, undeleted messages into the owner's Web Trash.
///
/// Trash replaces Web folder placement; a message already in Trash keeps its original time.
pub(super) async fn trash_user_messages(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    identifiers: &[MessageIdentifier],
    authorization: &FilterExpression,
    trashed_at: i64,
) -> Result<u64, StorageError> {
    if identifiers.len() > MAXIMUM_PLACEMENT_BATCH_SIZE {
        return Err(StorageError::ConstraintViolation(
            "a Trash batch may contain at most 256 messages".into(),
        ));
    }
    require_owner(connection, owner).await?;
    let sequences = authorized_sequences(connection, owner, identifiers, authorization).await?;
    for sequence in &sequences {
        sqlx::query(
            "DELETE FROM user_message_copies WHERE owner_identifier = ?1 AND mail_sequence = ?2",
        )
        .bind(owner.to_string())
        .bind(sequence)
        .execute(&mut *connection)
        .await
        .storage()?;
        sqlx::query("INSERT INTO user_message_trash(owner_identifier, mail_sequence, trashed_at) VALUES (?1, ?2, ?3) ON CONFLICT(owner_identifier, mail_sequence) DO NOTHING")
            .bind(owner.to_string()).bind(sequence).bind(trashed_at)
            .execute(&mut *connection).await.storage()?;
        sqlx::query(
            "DELETE FROM user_message_placements WHERE owner_identifier = ?1 AND mail_sequence = ?2",
        )
        .bind(owner.to_string())
        .bind(sequence)
        .execute(&mut *connection)
        .await
        .storage()?;
    }
    invalidate_owner_mailboxes(connection, owner, sequences.len()).await
}

/// Tombstone every authorized message in the owner's Web Trash and return new tombstones.
///
/// The tombstone trigger removes the Trash rows and every IMAP membership of the owner.
pub(super) async fn empty_user_trash(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    authorization: &FilterExpression,
) -> Result<u64, StorageError> {
    require_owner(connection, owner).await?;
    let compiled = compile(authorization);
    let sql = format!(
        "INSERT OR IGNORE INTO user_message_deletions(user_identifier, mail_sequence) SELECT ?, mail.sequence FROM mail WHERE ({}) AND (EXISTS (SELECT 1 FROM user_message_trash AS trash WHERE trash.owner_identifier = ? AND trash.mail_sequence = mail.sequence) OR EXISTS(SELECT 1 FROM user_message_copies AS copy WHERE copy.owner_identifier = ? AND copy.mail_sequence = mail.sequence AND copy.target_kind = 'trash'))",
        compiled.sql
    );
    let result = bind_query(
        sqlx::query(sqlx::AssertSqlSafe(sql)).bind(owner.to_string()),
        &compiled.parameters,
    )
    .bind(owner.to_string())
    .bind(owner.to_string())
    .execute(&mut *connection)
    .await
    .storage()?;
    let deleted = result.rows_affected();
    super::organization::collect_unreferenced_messages(connection).await?;
    Ok(deleted)
}

/// Fail with NotFound unless the owner still exists.
pub(super) async fn require_owner(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
) -> Result<(), StorageError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE identifier = ?1)")
            .bind(owner.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    if exists {
        Ok(())
    } else {
        Err(StorageError::NotFound)
    }
}

/// Resolve each distinct requested identifier still authorized and not tombstoned for the owner.
pub(super) async fn authorized_sequences(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    identifiers: &[MessageIdentifier],
    authorization: &FilterExpression,
) -> Result<Vec<i64>, StorageError> {
    let compiled = compile(authorization);
    let sql = format!(
        "SELECT mail.sequence FROM mail WHERE mail.identifier = ? AND ({}) AND NOT EXISTS (SELECT 1 FROM user_message_deletions WHERE user_identifier = ? AND mail_sequence = mail.sequence)",
        compiled.sql
    );
    let mut sequences = Vec::with_capacity(identifiers.len());
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
        if let Some(row) = row {
            sequences.push(row.try_get("sequence").storage()?);
        }
    }
    Ok(sequences)
}

/// Force full IMAP reconciliation for the owner's accounts after `changed` Web organization
/// changes, and return that count.
async fn invalidate_owner_mailboxes(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    changed: usize,
) -> Result<u64, StorageError> {
    if changed > 0 {
        sqlx::query("UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier IN (SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1)")
            .bind(owner.to_string()).execute(connection).await.storage()?;
    }
    u64::try_from(changed).map_err(|_| StorageError::IntegerRange)
}
