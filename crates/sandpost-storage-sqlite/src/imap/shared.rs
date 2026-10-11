//! Shared IMAP row codecs and UID helpers.

use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use sandpost_core::{ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags};
use sandpost_storage::{ImapMailboxState, StorageError};
use sqlx::{Row, SqliteConnection};

/// Named mailbox state columns shared by mailbox queries.
pub(super) const MAILBOX_COLUMNS: &str = "identifier,account_identifier,source,view_identifier,folder_identifier,uid_validity,uid_next,recent_through";
/// Largest UID; `uid_next` beyond it means the mailbox has no UIDs left.
pub(super) const MAXIMUM_UID: i64 = u32::MAX as i64;

/// Return the current Unix timestamp in seconds, or zero if the system clock predates the epoch.
pub(super) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Decode a mailbox state row.
pub(super) fn decode_mailbox(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ImapMailboxState, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let account: String = row.try_get("account_identifier")?;
    let source: String = row.try_get("source")?;
    let view: Option<String> = row.try_get("view_identifier")?;
    let folder: Option<String> = row.try_get("folder_identifier")?;
    let source = match (source.as_str(), view, folder) {
        ("inbox", None, None) => ImapMailboxSource::Inbox,
        ("trash", None, None) => ImapMailboxSource::Trash,
        ("view", Some(view), None) => ImapMailboxSource::View(
            parse_identifier(&view).map_err(|_| invalid_column("view_identifier"))?,
        ),
        ("folder", None, Some(folder)) => ImapMailboxSource::LocalFolder(
            parse_identifier(&folder).map_err(|_| invalid_column("folder_identifier"))?,
        ),
        _ => return Err(invalid_column("source")),
    };
    let uid_validity: i64 = row.try_get("uid_validity")?;
    let uid_next: i64 = row.try_get("uid_next")?;
    let recent_through: i64 = row.try_get("recent_through")?;
    Ok(ImapMailboxState {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        account_identifier: parse_identifier(&account)
            .map_err(|_| invalid_column("account_identifier"))?,
        source,
        uid_validity: u32::try_from(uid_validity).map_err(|_| invalid_column("uid_validity"))?,
        // An exhausted mailbox stores 2^32; report the largest representable UID instead.
        uid_next: u32::try_from(uid_next.min(MAXIMUM_UID))
            .map_err(|_| invalid_column("uid_next"))?,
        recent_through: u32::try_from(recent_through)
            .map_err(|_| invalid_column("recent_through"))?,
    })
}

/// Return the stored source kind and reference columns for a mailbox source.
pub(super) fn source_columns(
    source: ImapMailboxSource,
) -> (&'static str, Option<String>, Option<String>) {
    match source {
        ImapMailboxSource::Inbox => ("inbox", None, None),
        ImapMailboxSource::Trash => ("trash", None, None),
        ImapMailboxSource::View(view) => ("view", Some(view.to_string()), None),
        ImapMailboxSource::LocalFolder(folder) => ("folder", None, Some(folder.to_string())),
    }
}

/// Decode stored flag columns: per-mailbox deleted, then shared flags (NULL when no row).
pub(super) fn decode_flags(
    deleted: bool,
    seen: Option<bool>,
    answered: Option<bool>,
    flagged: Option<bool>,
    draft: Option<bool>,
    keywords: Option<String>,
) -> ImapMessageFlags {
    ImapMessageFlags {
        seen: seen.unwrap_or(false),
        answered: answered.unwrap_or(false),
        flagged: flagged.unwrap_or(false),
        draft: draft.unwrap_or(false),
        deleted,
        keywords: keywords
            .unwrap_or_default()
            .split(' ')
            .filter(|keyword| !keyword.is_empty())
            .map(str::to_owned)
            .collect(),
    }
}

/// Decode deleted and shared flag columns by their explicit projection names.
pub(super) fn decode_flag_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ImapMessageFlags, sqlx::Error> {
    Ok(decode_flags(
        row.try_get("member_deleted")?,
        row.try_get("shared_seen")?,
        row.try_get("shared_answered")?,
        row.try_get("shared_flagged")?,
        row.try_get("shared_draft")?,
        row.try_get("shared_keywords")?,
    ))
}

/// Convert a stored non-negative integer to a UID.
pub(super) fn uid_from(value: i64) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::IntegerRange)
}

/// Load one mailbox state inside a connection or transaction.
pub(super) async fn mailbox_state(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<ImapMailboxState>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {MAILBOX_COLUMNS} FROM imap_mailboxes WHERE identifier = ?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .as_ref()
    .map(decode_mailbox)
    .transpose()
    .storage()
}

/// Allocate the next persistent UIDVALIDITY within the caller's transaction.
pub(super) async fn allocate_uid_validity(
    connection: &mut SqliteConnection,
) -> Result<u32, StorageError> {
    let changed = sqlx::query(
        "UPDATE imap_uid_validity SET last_value = max(last_value + 1, ?1) WHERE singleton = 1 AND last_value < ?2",
    )
    .bind(now_unix().clamp(1, MAXIMUM_UID))
    .bind(MAXIMUM_UID)
    .execute(&mut *connection)
    .await
    .storage()?
    .rows_affected();
    if changed == 0 {
        return Err(StorageError::IntegerRange);
    }
    let value: i64 =
        sqlx::query_scalar("SELECT last_value FROM imap_uid_validity WHERE singleton = 1")
            .fetch_one(connection)
            .await
            .storage()?;
    uid_from(value)
}

#[cfg(test)]
mod tests {
    use super::decode_flag_row;
    use sandpost_core::ImapMessageFlags;
    use sqlx::Connection;

    /// Flag aliases preserve membership and shared flags across reordered and NULL columns.
    #[tokio::test]
    async fn flag_decoding_uses_names_and_preserves_null_defaults() {
        let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        let row = sqlx::query(
            "SELECT 'custom tagged' AS shared_keywords, 1 AS shared_flagged, 0 AS shared_seen, 1 AS member_deleted, NULL AS shared_answered, 0 AS shared_draft",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        assert_eq!(
            decode_flag_row(&row).unwrap(),
            ImapMessageFlags {
                seen: false,
                answered: false,
                flagged: true,
                draft: false,
                deleted: true,
                keywords: ["custom".to_owned(), "tagged".to_owned()]
                    .into_iter()
                    .collect(),
            }
        );
        let absent = sqlx::query(
            "SELECT NULL AS shared_keywords, NULL AS shared_seen, 0 AS member_deleted, NULL AS shared_answered, NULL AS shared_flagged, NULL AS shared_draft",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        assert_eq!(
            decode_flag_row(&absent).unwrap(),
            ImapMessageFlags::default()
        );
    }
}
