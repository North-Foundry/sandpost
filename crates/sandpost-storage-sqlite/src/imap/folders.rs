//! Owner-shared IMAP folder definitions and account mailbox projections.

use super::accounts::get_account;
use super::shared::now_unix;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use sandpost_core::{
    ImapAccountIdentifier, ImapFolderIdentifier, ImapFolderKind, ImapLocalFolder, UserIdentifier,
};
use sandpost_storage::{ImapLocalFolderSettings, NewImapLocalFolder, StorageError};
use sqlx::{Row, SqliteConnection};

/// Decode a folder projection using its authoritative owner-level definition.
pub(super) fn decode_folder(row: &sqlx::sqlite::SqliteRow) -> Result<ImapLocalFolder, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let account: String = row.try_get("account_identifier")?;
    let kind: String = row.try_get("kind")?;
    let definition: Option<String> = row.try_get("definition_identifier")?;
    Ok(ImapLocalFolder {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        account_identifier: parse_identifier(&account)
            .map_err(|_| invalid_column("account_identifier"))?,
        name: row.try_get("name")?,
        kind: match kind.as_str() {
            "smart" => ImapFolderKind::Smart,
            "manual" => ImapFolderKind::Manual,
            "private" => ImapFolderKind::Private,
            _ => return Err(invalid_column("kind")),
        },
        definition_identifier: definition
            .map(|value| {
                parse_identifier(&value).map_err(|_| invalid_column("definition_identifier"))
            })
            .transpose()?,
        filter: row.try_get("filter")?,
        include_in_inbox: row.try_get("include_in_inbox")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const FOLDER_SELECT: &str = "SELECT local.identifier AS identifier, local.account_identifier AS account_identifier, coalesce(web.name, definition.name, local.name) AS name, local.kind AS kind, coalesce(web.identifier, link.folder_identifier, local.definition_identifier) AS definition_identifier, local.filter AS filter, local.include_in_inbox AS include_in_inbox, local.created_at AS created_at, local.updated_at AS updated_at FROM imap_local_folders AS local LEFT JOIN imap_user_folder_accounts AS link ON link.local_folder_identifier = local.identifier LEFT JOIN imap_user_folders AS definition ON definition.identifier = link.folder_identifier LEFT JOIN web_folder_accounts AS web_link ON web_link.local_folder_identifier = local.identifier LEFT JOIN web_folders AS web ON web.identifier = web_link.folder_identifier";

/// Create one account's disposable projection for each existing owner definition.
pub(super) async fn project_owner_folders(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    owner: UserIdentifier,
) -> Result<(), StorageError> {
    let rows = sqlx::query("SELECT identifier, name, filter, include_in_inbox, created_at, updated_at FROM imap_user_folders WHERE owner_identifier = ?1")
        .bind(owner.to_string())
        .fetch_all(&mut *connection)
        .await
        .storage()?;
    for row in rows {
        let definition: String = row.try_get("identifier").storage()?;
        let local = ImapFolderIdentifier::new();
        sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at, kind, definition_identifier) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'smart', ?8)")
            .bind(local.to_string())
            .bind(account.to_string())
            .bind(format!("__sandpost_owner_folder_{definition}"))
            .bind(row.try_get::<String, _>("filter").storage()?)
            .bind(row.try_get::<bool, _>("include_in_inbox").storage()?)
            .bind(row.try_get::<i64, _>("created_at").storage()?)
            .bind(row.try_get::<i64, _>("updated_at").storage()?)
            .bind(&definition)
            .execute(&mut *connection)
            .await
            .storage()?;
        sqlx::query("INSERT INTO imap_user_folder_accounts(account_identifier, folder_identifier, local_folder_identifier) VALUES (?1, ?2, ?3)")
            .bind(account.to_string())
            .bind(definition)
            .bind(local.to_string())
            .execute(&mut *connection)
            .await
            .storage()?;
    }
    Ok(())
}

