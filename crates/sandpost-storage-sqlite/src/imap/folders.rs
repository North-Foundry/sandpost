//! Account-local IMAP folder persistence.

use super::accounts::get_account_blocking;
use super::shared::now_unix;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sandpost_core::{ImapAccountIdentifier, ImapFolderIdentifier, ImapLocalFolder};
use sandpost_storage::{ImapLocalFolderSettings, NewImapLocalFolder, StorageError};

/// Local folder columns in their fixed selection order.
pub(super) const FOLDER_COLUMNS: &str =
    "identifier,account_identifier,name,filter,include_in_inbox,created_at,updated_at";

/// Decode a local folder row.
pub(super) fn decode_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImapLocalFolder> {
    let identifier: String = row.get(0)?;
    let account: String = row.get(1)?;
    Ok(ImapLocalFolder {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column(0, "identifier"))?,
        account_identifier: parse_identifier(&account)
            .map_err(|_| invalid_column(1, "account_identifier"))?,
        name: row.get(2)?,
        filter: row.get(3)?,
        include_in_inbox: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

/// Create a local folder, rejecting a duplicate name in the account.
pub(super) fn create_folder_blocking(
    connection: &mut Connection,
    folder: &NewImapLocalFolder,
) -> Result<ImapLocalFolder, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    if get_account_blocking(&transaction, folder.account_identifier)?.is_none() {
        return Err(StorageError::NotFound);
    }
    let duplicate: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM imap_local_folders WHERE account_identifier = ?1 AND name = ?2)",
            params![folder.account_identifier.to_string(), folder.name],
            |row| row.get(0),
        )
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    let identifier = ImapFolderIdentifier::new();
    transaction
        .execute(
            "INSERT INTO imap_local_folders(identifier,account_identifier,name,filter,include_in_inbox,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6)",
            params![
                identifier.to_string(),
                folder.account_identifier.to_string(),
                folder.name,
                folder.filter,
                folder.include_in_inbox,
                folder.created_at,
            ],
        )
        .storage()?;
    let created = get_folder_blocking(&transaction, identifier)?.ok_or(StorageError::NotFound)?;
    transaction.commit().storage()?;
    Ok(created)
}

/// Load one local folder.
pub(super) fn get_folder_blocking(
    connection: &Connection,
    identifier: ImapFolderIdentifier,
) -> Result<Option<ImapLocalFolder>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {FOLDER_COLUMNS} FROM imap_local_folders WHERE identifier = ?1"),
            [identifier.to_string()],
            decode_folder,
        )
        .optional()
        .storage()
}

/// Replace a local folder's settings, rejecting a name used by another folder of the account.
pub(super) fn update_folder_blocking(
    connection: &mut Connection,
    identifier: ImapFolderIdentifier,
    settings: &ImapLocalFolderSettings,
) -> Result<Option<ImapLocalFolder>, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let Some(existing) = get_folder_blocking(&transaction, identifier)? else {
        return Ok(None);
    };
    let duplicate: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM imap_local_folders WHERE account_identifier = ?1 AND name = ?2 AND identifier != ?3)",
            params![
                existing.account_identifier.to_string(),
                settings.name,
                identifier.to_string()
            ],
            |row| row.get(0),
        )
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    transaction
        .execute(
            "UPDATE imap_local_folders SET name = ?1, filter = ?2, include_in_inbox = ?3, updated_at = ?4 WHERE identifier = ?5",
            params![
                settings.name,
                settings.filter,
                settings.include_in_inbox,
                now_unix(),
                identifier.to_string()
            ],
        )
        .storage()?;
    let updated = get_folder_blocking(&transaction, identifier)?;
    transaction.commit().storage()?;
    Ok(updated)
}

/// List an account's local folders by name and identifier.
pub(super) fn list_folders_blocking(
    connection: &Connection,
    account: ImapAccountIdentifier,
) -> Result<Vec<ImapLocalFolder>, StorageError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {FOLDER_COLUMNS} FROM imap_local_folders WHERE account_identifier = ?1 ORDER BY name, identifier"
        ))
        .storage()?;
    statement
        .query_map([account.to_string()], decode_folder)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// Delete a local folder and its cascading mailbox state.
pub(super) fn delete_folder_blocking(
    connection: &Connection,
    identifier: ImapFolderIdentifier,
) -> Result<bool, StorageError> {
    Ok(connection
        .execute(
            "DELETE FROM imap_local_folders WHERE identifier = ?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0)
}
