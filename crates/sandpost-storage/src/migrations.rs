//! Ordered table migrations forming schema baseline 1.
use crate::{StorageError, schema};
use rusqlite::{Connection, TransactionBehavior};

pub(crate) const SCHEMA_VERSION: i64 = 1;
pub(crate) const BASELINE_MIGRATIONS: [(&str, &str); 10] = [
    (
        "0001_create_migrations_table.sql",
        include_str!("../migrations/0001_create_migrations_table.sql"),
    ),
    (
        "0002_create_scopes_table.sql",
        include_str!("../migrations/0002_create_scopes_table.sql"),
    ),
    (
        "0003_create_users_table.sql",
        include_str!("../migrations/0003_create_users_table.sql"),
    ),
    (
        "0004_create_memberships_table.sql",
        include_str!("../migrations/0004_create_memberships_table.sql"),
    ),
    (
        "0005_create_inboxes_table.sql",
        include_str!("../migrations/0005_create_inboxes_table.sql"),
    ),
    (
        "0006_create_mail_table.sql",
        include_str!("../migrations/0006_create_mail_table.sql"),
    ),
    (
        "0007_create_mail_recipients_table.sql",
        include_str!("../migrations/0007_create_mail_recipients_table.sql"),
    ),
    (
        "0008_create_mail_headers_table.sql",
        include_str!("../migrations/0008_create_mail_headers_table.sql"),
    ),
    (
        "0009_create_mail_scope_table.sql",
        include_str!("../migrations/0009_create_mail_scope_table.sql"),
    ),
    (
        "0010_create_mail_attachments_table.sql",
        include_str!("../migrations/0010_create_mail_attachments_table.sql"),
    ),
];

/// Initialize an empty database or validate the current baseline in one write transaction.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let has_history: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'migrations')",
        [],
        |row| row.get(0),
    )?;
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
            return Err(StorageError::InvalidData(
                "migration history requires schema version 1".into(),
            ));
        }
        schema::validate_current_schema(&transaction)?;
        validate_history(&transaction)?;
    } else if version == 0 {
        if !schema::schema_is_empty(&transaction)? {
            return Err(StorageError::InvalidData(
                "unversioned database is not empty".into(),
            ));
        }
        for (_, script) in BASELINE_MIGRATIONS {
            transaction.execute_batch(script)?;
        }
        record_baseline(&transaction)?;
    } else {
        return Err(StorageError::InvalidData(
            "database is missing migration history".into(),
        ));
    }
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// Record each baseline file once, including the migration history table itself.
fn record_baseline(connection: &Connection) -> Result<(), StorageError> {
    for (name, _) in BASELINE_MIGRATIONS {
        connection.execute(
            "INSERT INTO migrations (migration, batch) VALUES (?1, 1)",
            [name],
        )?;
    }
    Ok(())
}

/// Reject incomplete, modified, or unknown migration history rather than silently skipping it.
fn validate_history(connection: &Connection) -> Result<(), StorageError> {
    let mut statement =
        connection.prepare("SELECT migration, batch FROM migrations ORDER BY migration")?;
    let history = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let expected: Vec<_> = BASELINE_MIGRATIONS
        .iter()
        .map(|(name, _)| ((*name).to_owned(), 1))
        .collect();
    if history != expected {
        return Err(StorageError::InvalidData(
            "migration history does not match baseline 1".into(),
        ));
    }
    Ok(())
}