/// Create an owner definition and a projection in every existing account.
pub(super) async fn create_folder(
    connection: &mut SqliteConnection,
    folder: &NewImapLocalFolder,
) -> Result<ImapLocalFolder, StorageError> {
    let account = get_account(connection, folder.account_identifier)
        .await?
        .ok_or(StorageError::NotFound)?;
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM imap_user_folders WHERE owner_identifier = ?1 AND name = ?2 UNION ALL SELECT 1 FROM web_folders WHERE owner_identifier = ?1 AND name = ?2)",
    )
    .bind(account.owner_identifier.to_string())
    .bind(&folder.name)
    .fetch_one(&mut *connection)
    .await
    .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    let definition = ImapFolderIdentifier::new();
    sqlx::query("INSERT INTO imap_user_folders(identifier, owner_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)")
        .bind(definition.to_string())
        .bind(account.owner_identifier.to_string())
        .bind(&folder.name)
        .bind(&folder.filter)
        .bind(folder.include_in_inbox)
        .bind(folder.created_at)
        .execute(&mut *connection)
        .await
        .storage()?;
    let accounts = sqlx::query_scalar::<_, String>(
        "SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1 ORDER BY identifier",
    )
    .bind(account.owner_identifier.to_string())
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    let mut created = None;
    for account_id in accounts {
        let local = ImapFolderIdentifier::new();
        sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at, kind, definition_identifier) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 'smart', ?7)")
            .bind(local.to_string())
            .bind(&account_id)
            .bind(format!("__sandpost_owner_folder_{definition}"))
            .bind(&folder.filter)
            .bind(folder.include_in_inbox)
            .bind(folder.created_at)
            .bind(definition.to_string())
            .execute(&mut *connection)
            .await
            .storage()?;
        sqlx::query("INSERT INTO imap_user_folder_accounts(account_identifier, folder_identifier, local_folder_identifier) VALUES (?1, ?2, ?3)")
            .bind(&account_id)
            .bind(definition.to_string())
            .bind(local.to_string())
            .execute(&mut *connection)
            .await
            .storage()?;
        if account_id == folder.account_identifier.to_string() {
            created = Some(local);
        }
    }
    get_folder(connection, created.ok_or(StorageError::NotFound)?)
        .await?
        .ok_or(StorageError::NotFound)
}

/// Create a folder private to one account, without an owner-level definition.
pub(super) async fn create_private_folder(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    name: &str,
    created_at: i64,
) -> Result<ImapLocalFolder, StorageError> {
    if get_account(connection, account).await?.is_none() {
        return Err(StorageError::NotFound);
    }
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM imap_local_folders WHERE account_identifier = ?1 AND name = ?2)",
    )
    .bind(account.to_string())
    .bind(name)
    .fetch_one(&mut *connection)
    .await
    .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    let identifier = ImapFolderIdentifier::new();
    sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at, kind) VALUES (?1, ?2, ?3, 'true', 0, ?4, ?4, 'private')")
        .bind(identifier.to_string())
        .bind(account.to_string())
        .bind(name)
        .bind(created_at)
        .execute(&mut *connection)
        .await
        .storage()?;
    get_folder(connection, identifier)
        .await?
        .ok_or(StorageError::NotFound)
}

/// Load one account projection and hydrate its fields from the canonical definition.
pub(super) async fn get_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
) -> Result<Option<ImapLocalFolder>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "{FOLDER_SELECT} WHERE local.identifier = ?1"
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

