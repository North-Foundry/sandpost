//! IMAP membership mutations: copy, expunge, and exclusion management.

use super::shared::{MAXIMUM_UID, mailbox_state, uid_from};
use crate::error::StorageResult;
use crate::filter::{bind_query, compile};
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier};
use sandpost_storage::{ImapCopyResult, ImapMembershipDefinition, StorageError};
use sqlx::{Row, SqliteConnection};
use std::collections::{HashMap, HashSet};

/// Keep UID filters below SQLite's supported bound-parameter limit.
const UID_BATCH_SIZE: usize = 500;

/// Tombstone flagged messages for the mailbox owner's user across all their IMAP accounts.
pub(super) async fn expunge(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    uids: Option<&[u32]>,
) -> Result<Vec<u32>, StorageError> {
    let Some(state) = mailbox_state(connection, identifier).await? else {
        return Ok(Vec::new());
    };
    let owner: String =
        sqlx::query_scalar("SELECT owner_identifier FROM imap_accounts WHERE identifier = ?1")
            .bind(state.account_identifier.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    let mailbox_id = identifier.to_string();
    let mut candidates = Vec::new();
    match uids {
        Some(uids) => {
            for batch in uids.chunks(UID_BATCH_SIZE) {
                if batch.is_empty() {
                    continue;
                }
                let placeholders = vec!["?"; batch.len()].join(",");
                let sql = format!(
                    "SELECT uid AS member_uid FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND deleted = 1 AND uid IN ({placeholders}) ORDER BY uid"
                );
                let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&mailbox_id);
                for uid in batch {
                    query = query.bind(uid);
                }
                let rows = query.fetch_all(&mut *connection).await.storage()?;
                candidates.extend(
                    rows.iter()
                        .map(|row| row.try_get::<i64, _>("member_uid"))
                        .collect::<Result<Vec<_>, _>>()
                        .storage()?,
                );
            }
        }
        None => {
            let rows = sqlx::query("SELECT uid AS member_uid FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND deleted = 1 ORDER BY uid")
                .bind(&mailbox_id)
                .fetch_all(&mut *connection)
                .await
                .storage()?;
            candidates = rows
                .iter()
                .map(|row| row.try_get::<i64, _>("member_uid"))
                .collect::<Result<Vec<_>, _>>()
                .storage()?;
        }
    }
    let mut removed = candidates
        .iter()
        .map(|uid| uid_from(*uid))
        .collect::<Result<Vec<_>, _>>()?;
    removed.sort_unstable();
    removed.dedup();
    if removed.is_empty() {
        return Ok(removed);
    }
    match uids {
        Some(uids) => {
            for batch in uids.chunks(UID_BATCH_SIZE) {
                if batch.is_empty() {
                    continue;
                }
                let placeholders = vec!["?"; batch.len()].join(",");
                let sql = format!(
                    "INSERT OR IGNORE INTO user_message_deletions(user_identifier, mail_sequence) SELECT ?, mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND deleted = 1 AND uid IN ({placeholders})"
                );
                let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(&owner)
                    .bind(&mailbox_id);
                for uid in batch {
                    query = query.bind(uid);
                }
                query.execute(&mut *connection).await.storage()?;
            }
        }
        None => {
            sqlx::query("INSERT OR IGNORE INTO user_message_deletions(user_identifier, mail_sequence) SELECT ?, mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND deleted = 1")
                .bind(&owner)
                .bind(&mailbox_id)
                .execute(&mut *connection)
                .await
                .storage()?;
        }
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
    if !members.is_empty() && destination_definition.is_none() {
        return Err(StorageError::ConstraintViolation(
            "a destination requires its authorization definition".into(),
        ));
    }
    if matches!(
        destination_state.source,
        sandpost_core::ImapMailboxSource::View(_)
    ) {
        return Err(StorageError::ConstraintViolation(
            "messages cannot be inserted into a smart View".into(),
        ));
    }
    if let sandpost_core::ImapMailboxSource::LocalFolder(folder) = destination_state.source {
        let kind: Option<String> = sqlx::query_scalar(
            "SELECT kind FROM imap_local_folders WHERE identifier = ?1 AND account_identifier = ?2",
        )
        .bind(folder.to_string())
        .bind(destination_state.account_identifier.to_string())
        .fetch_optional(&mut *connection)
        .await
        .storage()?;
        if kind.as_deref() != Some("manual") && kind.as_deref() != Some("private") {
            return Err(StorageError::ConstraintViolation(
                "messages cannot be inserted into a smart folder".into(),
            ));
        }
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
    let (source_kind, source_folder) =
        placement_target(source_state.source).unwrap_or(("smart", None));
    for (uid, sequence) in &members {
        let mut copied_occurrence: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM imap_message_copies WHERE account_identifier = ?1 AND mail_sequence = ?2 AND target_kind = ?3 AND folder_identifier IS ?4) AND NOT EXISTS(SELECT 1 FROM imap_message_placements WHERE account_identifier = ?1 AND mail_sequence = ?2 AND target_kind = ?3 AND folder_identifier IS ?4)")
            .bind(source_state.account_identifier.to_string()).bind(sequence)
            .bind(source_kind).bind(source_folder.as_deref())
            .fetch_one(&mut *connection).await.storage()?;
        if remove_from_source && copied_occurrence {
            copied_occurrence = !is_baseline_occurrence(
                connection,
                source_state.account_identifier,
                *sequence,
                source_kind,
                source_folder.as_deref(),
            )
            .await?;
        }
        sqlx::query("DELETE FROM imap_mailbox_exclusions WHERE mailbox_identifier = ?1 AND mail_sequence = ?2")
            .bind(destination.to_string()).bind(sequence).execute(&mut *connection).await.storage()?;
        let existing: Option<i64> = sqlx::query_scalar("SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND mail_sequence = ?2")
            .bind(destination.to_string()).bind(sequence).fetch_optional(&mut *connection).await.storage()?;
        let destination_uid = match existing {
            Some(_) => {
                return Err(StorageError::ConstraintViolation(
                    "destination already contains a message".into(),
                ));
            }
            None => {
                if next_uid > MAXIMUM_UID {
                    return Err(StorageError::IntegerRange);
                }
                sqlx::query("INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence, deleted) SELECT ?1, ?2, mail_sequence, deleted FROM imap_mailbox_messages WHERE mailbox_identifier = ?3 AND uid = ?4")
                    .bind(destination.to_string()).bind(next_uid).bind(source.to_string()).bind(uid)
                    .execute(&mut *connection).await.storage()?;
                next_uid += 1;
                next_uid - 1
            }
        };
        result.source_uids.push(*uid);
        result.destination_uids.push(uid_from(destination_uid)?);
        let (target_kind, folder_identifier) = placement_target(destination_state.source)?;
        if remove_from_source && !copied_occurrence {
            sqlx::query("INSERT INTO imap_message_placements(account_identifier, mail_sequence, target_kind, folder_identifier) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(account_identifier, mail_sequence) DO UPDATE SET target_kind = excluded.target_kind, folder_identifier = excluded.folder_identifier")
                .bind(destination_state.account_identifier.to_string()).bind(sequence)
                .bind(target_kind).bind(folder_identifier)
                .execute(&mut *connection).await.storage()?;
        } else {
            sqlx::query("INSERT OR IGNORE INTO imap_message_copies(account_identifier, mail_sequence, target_kind, folder_identifier) VALUES (?1, ?2, ?3, ?4)")
                .bind(destination_state.account_identifier.to_string()).bind(sequence)
                .bind(target_kind).bind(folder_identifier)
                .execute(&mut *connection).await.storage()?;
        }
    }
    sqlx::query("UPDATE imap_mailboxes SET uid_next = ?1 WHERE identifier = ?2")
        .bind(next_uid)
        .bind(destination.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
    if remove_from_source && source != destination {
        for (uid, sequence) in &members {
            // A MOVE removes only the source occurrence, preserving unrelated explicit copies.
            sqlx::query("DELETE FROM imap_message_copies WHERE account_identifier = ?1 AND mail_sequence = ?2 AND target_kind = ?3 AND folder_identifier IS ?4")
                .bind(source_state.account_identifier.to_string()).bind(sequence)
                .bind(source_kind).bind(source_folder.as_deref())
                .execute(&mut *connection).await.storage()?;
            sqlx::query(
                "DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2",
            )
            .bind(source.to_string())
            .bind(uid)
            .execute(&mut *connection)
            .await
            .storage()?;
            result.removed_uids.push(*uid);
        }
    }
    if remove_from_source {
        sqlx::query(
            "UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier = ?1",
        )
        .bind(source_state.account_identifier.to_string())
        .execute(connection)
        .await
        .storage()?;
    }
    Ok(result)
}

/// Identify an inherited or general baseline occurrence without an exclusive account override.
async fn is_baseline_occurrence(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    sequence: i64,
    target_kind: &str,
    folder: Option<&str>,
) -> Result<bool, StorageError> {
    sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM imap_message_placements WHERE account_identifier = ?1 AND mail_sequence = ?2) AND ((?3 = 'inbox' AND NOT EXISTS(SELECT 1 FROM user_message_placements AS placement JOIN web_folder_accounts AS exposure ON exposure.folder_identifier = placement.folder_identifier JOIN imap_accounts AS account ON account.identifier = exposure.account_identifier AND account.owner_identifier = placement.owner_identifier WHERE exposure.account_identifier = ?1 AND placement.mail_sequence = ?2)) OR (?3 = 'folder' AND EXISTS(SELECT 1 FROM user_message_placements AS placement JOIN web_folder_accounts AS exposure ON exposure.folder_identifier = placement.folder_identifier JOIN imap_accounts AS account ON account.identifier = exposure.account_identifier AND account.owner_identifier = placement.owner_identifier WHERE exposure.account_identifier = ?1 AND placement.mail_sequence = ?2 AND exposure.local_folder_identifier IS ?4)))")
        .bind(account.to_string()).bind(sequence).bind(target_kind).bind(folder)
        .fetch_one(connection).await.storage()
}

/// Convert a writable mailbox source into a durable organization target.
fn placement_target(
    source: sandpost_core::ImapMailboxSource,
) -> Result<(&'static str, Option<String>), StorageError> {
    match source {
        sandpost_core::ImapMailboxSource::Inbox => Ok(("inbox", None)),
        sandpost_core::ImapMailboxSource::Trash => Ok(("trash", None)),
        sandpost_core::ImapMailboxSource::LocalFolder(identifier) => {
            Ok(("folder", Some(identifier.to_string())))
        }
        sandpost_core::ImapMailboxSource::View(_) => Err(StorageError::ConstraintViolation(
            "smart Views are not placement destinations".into(),
        )),
    }
}

/// Clear legacy account exclusions and reconcile, preserving placements and user deletion tombstones.
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

/// Load each non-deleted owner's message sequence for the supplied member UIDs.
pub(super) async fn member_sequences(
    connection: &mut SqliteConnection,
    mailbox: ImapMailboxIdentifier,
    uids: &[u32],
) -> Result<Vec<(u32, i64)>, StorageError> {
    let state = mailbox_state(connection, mailbox)
        .await?
        .ok_or(StorageError::NotFound)?;
    let owner: String =
        sqlx::query_scalar("SELECT owner_identifier FROM imap_accounts WHERE identifier = ?1")
            .bind(state.account_identifier.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    let mut found = HashMap::with_capacity(uids.len());
    for batch in uids.chunks(UID_BATCH_SIZE) {
        if batch.is_empty() {
            continue;
        }
        let placeholders = vec!["?"; batch.len()].join(",");
        let sql = format!(
            "SELECT member.uid, member.mail_sequence FROM imap_mailbox_messages AS member WHERE member.mailbox_identifier = ? AND member.uid IN ({placeholders}) AND NOT EXISTS (SELECT 1 FROM user_message_deletions AS deletion WHERE deletion.user_identifier = ? AND deletion.mail_sequence = member.mail_sequence)"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(mailbox.to_string());
        for uid in batch {
            query = query.bind(uid);
        }
        let rows = query
            .bind(&owner)
            .fetch_all(&mut *connection)
            .await
            .storage()?;
        for row in &rows {
            found.insert(
                uid_from(row.try_get("uid").storage()?)?,
                row.try_get("mail_sequence").storage()?,
            );
        }
    }
    let mut unique = HashSet::new();
    Ok(uids
        .iter()
        .filter(|uid| unique.insert(**uid))
        .filter_map(|uid| found.get(uid).map(|sequence| (*uid, *sequence)))
        .collect())
}
