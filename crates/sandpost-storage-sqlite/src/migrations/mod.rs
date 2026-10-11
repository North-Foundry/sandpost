//! The SQLite schema baseline and its ordered forward migration batches.
//!
//! Version 1 creates the current schema directly, one table or trigger group per script.
//! Existing databases must match their version's schema and history exactly before any remaining
//! batches are applied atomically. Pre-release schemas from before this baseline are unsupported.
mod catalog;

use crate::{StorageError, error::StorageResult, schema};
pub(crate) use catalog::batches_through;
use catalog::{MIGRATION_BATCHES, MigrationScript, SCHEMA_VERSION};
use sqlx::{Connection, Row, SqliteConnection};

/// Initialize an empty database, or validate and upgrade an existing one, atomically.
pub(crate) async fn migrate(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await.storage()?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *transaction)
        .await
        .storage()?;
    let has_history: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'migrations')")
            .fetch_one(&mut *transaction)
            .await
            .storage()?;
    if version < 0 {
        return Err(StorageError::InvalidData(format!(
            "negative schema version {version}"
        )));
    }
    if version > SCHEMA_VERSION {
        return Err(StorageError::NewerSchema(version));
    }
    if has_history {
        if version < 1 {
            return Err(StorageError::InvalidData(format!(
                "unsupported schema version {version} (current {SCHEMA_VERSION})"
            )));
        }
        if !schema::matches_schema(&mut transaction, version).await? {
            return Err(StorageError::InvalidData(format!(
                "database schema does not match schema version {version}"
            )));
        }
        validate_history(&mut transaction, version).await?;
    } else if version == 0 {
        if !schema::schema_is_empty(&mut transaction).await? {
            return Err(StorageError::InvalidData(
                "unversioned database is not empty".into(),
            ));
        }
    } else {
        return Err(StorageError::InvalidData(
            "database is missing migration history".into(),
        ));
    }
    for (batch, scripts) in MIGRATION_BATCHES
        .iter()
        .filter(|(batch, _)| *batch > version)
    {
        for (name, script) in scripts.iter() {
            // Execute the immutable migration script through the startup transaction.
            sqlx::Executor::execute(&mut *transaction, sqlx::raw_sql(*script))
                .await
                .map_err(|error| StorageError::Migration(format!("{name}: {error}")))?;
        }
        record_batch(&mut transaction, *batch, scripts).await?;
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA user_version={SCHEMA_VERSION}"
    )))
    .execute(&mut *transaction)
    .await
    .storage()?;
    if !schema::matches_schema(&mut transaction, SCHEMA_VERSION).await? {
        return Err(StorageError::Migration(
            "migrated schema does not match the current version".into(),
        ));
    }
    validate_history(&mut transaction, SCHEMA_VERSION).await?;
    validate_foreign_keys(&mut transaction).await?;
    transaction.commit().await.storage()?;
    Ok(())
}

/// Record one applied batch's scripts in the migration history table.
async fn record_batch(
    connection: &mut SqliteConnection,
    batch: i64,
    scripts: &[MigrationScript],
) -> Result<(), StorageError> {
    for (name, _) in scripts {
        sqlx::query("INSERT INTO migrations (migration, batch) VALUES (?1, ?2)")
            .bind(*name)
            .bind(batch)
            .execute(&mut *connection)
            .await
            .storage()?;
    }
    Ok(())
}

/// Reject incomplete or unknown migration history for the given schema version.
async fn validate_history(
    connection: &mut SqliteConnection,
    version: i64,
) -> Result<(), StorageError> {
    let history = sqlx::query("SELECT migration, batch FROM migrations ORDER BY migration")
        .try_map(|row: sqlx::sqlite::SqliteRow| {
            Ok((
                row.try_get::<String, _>("migration")?,
                row.try_get::<i64, _>("batch")?,
            ))
        })
        .fetch_all(connection)
        .await
        .storage()?;
    let expected: Vec<_> = batches_through(version)
        .flat_map(|(batch, scripts)| {
            scripts
                .iter()
                .map(move |(name, _)| ((*name).to_owned(), *batch))
        })
        .collect();
    if history == expected {
        Ok(())
    } else {
        Err(StorageError::InvalidData(
            "migration history does not match the schema version".into(),
        ))
    }
}

/// Reject any database containing a row that references a missing parent.
async fn validate_foreign_keys(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    if let Some(violation) = sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(connection)
        .await
        .storage()?
    {
        let table: String = violation.try_get("table").storage()?;
        return Err(StorageError::Migration(format!(
            "foreign key violation in table {table}"
        )));
    }
    Ok(())
}