/// Update the owner definition behind a projection and synchronize every copy atomically.
pub(super) async fn update_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
    settings: &ImapLocalFolderSettings,
) -> Result<Option<ImapLocalFolder>, StorageError> {
    let Some(existing) = get_folder(connection, identifier).await? else {
        return Ok(None);
    };
    if existing.kind == ImapFolderKind::Private {
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
        sqlx::query(
            "UPDATE imap_local_folders SET name = ?1, updated_at = ?2 WHERE identifier = ?3",
        )
        .bind(&settings.name)
        .bind(now_unix())
        .bind(identifier.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
        return get_folder(connection, identifier).await;
    }
    if existing.kind == ImapFolderKind::Manual {
        return Err(StorageError::ConstraintViolation(
            "Web-managed manual folders are updated through their owner definition".into(),
        ));
    }
    let definition: String = sqlx::query_scalar("SELECT folder_identifier FROM imap_user_folder_accounts WHERE local_folder_identifier = ?1")
        .bind(identifier.to_string())
        .fetch_optional(&mut *connection)
        .await
        .storage()?
        .ok_or(StorageError::NotFound)?;
    if settings.name != existing.name {
        let duplicate: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM imap_user_folders AS other JOIN imap_user_folders AS current ON current.identifier = ?3 WHERE other.owner_identifier = current.owner_identifier AND other.name = ?1 AND other.identifier != ?2 UNION ALL SELECT 1 FROM web_folders WHERE owner_identifier = (SELECT owner_identifier FROM imap_user_folders WHERE identifier = ?3) AND name = ?1)")
            .bind(&settings.name)
            .bind(&definition)
            .bind(&definition)
            .fetch_one(&mut *connection)
            .await
            .storage()?;
        if duplicate {
            return Err(StorageError::DuplicateImapFolderName);
        }
    }
    let updated_at = now_unix();
    sqlx::query("UPDATE imap_user_folders SET name = ?1, filter = ?2, include_in_inbox = ?3, updated_at = ?4 WHERE identifier = ?5")
        .bind(&settings.name)
        .bind(&settings.filter)
        .bind(settings.include_in_inbox)
        .bind(updated_at)
        .bind(&definition)
        .execute(&mut *connection)
        .await
        .storage()?;
    sqlx::query("UPDATE imap_local_folders SET filter = ?1, include_in_inbox = ?2, updated_at = ?3 WHERE identifier IN (SELECT local_folder_identifier FROM imap_user_folder_accounts WHERE folder_identifier = ?4)")
        .bind(&settings.filter)
        .bind(settings.include_in_inbox)
        .bind(updated_at)
        .bind(definition)
        .execute(&mut *connection)
        .await
        .storage()?;
    get_folder(connection, identifier).await.map(|folder| {
        folder.map(|mut folder| {
            folder.account_identifier = existing.account_identifier;
            folder
        })
    })
}

/// List an account's projections, always reading canonical definition fields.
pub(super) async fn list_folders(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<Vec<ImapLocalFolder>, StorageError> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{FOLDER_SELECT} WHERE local.account_identifier = ?1 ORDER BY name, local.identifier"
    )))
    .bind(account.to_string())
    .fetch_all(connection)
    .await
    .storage()?;
    rows.iter()
        .map(decode_folder)
        .collect::<Result<_, _>>()
        .storage()
}

/// Delete private or legacy shared definitions and projections, preserving physical messages.
pub(super) async fn delete_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
) -> Result<bool, StorageError> {
    let Some(existing) = get_folder(connection, identifier).await? else {
        return Ok(false);
    };
    if existing.kind == ImapFolderKind::Private {
        let deleted = sqlx::query("DELETE FROM imap_local_folders WHERE identifier = ?1")
            .bind(identifier.to_string())
            .execute(&mut *connection)
            .await
            .storage()?
            .rows_affected()
            > 0;
        sqlx::query(
            "UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier = ?1",
        )
        .bind(existing.account_identifier.to_string())
        .execute(connection)
        .await
        .storage()?;
        return Ok(deleted);
    }
    if existing.kind == ImapFolderKind::Manual {
        return Err(StorageError::ConstraintViolation(
            "Web-managed manual folders are deleted through their owner definition".into(),
        ));
    }
    let definition = existing
        .definition_identifier
        .ok_or(StorageError::NotFound)?
        .to_string();
    let owner: String =
        sqlx::query_scalar("SELECT owner_identifier FROM imap_user_folders WHERE identifier = ?1")
            .bind(&definition)
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    sqlx::query("DELETE FROM imap_local_folders WHERE identifier IN (SELECT local_folder_identifier FROM imap_user_folder_accounts WHERE folder_identifier = ?1)")
        .bind(&definition)
        .execute(&mut *connection)
        .await
        .storage()?;
    let deleted = sqlx::query("DELETE FROM imap_user_folders WHERE identifier = ?1")
        .bind(&definition)
        .execute(&mut *connection)
        .await
        .storage()?
        .rows_affected()
        > 0;
    sqlx::query("UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier IN (SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1)")
        .bind(owner).execute(connection).await.storage()?;
    Ok(deleted)
}
