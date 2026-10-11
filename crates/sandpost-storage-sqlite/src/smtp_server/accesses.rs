//! Hashed SMTP credentials, mechanism policy, and successful-use metadata.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use sandpost_core::{SmtpAccess, SmtpAccessIdentifier, SmtpAuthenticationMechanism};
use sandpost_storage::{NewSmtpAccess, SmtpAccessCredential, StorageError};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

/// Named public access columns; the hash is selected separately.
const ACCESS_COLUMNS: &str = "identifier,name,username,enabled,created_at,last_used_at,requires_encryption,plain_mechanism_allowed,login_mechanism_allowed";
/// Seconds within which a repeated successful authentication does not rewrite `last_used_at`.
const LAST_USE_RESOLUTION_SECONDS: i64 = 60;

/// Current Unix time in seconds for the updated timestamp.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Decode the public access columns selected for an access row.
fn decode_access(row: &SqliteRow) -> Result<SmtpAccess, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    Ok(SmtpAccess {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        name: row.try_get("name")?,
        username: row.try_get("username")?,
        enabled: row.try_get("enabled")?,
        created_at: row.try_get("created_at")?,
        last_used_at: row.try_get("last_used_at")?,
        requires_encryption: row.try_get("requires_encryption")?,
        allowed_mechanisms: allowed_mechanisms(
            row.try_get("plain_mechanism_allowed")?,
            row.try_get("login_mechanism_allowed")?,
        ),
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

/// Decode an access and its selected password hash column.
fn decode_credential(row: &SqliteRow) -> Result<SmtpAccessCredential, sqlx::Error> {
    Ok(SmtpAccessCredential {
        access: decode_access(row)?,
        password_hash: row.try_get("password_hash")?,
    })
}

/// List every access by name and identifier without password hashes.
pub(super) async fn list_accesses(
    storage: &SqliteStorage,
) -> Result<Vec<SmtpAccess>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCESS_COLUMNS} FROM smtp_accesses ORDER BY name, identifier"
    )))
    .fetch_all(&storage.read_pool)
    .await
    .storage()?
    .iter()
    .map(decode_access)
    .collect::<Result<Vec<_>, _>>()
    .storage()
}

/// Load one access by identifier without its password hash.
pub(super) async fn get_access(
    storage: &SqliteStorage,
    identifier: SmtpAccessIdentifier,
) -> Result<Option<SmtpAccess>, StorageError> {
    let mut connection = storage.read_pool.acquire().await.storage()?;
    get_access_from_connection(&mut connection, identifier).await
}

/// Load one access row from an existing connection.
async fn get_access_from_connection(
    connection: &mut SqliteConnection,
    identifier: SmtpAccessIdentifier,
) -> Result<Option<SmtpAccess>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCESS_COLUMNS} FROM smtp_accesses WHERE identifier = ?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .as_ref()
    .map(decode_access)
    .transpose()
    .storage()
}

/// Load one access and its password hash by a column that identifies at most one row.
pub(super) async fn get_credential(
    connection: &mut SqliteConnection,
    column: &str,
    value: &str,
) -> Result<Option<SmtpAccessCredential>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCESS_COLUMNS},password_hash FROM smtp_accesses WHERE {column} = ?1"
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

/// Reserve the writer before checking username uniqueness across independent connections.
/// An immediate transaction avoids upgrading a stale WAL read snapshot after another creator wins.
pub(super) async fn create_access(
    storage: &SqliteStorage,
    access: &NewSmtpAccess,
) -> Result<SmtpAccess, StorageError> {
    let (plain_allowed, login_allowed) = mechanism_flags(&access.allowed_mechanisms)?;
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let duplicate: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM smtp_accesses WHERE username = ?1)")
            .bind(&access.username)
            .fetch_one(&mut *transaction)
            .await
            .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateSmtpUsername);
    }
    let identifier = SmtpAccessIdentifier::new();
    sqlx::query(
        "INSERT INTO smtp_accesses(identifier,name,username,password_hash,enabled,created_at,updated_at,requires_encryption,plain_mechanism_allowed,login_mechanism_allowed) VALUES (?1,?2,?3,?4,1,?5,?6,?7,?8,?9)",
    )
    .bind(identifier.to_string())
    .bind(&access.name)
    .bind(&access.username)
    .bind(&access.password_hash)
    .bind(access.created_at)
    .bind(now_unix())
    .bind(access.requires_encryption)
    .bind(plain_allowed)
    .bind(login_allowed)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let created = get_access_from_connection(&mut transaction, identifier)
        .await?
        .ok_or(StorageError::NotFound)?;
    transaction.commit().await.storage()?;
    Ok(created)
}

/// Value written by one SMTP access update statement.
pub(super) enum AccessUpdateValue<'a> {
    Password(&'a str),
    Enabled(bool),
}

/// Run one single-row update and return the access as stored afterwards.
pub(super) async fn update_access(
    storage: &SqliteStorage,
    identifier: SmtpAccessIdentifier,
    statement: &'static str,
    value: AccessUpdateValue<'_>,
) -> Result<Option<SmtpAccess>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let changed = match value {
        AccessUpdateValue::Password(value) => sqlx::query(statement)
            .bind(identifier.to_string())
            .bind(value)
            .bind(now_unix())
            .execute(&mut *transaction)
            .await
            .storage()?,
        AccessUpdateValue::Enabled(value) => sqlx::query(statement)
            .bind(identifier.to_string())
            .bind(value)
            .bind(now_unix())
            .execute(&mut *transaction)
            .await
            .storage()?,
    }
    .rows_affected();
    if changed == 0 {
        return Ok(None);
    }
    let updated = get_access_from_connection(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(updated)
}

/// Replace the security rules and read the updated access in the same transaction.
pub(super) async fn set_authentication_rules(
    storage: &SqliteStorage,
    identifier: SmtpAccessIdentifier,
    requires_encryption: bool,
    plain_allowed: bool,
    login_allowed: bool,
) -> Result<Option<SmtpAccess>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let changed = sqlx::query(
        "UPDATE smtp_accesses SET requires_encryption = ?2, plain_mechanism_allowed = ?3, login_mechanism_allowed = ?4, updated_at = ?5 WHERE identifier = ?1",
    )
    .bind(identifier.to_string())
    .bind(requires_encryption)
    .bind(plain_allowed)
    .bind(login_allowed)
    .bind(now_unix())
    .execute(&mut *transaction)
    .await
    .storage()?
    .rows_affected();
    if changed == 0 {
        return Ok(None);
    }
    let updated = get_access_from_connection(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(updated)
}

/// Delete a credential without affecting captured mail.
pub(super) async fn delete_access(
    storage: &SqliteStorage,
    identifier: SmtpAccessIdentifier,
) -> Result<bool, StorageError> {
    Ok(
        sqlx::query("DELETE FROM smtp_accesses WHERE identifier = ?1")
            .bind(identifier.to_string())
            .execute(&storage.write_pool)
            .await
            .storage()?
            .rows_affected()
            > 0,
    )
}

/// Advance the last successful use with at most one write per minute.
pub(super) async fn record_access_use(
    storage: &SqliteStorage,
    identifier: SmtpAccessIdentifier,
    used_at: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE smtp_accesses SET last_used_at = ?2 WHERE identifier = ?1 AND (last_used_at IS NULL OR last_used_at <= ?2 - ?3)",
    )
    .bind(identifier.to_string())
    .bind(used_at)
    .bind(LAST_USE_RESOLUTION_SECONDS)
    .execute(&storage.write_pool)
    .await
    .storage()?;
    Ok(())
}
