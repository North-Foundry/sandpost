//! Web-managed manual folder definitions and their selected IMAP projections.

use super::shared::now_unix;
use crate::error::StorageResult;
use crate::records::parse_identifier;
use sandpost_core::{ImapAccountIdentifier, ImapFolderIdentifier, UserIdentifier, WebFolder};
use sandpost_storage::StorageError;
use sqlx::{Row, SqliteConnection};

/// Load a Web folder with its exposed account identifiers.
pub(super) async fn get_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
) -> Result<Option<WebFolder>, StorageError> {
    let Some(row) = sqlx::query("SELECT identifier, owner_identifier, name, icon, created_at, updated_at FROM web_folders WHERE identifier = ?1")
        .bind(identifier.to_string()).fetch_optional(&mut *connection).await.storage()? else { return Ok(None); };
    let identifier: String = row.try_get("identifier").storage()?;
    let owner: String = row.try_get("owner_identifier").storage()?;
    let accounts = sqlx::query_scalar::<_, String>("SELECT account_identifier FROM web_folder_accounts WHERE folder_identifier = ?1 ORDER BY account_identifier")
        .bind(&identifier).fetch_all(&mut *connection).await.storage()?;
    Ok(Some(WebFolder {
        identifier: parse_identifier(&identifier)?,
        owner_identifier: parse_identifier(&owner)?,
        name: row.try_get("name").storage()?,
        icon: row.try_get("icon").storage()?,
        exposed_accounts: accounts
            .iter()
            .map(|value| parse_identifier(value))
            .collect::<Result<_, _>>()?,
        created_at: row.try_get("created_at").storage()?,
        updated_at: row.try_get("updated_at").storage()?,
    }))
}

/// List all manual folders belonging to one owner.
pub(super) async fn list_folders(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
) -> Result<Vec<WebFolder>, StorageError> {
    let identifiers = sqlx::query_scalar::<_, String>(
        "SELECT identifier FROM web_folders WHERE owner_identifier = ?1 ORDER BY name, identifier",
    )
    .bind(owner.to_string())
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    let mut folders = Vec::with_capacity(identifiers.len());
    for identifier in identifiers {
        if let Some(folder) = get_folder(connection, parse_identifier(&identifier)?).await? {
            folders.push(folder);
        }
    }
    Ok(folders)
}

/// Ensure every selected account belongs to the folder owner.
async fn validate_accounts(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    accounts: &[ImapAccountIdentifier],
) -> Result<Vec<ImapAccountIdentifier>, StorageError> {
    let mut unique = accounts.to_vec();
    unique.sort_unstable();
    unique.dedup();
    for account in &unique {
        let belongs: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM imap_accounts WHERE identifier = ?1 AND owner_identifier = ?2)")
            .bind(account.to_string()).bind(owner.to_string()).fetch_one(&mut *connection).await.storage()?;
        if !belongs {
            return Err(StorageError::NotFound);
        }
    }
    Ok(unique)
}

/// Create one manual definition and account-local projections for selected exposures.
pub(super) async fn create_folder(
    connection: &mut SqliteConnection,
    owner: UserIdentifier,
    name: &str,
    icon: Option<&str>,
    accounts: &[ImapAccountIdentifier],
    created_at: i64,
) -> Result<WebFolder, StorageError> {
    let mut accounts = validate_accounts(connection, owner, accounts).await?;
    let mirror_accounts = sqlx::query_scalar::<_, String>(
        "SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1 AND mode = 'mirror'",
    )
    .bind(owner.to_string())
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    for account in mirror_accounts {
        let account = parse_identifier(&account)?;
        if !accounts.contains(&account) {
            accounts.push(account);
        }
    }
    let duplicate: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM web_folders WHERE owner_identifier = ?1 AND name = ?2 UNION ALL SELECT 1 FROM imap_user_folders WHERE owner_identifier = ?1 AND name = ?2)")
        .bind(owner.to_string()).bind(name).fetch_one(&mut *connection).await.storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    let identifier = ImapFolderIdentifier::new();
    sqlx::query("INSERT INTO web_folders(identifier, owner_identifier, name, icon, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)")
        .bind(identifier.to_string()).bind(owner.to_string()).bind(name).bind(icon).bind(created_at).execute(&mut *connection).await.storage()?;
    for account in accounts {
        create_projection(connection, identifier, account, created_at).await?;
    }
    get_folder(connection, identifier)
        .await?
        .ok_or(StorageError::NotFound)
}

