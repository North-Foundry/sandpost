//! IMAP account credentials and linked View configuration.

use super::shared::now_unix;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sandpost_core::{ImapAccount, ImapAccountIdentifier, ImapLinkedView};
use sandpost_storage::{ImapAccountCredential, NewImapAccount, StorageError};

/// Public account columns in their fixed selection order; the hash is selected separately.
pub(super) const ACCOUNT_COLUMNS: &str = "identifier,owner_identifier,name,username,enabled,mirror_views,created_at,updated_at,last_used_at";
/// Seconds within which a repeated login does not rewrite `last_used_at`.
pub(super) const LAST_USE_RESOLUTION_SECONDS: i64 = 60;

/// Decode the account columns that start a row.
pub(super) fn decode_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImapAccount> {
    let identifier: String = row.get(0)?;
    let owner: String = row.get(1)?;
    Ok(ImapAccount {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column(0, "identifier"))?,
        owner_identifier: parse_identifier(&owner)
            .map_err(|_| invalid_column(1, "owner_identifier"))?,
        name: row.get(2)?,
        username: row.get(3)?,
        enabled: row.get(4)?,
        mirror_views: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        last_used_at: row.get(8)?,
    })
}

/// Decode an account followed by its password hash.
fn decode_credential(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImapAccountCredential> {
    Ok(ImapAccountCredential {
        account: decode_account(row)?,
        password_hash: row.get(9)?,
    })
}

/// Create an account, rejecting a taken username in the same transaction.
pub(super) fn create_account_blocking(
    connection: &mut Connection,
    account: &NewImapAccount,
) -> Result<ImapAccount, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let duplicate: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM imap_accounts WHERE username = ?1)",
            [&account.username],
            |row| row.get(0),
        )
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateImapUsername);
    }
    let owner_exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE identifier = ?1)",
            [account.owner_identifier.to_string()],
            |row| row.get(0),
        )
        .storage()?;
    if !owner_exists {
        return Err(StorageError::NotFound);
    }
    let identifier = ImapAccountIdentifier::new();
    transaction
        .execute(
            "INSERT INTO imap_accounts(identifier,owner_identifier,name,username,password_hash,enabled,mirror_views,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,1,?6,?7,?7)",
            params![
                identifier.to_string(),
                account.owner_identifier.to_string(),
                account.name,
                account.username,
                account.password_hash,
                account.mirror_views,
                account.created_at,
            ],
        )
        .storage()?;
    let created = get_account_blocking(&transaction, identifier)?.ok_or(StorageError::NotFound)?;
    transaction.commit().storage()?;
    Ok(created)
}

/// Load one account without its hash.
pub(super) fn get_account_blocking(
    connection: &Connection,
    identifier: ImapAccountIdentifier,
) -> Result<Option<ImapAccount>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {ACCOUNT_COLUMNS} FROM imap_accounts WHERE identifier = ?1"),
            [identifier.to_string()],
            decode_account,
        )
        .optional()
        .storage()
}

/// Load one account and its hash by a column that identifies at most one row.
pub(super) fn get_credential_blocking(
    connection: &Connection,
    column: &str,
    value: &str,
) -> Result<Option<ImapAccountCredential>, StorageError> {
    connection
        .query_row(
            &format!(
                "SELECT {ACCOUNT_COLUMNS},password_hash FROM imap_accounts WHERE {column} = ?1"
            ),
            [value],
            decode_credential,
        )
        .optional()
        .storage()
}

