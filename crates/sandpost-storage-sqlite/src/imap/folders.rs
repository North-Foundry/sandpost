//! Account-local IMAP folder persistence.

use super::accounts::get_account;
use super::shared::now_unix;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use sandpost_core::{ImapAccountIdentifier, ImapFolderIdentifier, ImapLocalFolder};
use sandpost_storage::{ImapLocalFolderSettings, NewImapLocalFolder, StorageError};
use sqlx::{Row, SqliteConnection};

/// Named local folder columns shared by folder queries.
pub(super) const FOLDER_COLUMNS: &str =
    "identifier,account_identifier,name,filter,include_in_inbox,created_at,updated_at";

/// Decode a local folder row.
pub(super) fn decode_folder(row: &sqlx::sqlite::SqliteRow) -> Result<ImapLocalFolder, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let account: String = row.try_get("account_identifier")?;
    Ok(ImapLocalFolder {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        account_identifier: parse_identifier(&account)
            .map_err(|_| invalid_column("account_identifier"))?,
        name: row.try_get("name")?,
        filter: row.try_get("filter")?,
        include_in_inbox: row.try_get("include_in_inbox")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Create a local folder after validating its account and name inside the caller's transaction.
pub(super) async fn create_folder(
    connection: &mut SqliteConnection,
    folder: &NewImapLocalFolder,
) -> Result<ImapLocalFolder, StorageError> {
    if get_account(connection, folder.account_identifier)
        .await?
        .is_none()
    {
        return Err(StorageError::NotFound);
    }
    let duplicate: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM imap_local_folders WHERE account_identifier = ?1 AND name = ?2)")
        .bind(folder.account_identifier.to_string())
        .bind(&folder.name)
        .fetch_one(&mut *connection)
        .await
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    let identifier = ImapFolderIdentifier::new();
    sqlx::query("INSERT INTO imap_local_folders(identifier,account_identifier,name,filter,include_in_inbox,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6)")
        .bind(identifier.to_string())
        .bind(folder.account_identifier.to_string())
        .bind(&folder.name)
        .bind(&folder.filter)
        .bind(folder.include_in_inbox)
        .bind(folder.created_at)
        .execute(&mut *connection)
        .await
        .storage()?;
    get_folder(connection, identifier)
        .await?
        .ok_or(StorageError::NotFound)
}

/// Load one local folder.
pub(super) async fn get_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
) -> Result<Option<ImapLocalFolder>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {FOLDER_COLUMNS} FROM imap_local_folders WHERE identifier = ?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .as_ref()
    .map(decode_folder)
    .transpose()
    .storage()
}

/// Replace a local folder's settings after checking account-local name uniqueness.
pub(super) async fn update_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
    settings: &ImapLocalFolderSettings,
) -> Result<Option<ImapLocalFolder>, StorageError> {
    let Some(existing) = get_folder(connection, identifier).await? else {
        return Ok(None);
    };
    let duplicate: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM imap_local_folders WHERE account_identifier = ?1 AND name = ?2 AND identifier != ?3)")
        .bind(existing.account_identifier.to_string())
        .bind(&settings.name)
        .bind(identifier.to_string())
        .fetch_one(&mut *connection)
        .await
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    sqlx::query("UPDATE imap_local_folders SET name = ?1, filter = ?2, include_in_inbox = ?3, updated_at = ?4 WHERE identifier = ?5")
        .bind(&settings.name)
        .bind(&settings.filter)
        .bind(settings.include_in_inbox)
        .bind(now_unix())
        .bind(identifier.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
    get_folder(connection, identifier).await
}

/// List an account's local folders by name and identifier.
pub(super) async fn list_folders(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<Vec<ImapLocalFolder>, StorageError> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {FOLDER_COLUMNS} FROM imap_local_folders WHERE account_identifier = ?1 ORDER BY name, identifier")))
        .bind(account.to_string())
        .fetch_all(connection)
        .await
        .storage()?;
    rows.iter()
        .map(decode_folder)
        .collect::<Result<_, _>>()
        .storage()
}

/// Delete a local folder and its cascading mailbox state.
pub(super) async fn delete_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
) -> Result<bool, StorageError> {
    Ok(
        sqlx::query("DELETE FROM imap_local_folders WHERE identifier = ?1")
            .bind(identifier.to_string())
            .execute(connection)
            .await
            .storage()?
            .rows_affected()
            > 0,
    )
}
