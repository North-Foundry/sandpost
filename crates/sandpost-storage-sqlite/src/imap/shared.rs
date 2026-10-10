//! Shared IMAP row codecs and UID helpers.

use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use rusqlite::{Connection, OptionalExtension, Transaction};
use sandpost_core::{ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags};
use sandpost_storage::{ImapMailboxState, StorageError};

/// Mailbox state columns in their fixed selection order.
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
pub(super) fn decode_mailbox(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImapMailboxState> {
    let identifier: String = row.get(0)?;
    let account: String = row.get(1)?;
    let source: String = row.get(2)?;
    let view: Option<String> = row.get(3)?;
    let folder: Option<String> = row.get(4)?;
    let source = match (source.as_str(), view, folder) {
        ("inbox", None, None) => ImapMailboxSource::Inbox,
        ("trash", None, None) => ImapMailboxSource::Trash,
        ("view", Some(view), None) => ImapMailboxSource::View(
            parse_identifier(&view).map_err(|_| invalid_column(3, "view_identifier"))?,
        ),
        ("folder", None, Some(folder)) => ImapMailboxSource::LocalFolder(
            parse_identifier(&folder).map_err(|_| invalid_column(4, "folder_identifier"))?,
        ),
        _ => return Err(invalid_column(2, "source")),
    };
    let uid_validity: i64 = row.get(5)?;
    let uid_next: i64 = row.get(6)?;
    let recent_through: i64 = row.get(7)?;
    Ok(ImapMailboxState {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column(0, "identifier"))?,
        account_identifier: parse_identifier(&account)
            .map_err(|_| invalid_column(1, "account_identifier"))?,
        source,
        uid_validity: u32::try_from(uid_validity).map_err(|_| invalid_column(5, "uid_validity"))?,
        // An exhausted mailbox stores 2^32; report the largest representable UID instead.
        uid_next: u32::try_from(uid_next.min(MAXIMUM_UID))
            .map_err(|_| invalid_column(6, "uid_next"))?,
        recent_through: u32::try_from(recent_through)
            .map_err(|_| invalid_column(7, "recent_through"))?,
    })
}

/// The stored source kind and reference columns of a mailbox source.
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

/// keywords.
pub(super) fn flags_at(
    row: &rusqlite::Row<'_>,
    first: usize,
) -> rusqlite::Result<ImapMessageFlags> {
    Ok(decode_flags(
        row.get(first)?,
        row.get(first + 1)?,
        row.get(first + 2)?,
        row.get(first + 3)?,
        row.get(first + 4)?,
        row.get(first + 5)?,
    ))
}

/// Convert a stored non-negative integer to a UID.
pub(super) fn uid_from(value: i64) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::IntegerRange)
}

/// Load one mailbox state inside a connection or transaction.
pub(super) fn mailbox_state(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<ImapMailboxState>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {MAILBOX_COLUMNS} FROM imap_mailboxes WHERE identifier = ?1"),
            [identifier.to_string()],
            decode_mailbox,
        )
        .optional()
        .storage()
}

/// so values also increase across database recreation.
pub(super) fn allocate_uid_validity(transaction: &Transaction<'_>) -> Result<u32, StorageError> {
    transaction
        .execute(
            "UPDATE imap_uid_validity SET last_value = max(last_value + 1, ?1) WHERE singleton = 1",
            [now_unix().clamp(1, MAXIMUM_UID)],
        )
        .storage()?;
    let value: i64 = transaction
        .query_row(
            "SELECT last_value FROM imap_uid_validity WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .storage()?;
    uid_from(value)
}
