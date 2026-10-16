//! Ordered SQLite scripts grouped by the execution batch that committed them.
mod catalog;
mod rollback_catalog;

use crate::{StorageError, error::StorageResult, schema};
pub(crate) use catalog::MIGRATIONS;
use rollback_catalog::ROLLBACKS;
use sqlx::{Connection, Row, SqliteConnection};

/// An applied script and the migration execution that committed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedMigration {
    pub migration: String,
    pub batch: i64,
}

/// Initialize an empty database, or validate and upgrade an existing one, atomically.
pub(crate) async fn migrate(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await.storage()?;
    let history = validated_history(&mut transaction).await?;
    upgrade_history(&mut transaction, &history).await?;
    let applied = history.len();
    if applied < MIGRATIONS.len() {
        let batch = history.last().map_or(Ok(1), |record| {
            record
                .batch
                .checked_add(1)
                .ok_or(StorageError::IntegerRange)
        })?;
        for (name, script) in MIGRATIONS.iter().skip(applied) {
            execute_script(&mut transaction, name, script).await?;
            sqlx::query("INSERT INTO migrations (migration, batch) VALUES (?1, ?2)")
                .bind(*name)
                .bind(batch)
                .execute(&mut *transaction)
                .await
                .storage()?;
        }
    }
    validate_schema(&mut transaction, MIGRATIONS.len(), false).await?;
    validate_foreign_keys(&mut transaction).await?;
    transaction.commit().await.storage()?;
    Ok(())
}

/// Read applied batches without applying pending scripts or changing legacy history.
pub(crate) async fn status(
    connection: &mut SqliteConnection,
) -> Result<Vec<AppliedMigration>, StorageError> {
    let mut transaction = connection.begin().await.storage()?;
    let history = validated_history(&mut transaction).await?;
    transaction.commit().await.storage()?;
    Ok(history)
}

/// Reverse only the latest applied batch, validating schema and history in one transaction.
pub(crate) async fn rollback(
    connection: &mut SqliteConnection,
) -> Result<Vec<AppliedMigration>, StorageError> {
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await.storage()?;
    let history = validated_history(&mut transaction).await?;
    let Some(last) = history.last() else {
        transaction.commit().await.storage()?;
        return Ok(Vec::new());
    };
    upgrade_history(&mut transaction, &history).await?;
    let remaining = history.partition_point(|record| record.batch < last.batch);
    let mut reverted = Vec::new();
    for index in (remaining..history.len()).rev() {
        let record = &history[index];
        execute_script(&mut transaction, &record.migration, ROLLBACKS[index]).await?;
        // The first migration removes the history table itself.
        if index != 0 {
            sqlx::query("DELETE FROM migrations WHERE migration = ?1")
                .bind(&record.migration)
                .execute(&mut *transaction)
                .await
                .storage()?;
        }
        reverted.push(record.clone());
    }
    validate_schema(&mut transaction, remaining, false).await?;
    validate_foreign_keys(&mut transaction).await?;
    transaction.commit().await.storage()?;
    Ok(reverted)
}

/// Validate the complete known script prefix, positive ordered batches, and physical schema.
async fn validated_history(
    connection: &mut SqliteConnection,
) -> Result<Vec<AppliedMigration>, StorageError> {
    let has_history: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'migrations')",
    )
    .fetch_one(&mut *connection)
    .await
    .storage()?;
    if !has_history {
        if !schema::schema_is_empty(connection).await? {
            return Err(StorageError::InvalidData(
                "nonempty database is missing migration history".into(),
            ));
        }
        return Ok(Vec::new());
    }
    let legacy = legacy_history(connection).await?;
    let query = if legacy {
        "SELECT migration, 1 AS batch FROM migrations ORDER BY migration"
    } else {
        "SELECT migration, batch FROM migrations ORDER BY migration"
    };
    let history: Vec<AppliedMigration> = sqlx::query(query)
        .try_map(|row: sqlx::sqlite::SqliteRow| {
            Ok(AppliedMigration {
                migration: row.try_get("migration")?,
                batch: row.try_get("batch")?,
            })
        })
        .fetch_all(&mut *connection)
        .await
        .storage()?;
    if history.is_empty()
        || history.len() > MIGRATIONS.len()
        || !history
            .iter()
            .zip(MIGRATIONS)
            .all(|(record, (name, _))| record.migration == name && record.batch > 0)
        || !history
            .windows(2)
            .all(|pair| pair[0].batch <= pair[1].batch)
    {
        return Err(StorageError::InvalidData(
            "migration history contains unknown or missing scripts or invalid batches".into(),
        ));
    }
    validate_schema(connection, history.len(), legacy).await?;
    Ok(history)
}

/// Recognize the supported pre-batch history layout without accepting other schema changes.
async fn legacy_history(connection: &mut SqliteConnection) -> Result<bool, StorageError> {
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('migrations') ORDER BY cid")
            .fetch_all(connection)
            .await
            .storage()?;
    Ok(columns == ["migration"])
}

/// Assign existing name-only records to batch one after validating their original schema.
async fn upgrade_history(
    connection: &mut SqliteConnection,
    history: &[AppliedMigration],
) -> Result<(), StorageError> {
    if !history.is_empty() && legacy_history(connection).await? {
        sqlx::query(
            "ALTER TABLE migrations ADD COLUMN batch INTEGER NOT NULL DEFAULT 1 CHECK (typeof(batch) = 'integer' AND batch > 0)",
        )
        .execute(connection)
        .await
        .storage()?;
    }
    Ok(())
}

/// Execute one checked-in script and retain its name in any migration error.
async fn execute_script(
    connection: &mut SqliteConnection,
    name: &str,
    script: &str,
) -> Result<(), StorageError> {
    sqlx::Executor::execute(connection, sqlx::raw_sql(sqlx::AssertSqlSafe(script)))
        .await
        .map_err(|error| StorageError::Migration(format!("{name}: {error}")))?;
    Ok(())
}

/// Reject schema drift before committing an upgrade or rollback.
async fn validate_schema(
    connection: &mut SqliteConnection,
    applied: usize,
    legacy_history: bool,
) -> Result<(), StorageError> {
    if !schema::matches_schema(connection, applied, legacy_history).await? {
        return Err(StorageError::InvalidData(
            "database schema does not match its migration history".into(),
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests;
