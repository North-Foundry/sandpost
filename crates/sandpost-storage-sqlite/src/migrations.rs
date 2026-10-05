//! The SQLite schema baseline owned by this backend.
//!
//! SandPost has not been deployed, so there is no incremental migration history: a fresh database
//! is created directly at the current physical schema. Any other stored user_version is rejected
//! rather than migrated, and the logical storage model is defined by the sandpost-storage contract
//! rather than by this physical schema.
use crate::{StorageError, error::StorageResult, schema};
use rusqlite::{Connection, TransactionBehavior};

/// Current physical schema version. A flattened baseline: fresh databases are created directly
/// at this version.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// The ordered, checked-in scripts that create the complete current schema.
///
/// Each file holds one schema operation: a table with its indexes, or one trigger group. They are
/// applied in filename order as a single baseline and recorded together in the migration history.
pub(crate) const BASELINE_MIGRATIONS: [(&str, &str); 16] = [
    (
        "0001_create_migrations_table.sql",
        include_str!("../migrations/0001_create_migrations_table.sql"),
    ),
    (
        "0002_create_endpoints_table.sql",
        include_str!("../migrations/0002_create_endpoints_table.sql"),
    ),
    (
        "0003_create_users_table.sql",
        include_str!("../migrations/0003_create_users_table.sql"),
    ),
    (
        "0004_create_scopes_table.sql",
        include_str!("../migrations/0004_create_scopes_table.sql"),
    ),
    (
        "0005_create_scope_memberships_table.sql",
        include_str!("../migrations/0005_create_scope_memberships_table.sql"),
    ),
    (
        "0006_create_endpoint_memberships_table.sql",
        include_str!("../migrations/0006_create_endpoint_memberships_table.sql"),
    ),
    (
        "0007_create_views_table.sql",
        include_str!("../migrations/0007_create_views_table.sql"),
    ),
    (
        "0008_create_mail_table.sql",
        include_str!("../migrations/0008_create_mail_table.sql"),
    ),
    (
        "0009_create_mail_recipients_table.sql",
        include_str!("../migrations/0009_create_mail_recipients_table.sql"),
    ),
    (
        "0010_create_mail_headers_table.sql",
        include_str!("../migrations/0010_create_mail_headers_table.sql"),
    ),
    (
        "0011_create_mail_attachments_table.sql",
        include_str!("../migrations/0011_create_mail_attachments_table.sql"),
    ),
    (
        "0012_create_search_outbox_state_table.sql",
        include_str!("../migrations/0012_create_search_outbox_state_table.sql"),
    ),
    (
        "0013_create_search_outbox_table.sql",
        include_str!("../migrations/0013_create_search_outbox_table.sql"),
    ),
    (
        "0014_create_mail_immutability_triggers.sql",
        include_str!("../migrations/0014_create_mail_immutability_triggers.sql"),
    ),
    (
        "0015_create_mail_search_outbox_triggers.sql",
        include_str!("../migrations/0015_create_mail_search_outbox_triggers.sql"),
    ),
    (
        "0016_create_mail_child_fact_triggers.sql",
        include_str!("../migrations/0016_create_mail_child_fact_triggers.sql"),
    ),
];

/// Initialize an empty database or validate an already-created baseline atomically.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let version: i64 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .storage()?;
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
        if version != SCHEMA_VERSION {
            return Err(StorageError::InvalidData(format!(
                "unsupported schema version {version} (current baseline {SCHEMA_VERSION})"
            )));
        }
        schema::validate_schema(&transaction)?;
        validate_history(&transaction)?;
    } else if version == 0 {
        if !schema::schema_is_empty(&transaction)? {
            return Err(StorageError::InvalidData(
                "unversioned database is not empty".into(),
            ));
        }
        for (_, script) in BASELINE_MIGRATIONS {
            transaction.execute_batch(script).storage()?;
        }
        record_baseline(&transaction)?;
    } else {
        return Err(StorageError::InvalidData(
            "database is missing migration history".into(),
        ));
    }
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .storage()?;
    schema::validate_schema(&transaction)?;
    validate_history(&transaction)?;
    transaction.commit().storage()?;
    Ok(())
}

/// Record the baseline script in the migration history table.
fn record_baseline(connection: &Connection) -> Result<(), StorageError> {
    for (name, _) in BASELINE_MIGRATIONS {
        connection
            .execute(
                "INSERT INTO migrations (migration, batch) VALUES (?1, 1)",
                [name],
            )
            .storage()?;
    }
    Ok(())
}

/// Reject incomplete or unknown migration history rather than silently accepting it.
fn validate_history(connection: &Connection) -> Result<(), StorageError> {
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
    let expected: Vec<_> = BASELINE_MIGRATIONS
        .iter()
        .map(|(name, _)| ((*name).to_owned(), 1))
        .collect();
    if history == expected {
        Ok(())
    } else {
        Err(StorageError::InvalidData(
            "migration history does not match the schema baseline".into(),
        ))
    }
}
