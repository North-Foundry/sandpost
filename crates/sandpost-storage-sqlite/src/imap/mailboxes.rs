//! IMAP mailbox identity reconciliation and materialized membership synchronization.

use super::accounts::get_account;
use super::shared::{
    MAILBOX_COLUMNS, MAXIMUM_UID, allocate_uid_validity, decode_mailbox, mailbox_state,
    source_columns,
};
use crate::error::StorageResult;
use crate::filter::{bind_query, compile};
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier, ImapMailboxSource};
use sandpost_storage::{ImapMailboxState, ImapMembershipDefinition, StorageError};
use sqlx::{Row, SqliteConnection};
use std::collections::HashMap;

/// Create missing mailbox identities and delete identities whose source is no longer listed.
///
/// The caller holds one transaction. Returned states preserve the source order supplied by the caller.
pub(super) async fn reconcile_mailboxes(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    sources: &[ImapMailboxSource],
) -> Result<Vec<ImapMailboxState>, StorageError> {
    if get_account(connection, account).await?.is_none() {
        return Err(StorageError::NotFound);
    }
    for source in sources {
        if let ImapMailboxSource::LocalFolder(folder) = source {
            let belongs_to_account: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM imap_local_folders WHERE identifier = ?1 AND account_identifier = ?2)",
            )
            .bind(folder.to_string())
            .bind(account.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
            if !belongs_to_account {
                return Err(StorageError::NotFound);
            }
        }
    }
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {MAILBOX_COLUMNS} FROM imap_mailboxes WHERE account_identifier = ?1"
    )))
    .bind(account.to_string())
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    let existing = rows
        .iter()
        .map(decode_mailbox)
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    let mut by_source: HashMap<ImapMailboxSource, ImapMailboxState> = HashMap::new();
    for state in existing {
        if sources.contains(&state.source) {
            by_source.insert(state.source, state);
        } else {
            sqlx::query("DELETE FROM imap_mailboxes WHERE identifier = ?1")
                .bind(state.identifier.to_string())
                .execute(&mut *connection)
                .await
                .storage()?;
        }
    }
    let mut states = Vec::with_capacity(sources.len());
    for source in sources {
        if let Some(state) = by_source.get(source) {
            states.push(*state);
            continue;
        }
        let identifier = ImapMailboxIdentifier::new();
        let uid_validity = allocate_uid_validity(&mut *connection).await?;
        let (kind, view, folder) = source_columns(*source);
        sqlx::query("INSERT INTO imap_mailboxes(identifier,account_identifier,source,view_identifier,folder_identifier,uid_validity) VALUES (?1,?2,?3,?4,?5,?6)")
            .bind(identifier.to_string())
            .bind(account.to_string())
            .bind(kind)
            .bind(view)
            .bind(folder)
            .bind(uid_validity)
            .execute(&mut *connection)
            .await
            .map_err(|error| match crate::error::sqlite_error(error) {
                StorageError::ConstraintViolation(_) => StorageError::NotFound,
                error => error,
            })?;
        let state = ImapMailboxState {
            identifier,
            account_identifier: account,
            source: *source,
            uid_validity,
            uid_next: 1,
            recent_through: 0,
        };
        by_source.insert(*source, state);
        states.push(state);
    }
    Ok(states)
}

/// Synchronize materialized membership; an unchanged definition with no new mail only reads.
pub(super) async fn synchronize_mailbox(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    definition: &ImapMembershipDefinition,
) -> Result<Option<ImapMailboxState>, StorageError> {
    let Some(state) = mailbox_state(connection, identifier).await? else {
        return Ok(None);
    };
    let stored = sqlx::query("SELECT examined_sequence, definition_fingerprint, uid_next FROM imap_mailboxes WHERE identifier = ?1")
        .bind(identifier.to_string())
        .fetch_one(&mut *connection)
        .await
        .storage()?;
    let examined_sequence: i64 = stored.try_get("examined_sequence").storage()?;
    let stored_fingerprint: Option<String> = stored.try_get("definition_fingerprint").storage()?;
    let uid_next: i64 = stored.try_get("uid_next").storage()?;
    let latest_sequence: i64 = sqlx::query_scalar("SELECT coalesce(max(sequence), 0) FROM mail")
        .fetch_one(&mut *connection)
        .await
        .storage()?;
    let full = stored_fingerprint.as_deref() != Some(definition.fingerprint.as_str());
    if !full && latest_sequence == examined_sequence {
        return Ok(Some(state));
    }
    let compiled = compile(&definition.filter);
    if full {
        let sql = format!(
            "DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND mail_sequence NOT IN (SELECT mail.sequence FROM mail WHERE ({}))",
            compiled.sql
        );
        let query = bind_query(
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(identifier.to_string()),
            &compiled.parameters,
        );
        query.execute(&mut *connection).await.storage()?;
    }
    let mut next_uid = uid_next;
    if !state.source.is_explicit() {
        let start = if full { 0 } else { examined_sequence };
        if latest_sequence > start {
            let sql = format!(
                "INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence) \
                 SELECT ?, ? + ROW_NUMBER() OVER (ORDER BY mail.sequence) - 1, mail.sequence FROM mail \
                 WHERE mail.sequence > ? AND mail.sequence <= ? AND ({}) \
                 AND NOT EXISTS (SELECT 1 FROM imap_mailbox_messages AS member WHERE member.mailbox_identifier = ? AND member.mail_sequence = mail.sequence) \
                 AND NOT EXISTS (SELECT 1 FROM imap_mailbox_exclusions AS exclusion WHERE exclusion.mailbox_identifier = ? AND exclusion.mail_sequence = mail.sequence) \
                 ORDER BY mail.sequence",
                compiled.sql
            );
            let query = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(identifier.to_string())
                .bind(uid_next)
                .bind(start)
                .bind(latest_sequence);
            let inserted = bind_query(query, &compiled.parameters)
                .bind(identifier.to_string())
                .bind(identifier.to_string())
                .execute(&mut *connection)
                .await
                .storage()?
                .rows_affected();
            next_uid += i64::try_from(inserted).map_err(|_| StorageError::IntegerRange)?;
            if next_uid > MAXIMUM_UID + 1 {
                return Err(StorageError::IntegerRange);
            }
        }
    }
    sqlx::query("UPDATE imap_mailboxes SET uid_next = ?1, examined_sequence = ?2, definition_fingerprint = ?3 WHERE identifier = ?4")
        .bind(next_uid)
        .bind(latest_sequence)
        .bind(&definition.fingerprint)
        .bind(identifier.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
    mailbox_state(connection, identifier).await
}

/// Load one mailbox identity by identifier.
pub(super) async fn get_mailbox(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<ImapMailboxState>, StorageError> {
    mailbox_state(connection, identifier).await
}
