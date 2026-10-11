//! IMAP per-account message flags and their modification cursor.

use super::shared::{decode_flag_row, mailbox_state, uid_from};
use crate::error::StorageResult;
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier, ImapMessageFlags};
use sandpost_storage::{ImapFlagChange, ImapFlagChanges, ImapFlagOperation, StorageError};
use sqlx::{Row, SqliteConnection};

/// Load changed flags and the account cursor from one caller-owned read transaction.
pub(super) async fn flag_changes(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    after_cursor: u64,
    through_uid: u32,
) -> Result<ImapFlagChanges, StorageError> {
    let Some(state) = mailbox_state(connection, identifier).await? else {
        return Ok(ImapFlagChanges::default());
    };
    let cursor: i64 =
        sqlx::query_scalar("SELECT flag_modification FROM imap_accounts WHERE identifier = ?1")
            .bind(state.account_identifier.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    let after = i64::try_from(after_cursor).map_err(|_| StorageError::IntegerRange)?;
    let rows = sqlx::query("WITH changed(uid) AS ( \
           SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND modification > ?2 AND uid <= ?3 \
           UNION \
           SELECT member.uid FROM imap_message_flags AS flags \
           JOIN imap_mailbox_messages AS member ON member.mailbox_identifier = ?1 AND member.mail_sequence = flags.mail_sequence \
           WHERE flags.account_identifier = ?4 AND flags.modification > ?2 AND member.uid <= ?3 \
         ) \
         SELECT member.uid AS member_uid, member.deleted AS member_deleted, \
                flags.seen AS shared_seen, flags.answered AS shared_answered, \
                flags.flagged AS shared_flagged, flags.draft AS shared_draft, \
                flags.keywords AS shared_keywords \
         FROM changed JOIN imap_mailbox_messages AS member ON member.mailbox_identifier = ?1 AND member.uid = changed.uid \
         LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = ?4 AND flags.mail_sequence = member.mail_sequence \
         ORDER BY member.uid")
        .bind(identifier.to_string()).bind(after).bind(through_uid).bind(state.account_identifier.to_string())
        .fetch_all(&mut *connection).await.storage()?;
    let changes = rows
        .iter()
        .map(|row| Ok((row.try_get::<i64, _>("member_uid")?, decode_flag_row(row)?)))
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .storage()?;
    Ok(ImapFlagChanges {
        changes: changes
            .into_iter()
            .map(|(uid, flags)| Ok((uid_from(uid)?, flags)))
            .collect::<Result<_, StorageError>>()?,
        cursor: u64::try_from(cursor).map_err(|_| StorageError::IntegerRange)?,
    })
}

/// Apply a flag change to one flag set.
fn apply_change(current: &ImapMessageFlags, change: &ImapFlagChange) -> ImapMessageFlags {
    let requested = &change.flags;
    let mut result = current.clone();
    match change.operation {
        ImapFlagOperation::Replace => result = requested.clone(),
        ImapFlagOperation::Add => {
            result.seen |= requested.seen;
            result.answered |= requested.answered;
            result.flagged |= requested.flagged;
            result.draft |= requested.draft;
            result.deleted |= requested.deleted;
            for keyword in &requested.keywords {
                if !result
                    .keywords
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(keyword))
                {
                    result.keywords.push(keyword.clone());
                }
            }
        }
        ImapFlagOperation::Remove => {
            result.seen &= !requested.seen;
            result.answered &= !requested.answered;
            result.flagged &= !requested.flagged;
            result.draft &= !requested.draft;
            result.deleted &= !requested.deleted;
            result.keywords.retain(|existing| {
                !requested
                    .keywords
                    .iter()
                    .any(|keyword| keyword.eq_ignore_ascii_case(existing))
            });
        }
    }
    result
        .keywords
        .sort_by_key(|keyword| keyword.to_ascii_lowercase());
    result
}

/// Apply a flag change to existing members and return their resulting flags.
pub(super) async fn store_flags(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    uids: &[u32],
    change: &ImapFlagChange,
) -> Result<Vec<(u32, ImapMessageFlags)>, StorageError> {
    let Some(state) = mailbox_state(connection, identifier).await? else {
        return Ok(Vec::new());
    };
    let account = state.account_identifier.to_string();
    let modification = next_modification(connection, state.account_identifier).await?;
    let mut results = Vec::with_capacity(uids.len());
    for uid in uids {
        let Some(row) = sqlx::query("SELECT member.mail_sequence AS mail_sequence, \
             member.deleted AS member_deleted, flags.seen AS shared_seen, \
             flags.answered AS shared_answered, flags.flagged AS shared_flagged, \
             flags.draft AS shared_draft, flags.keywords AS shared_keywords \
             FROM imap_mailbox_messages AS member \
             LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = ?1 AND flags.mail_sequence = member.mail_sequence \
             WHERE member.mailbox_identifier = ?2 AND member.uid = ?3")
            .bind(&account).bind(identifier.to_string()).bind(uid).fetch_optional(&mut *connection).await.storage()? else { continue; };
        let sequence: i64 = row.try_get("mail_sequence").storage()?;
        let current = decode_flag_row(&row).storage()?;
        let updated = apply_change(&current, change);
        let shared_changed = updated.seen != current.seen
            || updated.answered != current.answered
            || updated.flagged != current.flagged
            || updated.draft != current.draft
            || updated.keywords != current.keywords;
        if shared_changed {
            sqlx::query("INSERT INTO imap_message_flags(account_identifier,mail_sequence,seen,answered,flagged,draft,keywords,modification) \
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
                ON CONFLICT(account_identifier, mail_sequence) DO UPDATE SET seen = excluded.seen, answered = excluded.answered, \
                flagged = excluded.flagged, draft = excluded.draft, keywords = excluded.keywords, modification = excluded.modification")
                .bind(&account).bind(sequence).bind(updated.seen).bind(updated.answered).bind(updated.flagged).bind(updated.draft).bind(updated.keywords.join(" ")).bind(modification)
                .execute(&mut *connection).await.storage()?;
        }
        if updated.deleted != current.deleted {
            sqlx::query("UPDATE imap_mailbox_messages SET deleted = ?1, modification = ?2 WHERE mailbox_identifier = ?3 AND uid = ?4")
                .bind(updated.deleted).bind(modification).bind(identifier.to_string()).bind(uid)
                .execute(&mut *connection).await.storage()?;
        }
        results.push((*uid, updated));
    }
    Ok(results)
}

/// Read the current account flag modification cursor.
pub(super) async fn flag_cursor(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<u64, StorageError> {
    let cursor: Option<i64> =
        sqlx::query_scalar("SELECT flag_modification FROM imap_accounts WHERE identifier = ?1")
            .bind(account.to_string())
            .fetch_optional(connection)
            .await
            .storage()?;
    u64::try_from(cursor.unwrap_or(0)).map_err(|_| StorageError::IntegerRange)
}

/// Advance an account's flag cursor and return its new value.
async fn next_modification(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<i64, StorageError> {
    let changed = sqlx::query(
        "UPDATE imap_accounts SET flag_modification = flag_modification + 1 WHERE identifier = ?1 AND flag_modification < ?2",
    )
    .bind(account.to_string())
    .bind(i64::MAX)
    .execute(&mut *connection)
    .await
    .storage()?
    .rows_affected();
    if changed == 0 {
        return Err(StorageError::IntegerRange);
    }
    sqlx::query_scalar("SELECT flag_modification FROM imap_accounts WHERE identifier = ?1")
        .bind(account.to_string())
        .fetch_one(connection)
        .await
        .storage()
}
