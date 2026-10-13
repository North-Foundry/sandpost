//! Ordered SQLite migrations tracked by applied script names.
//!
//! Existing history must be an exact prefix of the checked-in sequence and its physical schema
//! must match those scripts. All remaining scripts and history records are committed atomically.
mod catalog;

use crate::{StorageError, error::StorageResult, schema};
pub(crate) use catalog::MIGRATIONS;
use sqlx::{Connection, Row, SqliteConnection};

/// Initialize an empty database, or validate and upgrade an existing one, atomically.
pub(crate) async fn migrate(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await.storage()?;
    let has_history: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'migrations')",
    )
    .fetch_one(&mut *transaction)
    .await
    .storage()?;
    let applied = if has_history {
        let applied = applied_migrations(&mut transaction).await?;
        if !schema::matches_schema(&mut transaction, applied).await? {
            return Err(StorageError::InvalidData(
                "database schema does not match its migration history".into(),
            ));
        }
        applied
    } else {
        if !schema::schema_is_empty(&mut transaction).await? {
            return Err(StorageError::InvalidData(
                "nonempty database is missing migration history".into(),
            ));
        }
        0
    };
    for (name, script) in MIGRATIONS.iter().skip(applied) {
        sqlx::Executor::execute(&mut *transaction, sqlx::raw_sql(*script))
            .await
            .map_err(|error| StorageError::Migration(format!("{name}: {error}")))?;
        sqlx::query("INSERT INTO migrations (migration) VALUES (?1)")
            .bind(*name)
            .execute(&mut *transaction)
            .await
            .storage()?;
    }
    if !schema::matches_schema(&mut transaction, MIGRATIONS.len()).await? {
        return Err(StorageError::Migration(
            "migrated schema does not match the checked-in scripts".into(),
        ));
    }
    validate_foreign_keys(&mut transaction).await?;
    transaction.commit().await.storage()?;
    Ok(())
}

/// Count applied scripts only when their names form an exact prefix of the known sequence.
async fn applied_migrations(connection: &mut SqliteConnection) -> Result<usize, StorageError> {
    let history: Vec<String> =
        sqlx::query_scalar("SELECT migration FROM migrations ORDER BY migration")
            .fetch_all(connection)
            .await
            .storage()?;
    if history.len() <= MIGRATIONS.len()
        && history
            .iter()
            .zip(MIGRATIONS)
            .all(|(applied, (name, _))| applied == name)
    {
        Ok(history.len())
    } else {
        Err(StorageError::InvalidData(
            "migration history contains unknown or missing scripts".into(),
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
