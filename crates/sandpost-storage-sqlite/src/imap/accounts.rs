//! IMAP account credentials and linked View configuration.

use super::shared::now_unix;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use sandpost_core::{ImapAccount, ImapAccountIdentifier, ImapLinkedView};
use sandpost_storage::{ImapAccountCredential, NewImapAccount, StorageError};
use sqlx::{Row, SqliteConnection};

/// Named public account columns; the hash is selected separately.
pub(super) const ACCOUNT_COLUMNS: &str = "identifier,owner_identifier,name,username,enabled,mirror_views,created_at,updated_at,last_used_at";
/// Seconds within which a repeated login does not rewrite `last_used_at`.
pub(super) const LAST_USE_RESOLUTION_SECONDS: i64 = 60;

/// Decode named account columns independently of their selection order.
pub(super) fn decode_account(row: &sqlx::sqlite::SqliteRow) -> Result<ImapAccount, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let owner: String = row.try_get("owner_identifier")?;
    Ok(ImapAccount {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        owner_identifier: parse_identifier(&owner)
            .map_err(|_| invalid_column("owner_identifier"))?,
        name: row.try_get("name")?,
        username: row.try_get("username")?,
        enabled: row.try_get("enabled")?,
        mirror_views: row.try_get("mirror_views")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        last_used_at: row.try_get("last_used_at")?,
    })
}

/// Decode an account followed by its password hash.
fn decode_credential(row: &sqlx::sqlite::SqliteRow) -> Result<ImapAccountCredential, sqlx::Error> {
    Ok(ImapAccountCredential {
        account: decode_account(row)?,
        password_hash: row.try_get("password_hash")?,
    })
}

/// Create an account after validating its username and owner inside the caller's transaction.
pub(super) async fn create_account(
    connection: &mut SqliteConnection,
    account: &NewImapAccount,
) -> Result<ImapAccount, StorageError> {
    let duplicate: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM imap_accounts WHERE username = ?1)")
            .bind(&account.username)
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapUsername);
    }
    let owner_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE identifier = ?1)")
            .bind(account.owner_identifier.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    if !owner_exists {
        return Err(StorageError::NotFound);
    }
    let identifier = ImapAccountIdentifier::new();
    sqlx::query("INSERT INTO imap_accounts(identifier,owner_identifier,name,username,password_hash,enabled,mirror_views,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,1,?6,?7,?7)")
        .bind(identifier.to_string())
        .bind(account.owner_identifier.to_string())
        .bind(&account.name)
        .bind(&account.username)
        .bind(&account.password_hash)
        .bind(account.mirror_views)
        .bind(account.created_at)
        .execute(&mut *connection)
        .await
        .storage()?;
    super::folders::project_owner_folders(connection, identifier, account.owner_identifier).await?;
    get_account(connection, identifier)
        .await?
        .ok_or(StorageError::NotFound)
}

/// Load one account without its hash.
pub(super) async fn get_account(
    connection: &mut SqliteConnection,
    identifier: ImapAccountIdentifier,
) -> Result<Option<ImapAccount>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM imap_accounts WHERE identifier = ?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .as_ref()
    .map(decode_account)
    .transpose()
    .storage()
}

/// Load one account and its hash by a column that identifies at most one row.
pub(super) async fn get_credential(
    connection: &mut SqliteConnection,
    column: &str,
    value: &str,
) -> Result<Option<ImapAccountCredential>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS},password_hash FROM imap_accounts WHERE {column} = ?1"
    )))
    .bind(value)
    .fetch_optional(connection)
    .await
    .storage()?
    .as_ref()
    .map(decode_credential)
    .transpose()
    .storage()
}

/// Replace the linked View selection after the caller checks every View exists.
pub(super) async fn set_linked_views(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    views: &[ImapLinkedView],
) -> Result<(), StorageError> {
    if get_account(connection, account).await?.is_none() {
        return Err(StorageError::NotFound);
    }
    sqlx::query("DELETE FROM imap_account_views WHERE account_identifier = ?1")
        .bind(account.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
    for view in views {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM views WHERE identifier = ?1)")
                .bind(view.view_identifier.to_string())
                .fetch_one(&mut *connection)
                .await
                .storage()?;
        if !exists {
            return Err(StorageError::NotFound);
        }
        sqlx::query("INSERT OR REPLACE INTO imap_account_views(account_identifier,view_identifier,alias) VALUES (?1,?2,?3)")
            .bind(account.to_string())
            .bind(view.view_identifier.to_string())
            .bind(&view.alias)
            .execute(&mut *connection)
            .await
            .storage()?;
    }
    Ok(())
}

