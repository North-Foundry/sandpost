//! IMAP member, metadata, content, and recent-watermark reads.

use super::shared::{flags_at, mailbox_state, uid_from};
use crate::error::StorageResult;
use crate::records::parse_identifier;
use rusqlite::{
    Connection, OptionalExtension, TransactionBehavior, params, params_from_iter,
    types::Value as SqlValue,
};
use sandpost_core::{ImapMailboxIdentifier, ImapMessageFlags, MessageIdentifier};
use sandpost_storage::{
    ImapMailboxCounts, ImapMember, ImapMemberContent, ImapMemberMetadata, StorageError,
};

/// UIDs looked up per statement in batched member reads.
const UID_BATCH_SIZE: usize = 256;
/// The member projection joins message identifiers with account and mailbox flags.
const MEMBER_SELECT: &str = "SELECT member.uid, mail.identifier, member.deleted, flags.seen, flags.answered, flags.flagged, flags.draft, flags.keywords \
     FROM imap_mailbox_messages AS member \
     JOIN imap_mailboxes AS box ON box.identifier = member.mailbox_identifier \
     JOIN mail ON mail.sequence = member.mail_sequence \
     LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = box.account_identifier AND flags.mail_sequence = member.mail_sequence";

/// Decode a row of [`MEMBER_SELECT`].
fn decode_member(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, String, ImapMessageFlags)> {
    Ok((row.get(0)?, row.get(1)?, flags_at(row, 2)?))
}

/// Convert decoded member rows into members.
fn members_from_rows(
    rows: Vec<(i64, String, ImapMessageFlags)>,
) -> Result<Vec<ImapMember>, StorageError> {
    rows.into_iter()
        .map(|(uid, identifier, flags)| {
            Ok(ImapMember {
                uid: uid_from(uid)?,
                message_identifier: parse_identifier::<MessageIdentifier>(&identifier)?,
                flags,
            })
        })
        .collect()
}

/// Load members above a UID with their flags.
pub(super) fn members_blocking(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
    after_uid: u32,
) -> Result<Vec<ImapMember>, StorageError> {
    let mut statement = connection
        .prepare(&format!(
            "{MEMBER_SELECT} WHERE member.mailbox_identifier = ?1 AND member.uid > ?2 ORDER BY member.uid"
        ))
        .storage()?;
    let rows = statement
        .query_map(params![identifier.to_string(), after_uid], decode_member)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    members_from_rows(rows)
}

/// Atomically raise the recent watermark to the newest member and return the previous one.
pub(super) fn claim_recent_blocking(
    connection: &mut Connection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<u32>, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let Some(state) = mailbox_state(&transaction, identifier)? else {
        return Ok(None);
    };
    transaction
        .execute(
            "UPDATE imap_mailboxes SET recent_through = max(recent_through, uid_next - 1) WHERE identifier = ?1",
            [identifier.to_string()],
        )
        .storage()?;
    transaction.commit().storage()?;
    Ok(Some(state.recent_through))
}

/// Count members, unseen members, and recent members of a mailbox.
pub(super) fn counts_blocking(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<ImapMailboxCounts>, StorageError> {
    if mailbox_state(connection, identifier)?.is_none() {
        return Ok(None);
    }
    let (messages, unseen, recent): (i64, i64, i64) = connection
        .query_row(
            "SELECT count(*), \
                    coalesce(sum(CASE WHEN coalesce(flags.seen, 0) = 0 THEN 1 ELSE 0 END), 0), \
                    coalesce(sum(CASE WHEN member.uid > box.recent_through THEN 1 ELSE 0 END), 0) \
             FROM imap_mailbox_messages AS member \
             JOIN imap_mailboxes AS box ON box.identifier = member.mailbox_identifier \
             LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = box.account_identifier AND flags.mail_sequence = member.mail_sequence \
             WHERE member.mailbox_identifier = ?1",
            [identifier.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .storage()?;
    Ok(Some(ImapMailboxCounts {
        messages: uid_from(messages)?,
        unseen: uid_from(unseen)?,
        recent: uid_from(recent)?,
    }))
}

/// Load sizes and dates of members in batches.
pub(super) fn metadata_blocking(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
    uids: &[u32],
) -> Result<Vec<ImapMemberMetadata>, StorageError> {
    let mut metadata = Vec::with_capacity(uids.len());
    for chunk in uids.chunks(UID_BATCH_SIZE) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut parameters = vec![SqlValue::Text(identifier.to_string())];
        parameters.extend(chunk.iter().map(|uid| SqlValue::Integer(i64::from(*uid))));
        let mut statement = connection
            .prepare(&format!(
                "SELECT member.uid, length(mail.raw_message), mail.received_at FROM imap_mailbox_messages AS member \
                 JOIN mail ON mail.sequence = member.mail_sequence \
                 WHERE member.mailbox_identifier = ? AND member.uid IN ({placeholders}) ORDER BY member.uid"
            ))
            .storage()?;
        let rows = statement
            .query_map(params_from_iter(parameters), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .storage()?
            .collect::<Result<Vec<_>, _>>()
            .storage()?;
        for (uid, size, received_at) in rows {
            metadata.push(ImapMemberMetadata {
                uid: uid_from(uid)?,
                size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
                received_at,
            });
        }
    }
    Ok(metadata)
}

/// Load the octets of one member.
pub(super) fn content_blocking(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
    uid: u32,
) -> Result<Option<ImapMemberContent>, StorageError> {
    let row: Option<(String, Vec<u8>, i64)> = connection
        .query_row(
            "SELECT mail.identifier, mail.raw_message, mail.received_at FROM imap_mailbox_messages AS member \
             JOIN mail ON mail.sequence = member.mail_sequence \
             WHERE member.mailbox_identifier = ?1 AND member.uid = ?2",
            params![identifier.to_string(), uid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .storage()?;
    row.map(|(message, raw_message, received_at)| {
        Ok(ImapMemberContent {
            uid,
            message_identifier: parse_identifier(&message)?,
            raw_message,
            received_at,
        })
    })
    .transpose()
}

/// Count members at or below the supplied UID watermark.
pub(super) fn count_members_blocking(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
    through_uid: u32,
) -> Result<u64, StorageError> {
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid <= ?2",
            params![identifier.to_string(), through_uid],
            |row| row.get(0),
        )
        .storage()?;
    u64::try_from(count).map_err(|_| StorageError::IntegerRange)
}

/// List member UIDs at or below the supplied watermark in ascending order.
pub(super) fn member_uids_blocking(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
    through_uid: u32,
) -> Result<Vec<u32>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid <= ?2 ORDER BY uid",
        )
        .storage()?;
    let rows = statement
        .query_map(params![identifier.to_string(), through_uid], |row| {
            row.get::<_, i64>(0)
        })
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    rows.into_iter().map(uid_from).collect()
}