/// Replace the linked View selection after checking that every View exists.
pub(super) fn set_linked_views_blocking(
    connection: &mut Connection,
    account: ImapAccountIdentifier,
    views: &[ImapLinkedView],
) -> Result<(), StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    if get_account_blocking(&transaction, account)?.is_none() {
        return Err(StorageError::NotFound);
    }
    transaction
        .execute(
            "DELETE FROM imap_account_views WHERE account_identifier = ?1",
            [account.to_string()],
        )
        .storage()?;
    for view in views {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM views WHERE identifier = ?1)",
                [view.view_identifier.to_string()],
                |row| row.get(0),
            )
            .storage()?;
        if !exists {
            return Err(StorageError::NotFound);
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO imap_account_views(account_identifier,view_identifier,alias) VALUES (?1,?2,?3)",
                params![account.to_string(), view.view_identifier.to_string(), view.alias],
            )
            .storage()?;
    }
    transaction.commit().storage()
}

/// List an owner's accounts by name and identifier without password hashes.
pub(super) fn list_accounts_blocking(
    connection: &Connection,
    owner: sandpost_core::UserIdentifier,
) -> Result<Vec<ImapAccount>, StorageError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM imap_accounts WHERE owner_identifier = ?1 ORDER BY name, identifier"
        ))
        .storage()?;
    statement
        .query_map([owner.to_string()], decode_account)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// Replace an account's display name and enabled or mirror settings.
pub(super) fn update_account_blocking(
    connection: &Connection,
    identifier: ImapAccountIdentifier,
    settings: &sandpost_storage::ImapAccountSettings,
) -> Result<Option<ImapAccount>, StorageError> {
    let changed = connection
        .execute(
            "UPDATE imap_accounts SET name = ?1, enabled = ?2, mirror_views = ?3, updated_at = ?4 WHERE identifier = ?5",
            params![settings.name, settings.enabled, settings.mirror_views, now_unix(), identifier.to_string()],
        )
        .storage()?;
    if changed == 0 {
        return Ok(None);
    }
    get_account_blocking(connection, identifier)
}

/// Replace the password hash and return the updated public account.
pub(super) fn replace_password_blocking(
    connection: &Connection,
    identifier: ImapAccountIdentifier,
    password_hash: &str,
) -> Result<Option<ImapAccount>, StorageError> {
    let changed = connection
        .execute(
            "UPDATE imap_accounts SET password_hash = ?1, updated_at = ?2 WHERE identifier = ?3",
            params![password_hash, now_unix(), identifier.to_string()],
        )
        .storage()?;
    if changed == 0 {
        return Ok(None);
    }
    get_account_blocking(connection, identifier)
}

/// Delete an account and its cascading IMAP state without deleting mail.
pub(super) fn delete_account_blocking(
    connection: &Connection,
    identifier: ImapAccountIdentifier,
) -> Result<bool, StorageError> {
    Ok(connection
        .execute(
            "DELETE FROM imap_accounts WHERE identifier = ?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0)
}

/// Record successful account use, skipping writes within the timestamp resolution window.
pub(super) fn record_account_use_blocking(
    connection: &Connection,
    identifier: ImapAccountIdentifier,
    used_at: i64,
) -> Result<(), StorageError> {
    connection
        .execute(
            "UPDATE imap_accounts SET last_used_at = ?1 WHERE identifier = ?2 AND (last_used_at IS NULL OR last_used_at <= ?1 - ?3)",
            params![used_at, identifier.to_string(), LAST_USE_RESOLUTION_SECONDS],
        )
        .storage()?;
    Ok(())
}

/// List linked Views ordered by View identifier.
pub(super) fn list_linked_views_blocking(
    connection: &Connection,
    account: ImapAccountIdentifier,
) -> Result<Vec<ImapLinkedView>, StorageError> {
    use sandpost_core::ViewIdentifier;

    let mut statement = connection
        .prepare(
            "SELECT view_identifier, alias FROM imap_account_views WHERE account_identifier = ?1 ORDER BY view_identifier",
        )
        .storage()?;
    let rows = statement
        .query_map([account.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    rows.into_iter()
        .map(|(view, alias)| {
            Ok(ImapLinkedView {
                view_identifier: parse_identifier::<ViewIdentifier>(&view)?,
                alias,
            })
        })
        .collect()
}