/// List an owner's accounts by name and identifier without password hashes.
pub(super) async fn list_accounts(
    connection: &mut SqliteConnection,
    owner: sandpost_core::UserIdentifier,
) -> Result<Vec<ImapAccount>, StorageError> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {ACCOUNT_COLUMNS} FROM imap_accounts WHERE owner_identifier = ?1 ORDER BY name, identifier")))
        .bind(owner.to_string())
        .fetch_all(connection)
        .await
        .storage()?;
    rows.iter()
        .map(decode_account)
        .collect::<Result<_, _>>()
        .storage()
}

/// Replace an account's display name and enabled or mirror settings.
pub(super) async fn update_account(
    connection: &mut SqliteConnection,
    identifier: ImapAccountIdentifier,
    settings: &sandpost_storage::ImapAccountSettings,
) -> Result<Option<ImapAccount>, StorageError> {
    let changed = sqlx::query("UPDATE imap_accounts SET name = ?1, enabled = ?2, mirror_views = ?3, updated_at = ?4 WHERE identifier = ?5")
        .bind(&settings.name)
        .bind(settings.enabled)
        .bind(settings.mirror_views)
        .bind(now_unix())
        .bind(identifier.to_string())
        .execute(&mut *connection)
        .await
        .storage()?
        .rows_affected();
    if changed == 0 {
        return Ok(None);
    }
    get_account(connection, identifier).await
}

/// Replace the password hash and return the updated public account.
pub(super) async fn replace_password(
    connection: &mut SqliteConnection,
    identifier: ImapAccountIdentifier,
    password_hash: &str,
) -> Result<Option<ImapAccount>, StorageError> {
    let changed = sqlx::query(
        "UPDATE imap_accounts SET password_hash = ?1, updated_at = ?2 WHERE identifier = ?3",
    )
    .bind(password_hash)
    .bind(now_unix())
    .bind(identifier.to_string())
    .execute(&mut *connection)
    .await
    .storage()?
    .rows_affected();
    if changed == 0 {
        return Ok(None);
    }
    get_account(connection, identifier).await
}

/// Delete an account and its cascading IMAP state without deleting mail.
pub(super) async fn delete_account(
    connection: &mut SqliteConnection,
    identifier: ImapAccountIdentifier,
) -> Result<bool, StorageError> {
    Ok(
        sqlx::query("DELETE FROM imap_accounts WHERE identifier = ?1")
            .bind(identifier.to_string())
            .execute(connection)
            .await
            .storage()?
            .rows_affected()
            > 0,
    )
}

/// Record successful account use, skipping writes within the timestamp resolution window.
pub(super) async fn record_account_use(
    connection: &mut SqliteConnection,
    identifier: ImapAccountIdentifier,
    used_at: i64,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE imap_accounts SET last_used_at = ?1 WHERE identifier = ?2 AND (last_used_at IS NULL OR last_used_at <= ?1 - ?3)")
        .bind(used_at)
        .bind(identifier.to_string())
        .bind(LAST_USE_RESOLUTION_SECONDS)
        .execute(connection)
        .await
        .storage()?;
    Ok(())
}

/// List linked Views ordered by View identifier.
pub(super) async fn list_linked_views(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<Vec<ImapLinkedView>, StorageError> {
    use sandpost_core::ViewIdentifier;
    let rows = sqlx::query("SELECT view_identifier, alias FROM imap_account_views WHERE account_identifier = ?1 ORDER BY view_identifier")
        .bind(account.to_string())
        .fetch_all(connection)
        .await
        .storage()?;
    rows.iter()
        .map(|row| {
            let view: String = row.try_get("view_identifier")?;
            Ok(ImapLinkedView {
                view_identifier: parse_identifier::<ViewIdentifier>(&view)
                    .map_err(|_| invalid_column("view_identifier"))?,
                alias: row.try_get("alias")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .storage()
}
