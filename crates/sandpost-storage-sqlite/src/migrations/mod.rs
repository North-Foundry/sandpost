//! The SQLite schema owned by this backend: the version 1 baseline and its forward migrations.
//!
//! Version 1 is a flattened baseline created directly by a fresh database. Later versions are
//! ordered batches of forward migration scripts. A fresh database applies every batch; an existing
//! database is validated against the exact schema of its stored version, then upgraded through the
//! remaining batches inside the same immediate transaction. Unknown, newer, or structurally invalid
//! databases are rejected, and the logical storage model is defined by the sandpost-storage
//! contract rather than by this physical schema.
mod catalog;

use crate::{StorageError, error::StorageResult, schema};
use catalog::{COPY_ENDPOINT_LISTENER_ADDRESS, MIGRATION_BATCHES, MigrationScript, SCHEMA_VERSION};
pub(crate) use catalog::{ENDPOINT_LISTENER_VARIANT, SchemaLayout, batches_through};
use rusqlite::{Connection, TransactionBehavior};

/// Initialize an empty database, or validate and upgrade an existing one, atomically.
///
/// Foreign-key enforcement is switched off for the duration of the transaction, as SQLite
/// requires for table rebuilds, and every reference is verified with `foreign_key_check` before
/// the transaction commits. Enforcement is restored afterwards by the caller's connection setup.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .storage()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let version: i64 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .storage()?;
    let mut layout = SchemaLayout::Canonical;
    let has_history: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'migrations')",
            [],
            |row| row.get(0),
        )
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
        layout = existing_layout(&transaction, version)?;
        validate_history(&transaction, version)?;
    } else if version == 0 {
        if !schema::schema_is_empty(&transaction)? {
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
            transaction
                .execute_batch(script)
                .map_err(|error| StorageError::Migration(format!("{name}: {error}")))?;
            if *name == "0017_create_smtp_server_table.sql"
                && layout == SchemaLayout::EndpointListeners
            {
                transaction
                    .execute_batch(COPY_ENDPOINT_LISTENER_ADDRESS)
                    .map_err(|error| StorageError::Migration(format!("{name}: {error}")))?;
            }
        }
        record_batch(&transaction, *batch, scripts)?;
    }
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .storage()?;
    if !schema::matches_schema(&transaction, SCHEMA_VERSION, SchemaLayout::Canonical)? {
        return Err(StorageError::Migration(
            "migrated schema does not match the current version".into(),
        ));
    }
    validate_history(&transaction, SCHEMA_VERSION)?;
    validate_foreign_keys(&transaction)?;
    transaction.commit().storage()?;
    Ok(())
}

/// Identify the recognized layout of an existing database, rejecting any other structure.
///
/// Only version 1 has an alternative layout; every other version must match its scripts exactly.
fn existing_layout(connection: &Connection, version: i64) -> Result<SchemaLayout, StorageError> {
    if schema::matches_schema(connection, version, SchemaLayout::Canonical)? {
        return Ok(SchemaLayout::Canonical);
    }
    if version == 1 && schema::matches_schema(connection, version, SchemaLayout::EndpointListeners)?
    {
        return Ok(SchemaLayout::EndpointListeners);
    }
    Err(StorageError::InvalidData(format!(
        "database schema does not match schema version {version}"
    )))
}

/// Record one applied batch's scripts in the migration history table.
fn record_batch(
    connection: &Connection,
    batch: i64,
    scripts: &[MigrationScript],
) -> Result<(), StorageError> {
    for (name, _) in scripts {
        connection
            .execute(
                "INSERT INTO migrations (migration, batch) VALUES (?1, ?2)",
                rusqlite::params![name, batch],
            )
            .storage()?;
    }
    Ok(())
}

/// Reject incomplete or unknown migration history for the given schema version.
fn validate_history(connection: &Connection, version: i64) -> Result<(), StorageError> {
    let mut statement = connection
        .prepare("SELECT migration, batch FROM migrations ORDER BY migration")
        .storage()?;
    let history = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .storage()?
        .collect::<Result<Vec<_>, _>>()
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

/// Fail the migration when any row references a missing parent after a table rebuild.
fn validate_foreign_keys(connection: &Connection) -> Result<(), StorageError> {
    let mut statement = connection.prepare("PRAGMA foreign_key_check").storage()?;
    let mut violations = statement.query([]).storage()?;
    if let Some(violation) = violations.next().storage()? {
        let table: String = violation.get(0).storage()?;
        return Err(StorageError::Migration(format!(
            "foreign key violation in table {table}"
        )));
    }
    Ok(())
}
