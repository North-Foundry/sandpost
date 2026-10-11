//! IMAP membership mutations: copy, expunge, and exclusion management.

use super::shared::{MAXIMUM_UID, mailbox_state, uid_from};
use crate::error::StorageResult;
use crate::filter::{bind_query, compile};
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier};
use sandpost_storage::{ImapCopyResult, ImapMembershipDefinition, StorageError};
use sqlx::{Row, SqliteConnection};

/// Remove deleted members and remember expunges from dynamic mailboxes.
pub(super) async fn expunge(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    uids: Option<&[u32]>,
) -> Result<Vec<u32>, StorageError> {
    let Some(state) = mailbox_state(connection, identifier).await? else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query("SELECT uid AS member_uid, mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND deleted = 1 ORDER BY uid")
        .bind(identifier.to_string())
        .fetch_all(&mut *connection)
        .await
        .storage()?;
    let candidates = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<i64, _>("member_uid")?,
                row.try_get::<i64, _>("mail_sequence")?,
            ))
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .storage()?;
    let mut removed = Vec::new();
    for (stored_uid, sequence) in candidates {
        let uid = uid_from(stored_uid)?;
        if uids.is_some_and(|uids| !uids.contains(&uid)) {
            continue;
        }
        sqlx::query("DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2")
            .bind(identifier.to_string())
            .bind(uid)
            .execute(&mut *connection)
            .await
            .storage()?;
        if !state.source.is_explicit() {
            sqlx::query("INSERT OR IGNORE INTO imap_mailbox_exclusions(mailbox_identifier, mail_sequence) VALUES (?1, ?2)")
                .bind(identifier.to_string()).bind(sequence)
                .execute(&mut *connection).await.storage()?;
        }
        removed.push(uid);
    }
    Ok(removed)
}

/// Copy or move members between two mailboxes of one account.
pub(super) async fn copy(
    connection: &mut SqliteConnection,
    source: ImapMailboxIdentifier,
    uids: &[u32],
    destination: ImapMailboxIdentifier,
    destination_definition: Option<&ImapMembershipDefinition>,
    remove_from_source: bool,
) -> Result<ImapCopyResult, StorageError> {
    let source_state = mailbox_state(connection, source)
        .await?
        .ok_or(StorageError::NotFound)?;
    let destination_state = mailbox_state(connection, destination)
        .await?
        .ok_or(StorageError::NotFound)?;
    if source_state.account_identifier != destination_state.account_identifier {
        return Err(StorageError::NotFound);
    }
    let members = member_sequences(connection, source, uids).await?;
    if !members.is_empty()
        && !destination_state.source.is_explicit()
        && destination_definition.is_none()
    {
        return Err(StorageError::ConstraintViolation(
            "a dynamic destination requires its membership definition".into(),
        ));
    }
    if let Some(definition) = destination_definition {
        let compiled = compile(&definition.filter);
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM mail WHERE mail.sequence = ? AND ({})) AS matches_definition",
            compiled.sql
        );
        for (_, sequence) in &members {
            let row = bind_query(
                sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(*sequence),
                &compiled.parameters,
            )
            .fetch_one(&mut *connection)
            .await
            .storage()?;
            let matches: bool = row.try_get("matches_definition").storage()?;
            if !matches {
                return Err(StorageError::ConstraintViolation(
                    "a message does not match the destination mailbox".into(),
                ));
            }
        }
    }
    let mut next_uid: i64 =
        sqlx::query_scalar("SELECT uid_next FROM imap_mailboxes WHERE identifier = ?1")
            .bind(destination.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    let mut result = ImapCopyResult {
        uid_validity: destination_state.uid_validity,
        all_new: true,
        ..ImapCopyResult::default()
    };
    for (uid, sequence) in &members {
        sqlx::query("DELETE FROM imap_mailbox_exclusions WHERE mailbox_identifier = ?1 AND mail_sequence = ?2")
            .bind(destination.to_string()).bind(sequence).execute(&mut *connection).await.storage()?;
        let existing: Option<i64> = sqlx::query_scalar("SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND mail_sequence = ?2")
            .bind(destination.to_string()).bind(sequence).fetch_optional(&mut *connection).await.storage()?;
        let destination_uid = match existing {
            Some(existing) => {
                result.all_new = false;
                existing
            }
            None => {
                if next_uid > MAXIMUM_UID {
                    return Err(StorageError::IntegerRange);
                }
                sqlx::query("INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence) VALUES (?1, ?2, ?3)")
                    .bind(destination.to_string()).bind(next_uid).bind(sequence)
                    .execute(&mut *connection).await.storage()?;
                next_uid += 1;
                next_uid - 1
            }
        };
        result.source_uids.push(*uid);
        result.destination_uids.push(uid_from(destination_uid)?);
    }
    sqlx::query("UPDATE imap_mailboxes SET uid_next = ?1 WHERE identifier = ?2")
        .bind(next_uid)
        .bind(destination.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
    if remove_from_source && source != destination {
        for (uid, sequence) in &members {
            sqlx::query(
                "DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2",
            )
            .bind(source.to_string())
            .bind(uid)
            .execute(&mut *connection)
            .await
            .storage()?;
            if !source_state.source.is_explicit() {
                sqlx::query("INSERT OR IGNORE INTO imap_mailbox_exclusions(mailbox_identifier, mail_sequence) VALUES (?1, ?2)")
                    .bind(source.to_string()).bind(sequence).execute(&mut *connection).await.storage()?;
            }
            result.removed_uids.push(*uid);
        }
    }
    Ok(result)
}

/// Forget an account's expunges and force full reconciliation of its mailboxes.
pub(super) async fn clear_exclusions(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<u64, StorageError> {
    let cleared = sqlx::query("DELETE FROM imap_mailbox_exclusions WHERE mailbox_identifier IN (SELECT identifier FROM imap_mailboxes WHERE account_identifier = ?1)")
        .bind(account.to_string()).execute(&mut *connection).await.storage()?.rows_affected();
    sqlx::query(
        "UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier = ?1",
    )
    .bind(account.to_string())
    .execute(connection)
    .await
    .storage()?;
    Ok(cleared)
}

/// Load the message sequence of each existing member UID in the supplied UID order.
pub(super) async fn member_sequences(
    connection: &mut SqliteConnection,
    mailbox: ImapMailboxIdentifier,
    uids: &[u32],
) -> Result<Vec<(u32, i64)>, StorageError> {
    let mut members = Vec::with_capacity(uids.len());
    for uid in uids {
        if let Some(sequence) = sqlx::query_scalar("SELECT mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2")
            .bind(mailbox.to_string()).bind(uid).fetch_optional(&mut *connection).await.storage()? {
            members.push((*uid, sequence));
        }
    }
    Ok(members)
}
