//! IMAP member, metadata, content, and recent-watermark reads.

use super::shared::{decode_flag_row, mailbox_state, uid_from};
use crate::error::StorageResult;
use crate::records::parse_identifier;
use sandpost_core::{ImapMailboxIdentifier, ImapMessageFlags, MessageIdentifier};
use sandpost_storage::{
    ImapMailboxCounts, ImapMember, ImapMemberContent, ImapMemberMetadata, StorageError,
};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

/// Maximum UIDs selected by one metadata query.
const UID_BATCH_SIZE: usize = 256;
/// Member projection with current account flags.
const MEMBER_SELECT: &str = "SELECT member.uid AS member_uid, mail.identifier AS message_identifier, \
     member.deleted AS member_deleted, flags.seen AS shared_seen, flags.answered AS shared_answered, \
     flags.flagged AS shared_flagged, flags.draft AS shared_draft, flags.keywords AS shared_keywords \
     FROM imap_mailbox_messages AS member \
     JOIN imap_mailboxes AS box ON box.identifier = member.mailbox_identifier \
     JOIN mail ON mail.sequence = member.mail_sequence \
     LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = box.account_identifier AND flags.mail_sequence = member.mail_sequence";

/// Decode one row from [`MEMBER_SELECT`].
fn decode_member(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<(i64, String, ImapMessageFlags), sqlx::Error> {
    Ok((
        row.try_get("member_uid")?,
        row.try_get("message_identifier")?,
        decode_flag_row(row)?,
    ))
}

/// Convert decoded member rows into contract members.
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
pub(super) async fn members(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    after_uid: u32,
) -> Result<Vec<ImapMember>, StorageError> {
    let sql = format!(
        "{MEMBER_SELECT} WHERE member.mailbox_identifier = ?1 AND member.uid > ?2 ORDER BY member.uid"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(identifier.to_string())
        .bind(after_uid)
        .fetch_all(connection)
        .await
        .storage()?;
    members_from_rows(
        rows.iter()
            .map(decode_member)
            .collect::<Result<Vec<_>, _>>()
            .storage()?,
    )
}

/// Atomically raise the recent watermark and return its previous value.
pub(super) async fn claim_recent(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<u32>, StorageError> {
    let Some(state) = mailbox_state(connection, identifier).await? else {
        return Ok(None);
    };
    sqlx::query("UPDATE imap_mailboxes SET recent_through = max(recent_through, uid_next - 1) WHERE identifier = ?1")
        .bind(identifier.to_string()).execute(connection).await.storage()?;
    Ok(Some(state.recent_through))
}

/// Count members, unseen members, and recent members of a mailbox.
pub(super) async fn counts(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<ImapMailboxCounts>, StorageError> {
    if mailbox_state(connection, identifier).await?.is_none() {
        return Ok(None);
    }
    let row = sqlx::query("SELECT count(*) AS message_count, \
             coalesce(sum(CASE WHEN coalesce(flags.seen, 0) = 0 THEN 1 ELSE 0 END), 0) AS unseen_count, \
             coalesce(sum(CASE WHEN member.uid > box.recent_through THEN 1 ELSE 0 END), 0) AS recent_count \
             FROM imap_mailbox_messages AS member \
             JOIN imap_mailboxes AS box ON box.identifier = member.mailbox_identifier \
             LEFT JOIN imap_message_flags AS flags ON flags.account_identifier = box.account_identifier AND flags.mail_sequence = member.mail_sequence \
             WHERE member.mailbox_identifier = ?1")
        .bind(identifier.to_string()).fetch_one(connection).await.storage()?;
    Ok(Some(ImapMailboxCounts {
        messages: uid_from(row.try_get("message_count").storage()?)?,
        unseen: uid_from(row.try_get("unseen_count").storage()?)?,
        recent: uid_from(row.try_get("recent_count").storage()?)?,
    }))
}

/// Load sizes and dates of members in bounded batches.
pub(super) async fn metadata(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    uids: &[u32],
) -> Result<Vec<ImapMemberMetadata>, StorageError> {
    let mut metadata = Vec::with_capacity(uids.len());
    for chunk in uids.chunks(UID_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT member.uid AS member_uid, length(mail.raw_message) AS message_size, mail.received_at FROM imap_mailbox_messages AS member \
            JOIN mail ON mail.sequence = member.mail_sequence WHERE member.mailbox_identifier = ",
        );
        query
            .push_bind(identifier.to_string())
            .push(" AND member.uid IN (");
        let mut separated = query.separated(",");
        for uid in chunk {
            separated.push_bind(i64::from(*uid));
        }
        separated.push_unseparated(") ORDER BY member.uid");
        let rows = query.build().fetch_all(&mut *connection).await.storage()?;
        for row in rows {
            let uid: i64 = row.try_get("member_uid").storage()?;
            let size: i64 = row.try_get("message_size").storage()?;
            let received_at: i64 = row.try_get("received_at").storage()?;
            metadata.push(ImapMemberMetadata {
                uid: uid_from(uid)?,
                size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
                received_at,
            });
        }
    }
    Ok(metadata)
}

/// Load the stored octets of one member.
pub(super) async fn content(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    uid: u32,
) -> Result<Option<ImapMemberContent>, StorageError> {
    let row = sqlx::query("SELECT mail.identifier AS message_identifier, mail.raw_message, mail.received_at FROM imap_mailbox_messages AS member \
         JOIN mail ON mail.sequence = member.mail_sequence WHERE member.mailbox_identifier = ?1 AND member.uid = ?2")
        .bind(identifier.to_string()).bind(uid).fetch_optional(connection).await.storage()?;
    row.as_ref()
        .map(|row| {
            Ok(ImapMemberContent {
                uid,
                message_identifier: parse_identifier(
                    &row.try_get::<String, _>("message_identifier")
                        .map_err(crate::error::sqlite_error)?,
                )?,
                raw_message: row
                    .try_get("raw_message")
                    .map_err(crate::error::sqlite_error)?,
                received_at: row
                    .try_get("received_at")
                    .map_err(crate::error::sqlite_error)?,
            })
        })
        .transpose()
}

/// Count members at or below the supplied UID watermark.
pub(super) async fn count_members(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    through_uid: u32,
) -> Result<u64, StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid <= ?2",
    )
    .bind(identifier.to_string())
    .bind(through_uid)
    .fetch_one(connection)
    .await
    .storage()?;
    u64::try_from(count).map_err(|_| StorageError::IntegerRange)
}

/// List member UIDs at or below a watermark in ascending order.
pub(super) async fn member_uids(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    through_uid: u32,
) -> Result<Vec<u32>, StorageError> {
    let values: Vec<i64> = sqlx::query_scalar("SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid <= ?2 ORDER BY uid")
        .bind(identifier.to_string()).bind(through_uid).fetch_all(connection).await.storage()?;
    values.into_iter().map(uid_from).collect()
}
