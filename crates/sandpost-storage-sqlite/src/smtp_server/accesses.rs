//! Hashed SMTP credentials, mechanism policy, and successful-use metadata.
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sandpost_core::{SmtpAccess, SmtpAccessIdentifier, SmtpAuthenticationMechanism};
use sandpost_storage::{NewSmtpAccess, SmtpAccessCredential, StorageError};

/// Public access columns in their fixed selection order; the hash is selected separately.
const ACCESS_COLUMNS: &str = "identifier,name,username,enabled,created_at,last_used_at,requires_encryption,plain_mechanism_allowed,login_mechanism_allowed";
/// Position of the password hash column after [`ACCESS_COLUMNS`].
const PASSWORD_HASH_COLUMN: usize = 9;
/// Seconds within which a repeated successful authentication does not rewrite `last_used_at`.
const LAST_USE_RESOLUTION_SECONDS: i64 = 60;

/// Current Unix time in seconds for the updated timestamp.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Decode the public access columns that start a row.
fn decode_access(row: &rusqlite::Row<'_>) -> rusqlite::Result<SmtpAccess> {
    let identifier: String = row.get(0)?;
    Ok(SmtpAccess {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column(0, "identifier"))?,
        name: row.get(1)?,
        username: row.get(2)?,
        enabled: row.get(3)?,
        created_at: row.get(4)?,
        last_used_at: row.get(5)?,
        requires_encryption: row.get(6)?,
        allowed_mechanisms: allowed_mechanisms(row.get(7)?, row.get(8)?),
    })
}

/// List the mechanisms whose stored flags are set, in canonical order.
fn allowed_mechanisms(plain: bool, login: bool) -> Vec<SmtpAuthenticationMechanism> {
    SmtpAuthenticationMechanism::ALL
        .into_iter()
        .filter(|mechanism| match mechanism {
            SmtpAuthenticationMechanism::Plain => plain,
            SmtpAuthenticationMechanism::Login => login,
        })
        .collect()
}

/// Turn a mechanism list into the stored PLAIN and LOGIN flags, refusing an empty list.
pub(super) fn mechanism_flags(
    mechanisms: &[SmtpAuthenticationMechanism],
) -> Result<(bool, bool), StorageError> {
    if mechanisms.is_empty() {
        return Err(StorageError::ConstraintViolation(
            "an SMTP access needs at least one AUTH mechanism".into(),
        ));
    }
    Ok((
        mechanisms.contains(&SmtpAuthenticationMechanism::Plain),
        mechanisms.contains(&SmtpAuthenticationMechanism::Login),
    ))
}

/// Decode an access followed by its password hash column.
fn decode_credential(row: &rusqlite::Row<'_>) -> rusqlite::Result<SmtpAccessCredential> {
    Ok(SmtpAccessCredential {
        access: decode_access(row)?,
        password_hash: row.get(PASSWORD_HASH_COLUMN)?,
    })
}

/// List every access by name and identifier without password hashes.
pub(super) fn list_accesses_blocking(
    connection: &Connection,
) -> Result<Vec<SmtpAccess>, StorageError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {ACCESS_COLUMNS} FROM smtp_accesses ORDER BY name, identifier"
        ))
        .storage()?;
    statement
        .query_map([], decode_access)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// Load one access by identifier without its password hash.
pub(super) fn get_access_blocking(
    connection: &Connection,
    identifier: SmtpAccessIdentifier,
) -> Result<Option<SmtpAccess>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {ACCESS_COLUMNS} FROM smtp_accesses WHERE identifier = ?1"),
            [identifier.to_string()],
            decode_access,
        )
        .optional()
        .storage()
}

/// Load one access and its password hash by a column that identifies at most one row.
pub(super) fn get_credential_blocking(
    connection: &Connection,
    column: &str,
    value: &str,
) -> Result<Option<SmtpAccessCredential>, StorageError> {
    connection
        .query_row(
            &format!(
                "SELECT {ACCESS_COLUMNS},password_hash FROM smtp_accesses WHERE {column} = ?1"
            ),
            [value],
            decode_credential,
        )
        .optional()
        .storage()
}

