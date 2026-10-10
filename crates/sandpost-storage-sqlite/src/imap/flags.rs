//! IMAP per-account message flags and their modification cursor.

use super::shared::{flags_at, mailbox_state, uid_from};
use crate::error::StorageResult;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier, ImapMessageFlags};
use sandpost_storage::{ImapFlagChange, ImapFlagChanges, ImapFlagOperation, StorageError};

/// Load flags of members whose shared or per-mailbox flags changed after a cursor.
pub(super) fn flag_changes_blocking(
    connection: &mut Connection,
    identifier: ImapMailboxIdentifier,
    after_cursor: u64,
    through_uid: u32,
) -> Result<ImapFlagChanges, StorageError> {
    let transaction = connection.transaction().storage()?;
    let Some(state) = mailbox_state(&transaction, identifier)? else {
        return Ok(ImapFlagChanges::default());
    };
    let cursor: i64 = transaction
        .query_row(
            "SELECT flag_modification FROM imap_accounts WHERE identifier = ?1",
            [state.account_identifier.to_string()],
            |row| row.get(0),
        )
        .storage()?;
    let after = i64::try_from(after_cursor).map_err(|_| StorageError::IntegerRange)?;
    let rows = {
        let mut statement = transaction
            .prepare(
                "WITH changed(uid) AS ( \
                   SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND modification > ?2 AND uid <= ?3 \
                   UNION \
                   SELECT member.uid FROM imap_message_flags AS flags \
                   JOIN imap_mailbox_messages AS member ON member.mailbox_identifier = ?1 AND member.mail_sequence = flags.mail_sequence \
                   WHERE flags.account_identifier = ?4 AND flags.modification > ?2 AND member.uid <= ?3 \
                 ) \
                 SELECT member.uid, member.deleted, flags.seen, flags.answered, flags.flagged, flags.draft, flags.keywords \
                 FROM changed JOIN imap_mailbox_messages AS member ON member.mailbox_identifier = ?1 AND member.uid = changed.uid \
                 LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = ?4 AND flags.mail_sequence = member.mail_sequence \
                 ORDER BY member.uid",
            )
            .storage()?;
        statement
            .query_map(
                params![
                    identifier.to_string(),
                    after,
                    through_uid,
                    state.account_identifier.to_string()
                ],
                |row| Ok((row.get::<_, i64>(0)?, flags_at(row, 1)?)),
            )
            .storage()?
            .collect::<Result<Vec<_>, _>>()
            .storage()?
    };
    transaction.commit().storage()?;
    Ok(ImapFlagChanges {
        changes: rows
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
        ImapFlagOperation::Replace => {
            result = requested.clone();
        }
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
pub(super) fn store_flags_blocking(
    connection: &mut Connection,
    identifier: ImapMailboxIdentifier,
    uids: &[u32],
    change: &ImapFlagChange,
) -> Result<Vec<(u32, ImapMessageFlags)>, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let Some(state) = mailbox_state(&transaction, identifier)? else {
        return Ok(Vec::new());
    };
    let account = state.account_identifier.to_string();
    let modification = next_modification(&transaction, state.account_identifier)?;
    let mut results = Vec::with_capacity(uids.len());
    {
        let mut select = transaction
            .prepare_cached(
                "SELECT member.mail_sequence, member.deleted, flags.seen, flags.answered, flags.flagged, flags.draft, flags.keywords \
                 FROM imap_mailbox_messages AS member \
                 LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = ?1 AND flags.mail_sequence = member.mail_sequence \
                 WHERE member.mailbox_identifier = ?2 AND member.uid = ?3",
            )
            .storage()?;
        let mut shared = transaction
            .prepare_cached(
                "INSERT INTO imap_message_flags(account_identifier,mail_sequence,seen,answered,flagged,draft,keywords,modification) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
                 ON CONFLICT(account_identifier, mail_sequence) DO UPDATE SET seen = excluded.seen, answered = excluded.answered, \
                 flagged = excluded.flagged, draft = excluded.draft, keywords = excluded.keywords, modification = excluded.modification",
            )
            .storage()?;
        let mut deleted_update = transaction
            .prepare_cached(
                "UPDATE imap_mailbox_messages SET deleted = ?1, modification = ?2 WHERE mailbox_identifier = ?3 AND uid = ?4",
            )
            .storage()?;
        for uid in uids {
            let Some((sequence, current)) = select
                .query_row(params![account, identifier.to_string(), uid], |row| {
                    Ok((row.get::<_, i64>(0)?, flags_at(row, 1)?))
                })
                .optional()
                .storage()?
            else {
                continue;
            };
            let updated = apply_change(&current, change);
            let shared_changed = updated.seen != current.seen
                || updated.answered != current.answered
                || updated.flagged != current.flagged
                || updated.draft != current.draft
                || updated.keywords != current.keywords;
            if shared_changed {
                shared
                    .execute(params![
                        account,
                        sequence,
                        updated.seen,
                        updated.answered,
                        updated.flagged,
                        updated.draft,
                        updated.keywords.join(" "),
                        modification
                    ])
                    .storage()?;
            }
            if updated.deleted != current.deleted {
                deleted_update
                    .execute(params![
                        updated.deleted,
                        modification,
                        identifier.to_string(),
                        uid
                    ])
                    .storage()?;
            }
            results.push((*uid, updated));
        }
    }
    transaction.commit().storage()?;
    Ok(results)
}

/// Read the current account flag modification cursor.
pub(super) fn flag_cursor_blocking(
    connection: &Connection,
    account: ImapAccountIdentifier,
) -> Result<u64, StorageError> {
    let cursor: Option<i64> = connection
        .query_row(
            "SELECT flag_modification FROM imap_accounts WHERE identifier = ?1",
            [account.to_string()],
            |row| row.get(0),
        )
        .optional()
        .storage()?;
    u64::try_from(cursor.unwrap_or(0)).map_err(|_| StorageError::IntegerRange)
}

/// Advance an account's flag cursor and return its new value.
fn next_modification(
    transaction: &Transaction<'_>,
    account: ImapAccountIdentifier,
) -> Result<i64, StorageError> {
    transaction
        .execute(
            "UPDATE imap_accounts SET flag_modification = flag_modification + 1 WHERE identifier = ?1",
            [account.to_string()],
        )
        .storage()?;
    transaction
        .query_row(
            "SELECT flag_modification FROM imap_accounts WHERE identifier = ?1",
            [account.to_string()],
            |row| row.get(0),
        )
        .storage()
}