/// Create one account-local projection with its own mailbox and UID identity.
pub(super) async fn create_projection(
    connection: &mut SqliteConnection,
    folder: ImapFolderIdentifier,
    account: ImapAccountIdentifier,
    created_at: i64,
) -> Result<(), StorageError> {
    let existing: Option<String> = sqlx::query_scalar("SELECT local_folder_identifier FROM web_folder_accounts WHERE folder_identifier = ?1 AND account_identifier = ?2")
        .bind(folder.to_string())
        .bind(account.to_string())
        .fetch_optional(&mut *connection)
        .await
        .storage()?;
    if existing.is_some() {
        return Ok(());
    }
    let local = ImapFolderIdentifier::new();
    sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at, kind, definition_identifier, organization_mode) VALUES (?1, ?2, ?3, 'true', 0, ?4, ?4, 'manual', ?5, 'mirror')")
        .bind(local.to_string()).bind(account.to_string()).bind(format!("__sandpost_web_folder_{folder}"))
        .bind(created_at).bind(folder.to_string()).execute(&mut *connection).await.storage()?;
    sqlx::query("INSERT INTO web_folder_accounts(folder_identifier, account_identifier, local_folder_identifier) VALUES (?1, ?2, ?3)")
        .bind(folder.to_string()).bind(account.to_string()).bind(local.to_string()).execute(&mut *connection).await.storage()?;
    Ok(())
}

/// Replace a folder's name and account exposures atomically.
pub(super) async fn update_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
    name: &str,
    icon: Option<&str>,
    requested_accounts: &[ImapAccountIdentifier],
) -> Result<Option<WebFolder>, StorageError> {
    let Some(existing) = get_folder(connection, identifier).await? else {
        return Ok(None);
    };
    let mut requested =
        validate_accounts(connection, existing.owner_identifier, requested_accounts).await?;
    let mirror_accounts = sqlx::query_scalar::<_, String>(
        "SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1 AND mode = 'mirror'",
    )
    .bind(existing.owner_identifier.to_string())
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    for account in mirror_accounts {
        let account = parse_identifier(&account)?;
        if !requested.contains(&account) {
            requested.push(account);
        }
    }
    let duplicate: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM web_folders WHERE owner_identifier = ?1 AND name = ?2 AND identifier != ?3 UNION ALL SELECT 1 FROM imap_user_folders WHERE owner_identifier = ?1 AND name = ?2)")
        .bind(existing.owner_identifier.to_string()).bind(name).bind(identifier.to_string()).fetch_one(&mut *connection).await.storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapFolderName);
    }
    super::organization::rename_folder_descendants(
        connection,
        true,
        &existing.owner_identifier.to_string(),
        &existing.name,
        name,
    )
    .await?;
    sqlx::query(
        "UPDATE web_folders SET name = ?1, icon = ?2, updated_at = ?3 WHERE identifier = ?4",
    )
    .bind(name)
    .bind(icon)
    .bind(now_unix())
    .bind(identifier.to_string())
    .execute(&mut *connection)
    .await
    .storage()?;
    let current: Vec<String> = sqlx::query_scalar(
        "SELECT account_identifier FROM web_folder_accounts WHERE folder_identifier = ?1",
    )
    .bind(identifier.to_string())
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    for account in requested
        .iter()
        .filter(|account| !current.contains(&account.to_string()))
    {
        create_projection(connection, identifier, *account, now_unix()).await?;
    }
    sqlx::query("UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier IN (SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1)")
        .bind(existing.owner_identifier.to_string()).execute(&mut *connection).await.storage()?;
    get_folder(connection, identifier).await
}

/// Delete the definition and projections without deleting physical mail.
pub(super) async fn delete_folder(
    connection: &mut SqliteConnection,
    identifier: ImapFolderIdentifier,
) -> Result<bool, StorageError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_identifier FROM web_folders WHERE identifier = ?1")
            .bind(identifier.to_string())
            .fetch_optional(&mut *connection)
            .await
            .storage()?;
    if let Some(owner) = &owner {
        super::organization::delete_primary_folder_occurrences(
            connection,
            parse_identifier(owner)?,
            identifier,
        )
        .await?;
    }
    let local_identifiers: Vec<String> = sqlx::query_scalar(
        "SELECT local_folder_identifier FROM web_folder_accounts WHERE folder_identifier = ?1",
    )
    .bind(identifier.to_string())
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    for local in local_identifiers {
        sqlx::query("DELETE FROM imap_local_folders WHERE identifier = ?1")
            .bind(local)
            .execute(&mut *connection)
            .await
            .storage()?;
    }
    let deleted = sqlx::query("DELETE FROM web_folders WHERE identifier = ?1")
        .bind(identifier.to_string())
        .execute(&mut *connection)
        .await
        .storage()?
        .rows_affected()
        > 0;
    if let Some(owner) = owner {
        sqlx::query("UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier IN (SELECT identifier FROM imap_accounts WHERE owner_identifier = ?1)")
            .bind(owner).execute(connection).await.storage()?;
    }
    Ok(deleted)
}