/// Reserve the writer before checking username uniqueness across independent connections.
/// An immediate transaction avoids upgrading a stale WAL read snapshot after another creator wins.
pub(super) fn create_access_blocking(
    connection: &mut Connection,
    access: &NewSmtpAccess,
) -> Result<SmtpAccess, StorageError> {
    let (plain_allowed, login_allowed) = mechanism_flags(&access.allowed_mechanisms)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let duplicate: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM smtp_accesses WHERE username = ?1)",
            [&access.username],
            |row| row.get(0),
        )
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateSmtpUsername);
    }
    let identifier = SmtpAccessIdentifier::new();
    transaction
        .execute(
            "INSERT INTO smtp_accesses(identifier,name,username,password_hash,enabled,created_at,updated_at,requires_encryption,plain_mechanism_allowed,login_mechanism_allowed) VALUES (?1,?2,?3,?4,1,?5,?6,?7,?8,?9)",
            params![
                identifier.to_string(),
                access.name,
                access.username,
                access.password_hash,
                access.created_at,
                now_unix(),
                access.requires_encryption,
                plain_allowed,
                login_allowed,
            ],
        )
        .storage()?;
    let created = get_access_blocking(&transaction, identifier)?.ok_or(StorageError::NotFound)?;
    transaction.commit().storage()?;
    Ok(created)
}

/// Run one single-row update and return the access as stored afterwards.
pub(super) fn update_access_blocking(
    connection: &mut Connection,
    identifier: SmtpAccessIdentifier,
    statement: &str,
    value: &dyn rusqlite::ToSql,
) -> Result<Option<SmtpAccess>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let changed = transaction
        .execute(
            statement,
            params![identifier.to_string(), value, now_unix()],
        )
        .storage()?;
    if changed == 0 {
        return Ok(None);
    }
    let updated = get_access_blocking(&transaction, identifier)?;
    transaction.commit().storage()?;
    Ok(updated)
}

/// Replace the security rules and read the updated access in the same transaction.
pub(super) fn set_authentication_rules_blocking(
    connection: &mut Connection,
    identifier: SmtpAccessIdentifier,
    requires_encryption: bool,
    plain_allowed: bool,
    login_allowed: bool,
) -> Result<Option<SmtpAccess>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let changed = transaction
    .execute(
        "UPDATE smtp_accesses SET requires_encryption = ?2, plain_mechanism_allowed = ?3, login_mechanism_allowed = ?4, updated_at = ?5 WHERE identifier = ?1",
        params![
            identifier.to_string(),
            requires_encryption,
            plain_allowed,
            login_allowed,
            now_unix()
        ],
    )
    .storage()?;
    if changed == 0 {
        return Ok(None);
    }
    let updated = get_access_blocking(&transaction, identifier)?;
    transaction.commit().storage()?;
    Ok(updated)
}

/// Delete a credential without affecting captured mail.
pub(super) fn delete_access_blocking(
    connection: &Connection,
    identifier: SmtpAccessIdentifier,
) -> Result<bool, StorageError> {
    Ok(connection
        .execute(
            "DELETE FROM smtp_accesses WHERE identifier = ?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0)
}

/// Advance the last successful use with at most one write per minute.
pub(super) fn record_access_use_blocking(
    connection: &Connection,
    identifier: SmtpAccessIdentifier,
    used_at: i64,
) -> Result<(), StorageError> {
    connection.execute(
        "UPDATE smtp_accesses SET last_used_at = ?2 WHERE identifier = ?1 AND (last_used_at IS NULL OR last_used_at <= ?2 - ?3)",
        params![identifier.to_string(), used_at, LAST_USE_RESOLUTION_SECONDS],
    ).storage()?;
    Ok(())
}
