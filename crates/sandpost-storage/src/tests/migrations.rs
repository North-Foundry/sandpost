//! Regression coverage for baseline migration creation and schema validation.
use super::message;
use crate::{
    Storage, StorageError,
    migrations::{BASELINE_MIGRATIONS, SCHEMA_VERSION, migrate},
};
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

/// Apply the checked-in baseline scripts and record the current schema version.
fn create_baseline(connection: &mut Connection) {
    migrate(connection).expect("baseline migration should succeed");
}

/// Return SQL-visible user schema definitions for before-and-after rejection checks.
fn schema_definitions(connection: &Connection) -> Vec<(String, String, String, Option<String>)> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name",
        )
        .expect("schema query should prepare");
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("schema query should run")
        .collect::<Result<_, _>>()
        .expect("schema rows should decode")
}

/// Return recorded migration names and batches when a history table exists.
fn migration_history(connection: &Connection) -> Option<Vec<(String, i64)>> {
    let history_table_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'migrations')",
            [],
            |row| row.get(0),
        )
        .expect("migration table lookup should succeed");
    history_table_exists.then(|| {
        connection
            .prepare("SELECT migration, batch FROM migrations ORDER BY migration")
            .expect("migration history query should prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("migration history query should run")
            .collect::<Result<_, _>>()
            .expect("migration history should decode")
    })
}

/// Reject a migration and verify its transaction leaves schema, history, and version unchanged.
fn assert_migration_rolls_back(connection: &mut Connection) -> StorageError {
    let schema_before = schema_definitions(connection);
    let history_before = migration_history(connection);
    let version_before: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("schema version should be readable");

    let migration_error = migrate(connection).expect_err("invalid migration input should fail");

    assert_eq!(
        schema_definitions(connection),
        schema_before,
        "failed migration should roll back schema changes"
    );
    assert_eq!(migration_history(connection), history_before);
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .expect("schema version should remain readable"),
        version_before
    );
    migration_error
}

/// Fresh migration creates the exact nine-entry batch-one baseline and version one.
#[test]
fn fresh_database_records_all_baseline_migrations_once() {
    let mut connection = Connection::open_in_memory().unwrap();
    create_baseline(&mut connection);

    let history = migration_history(&connection).unwrap();
    let mut expected: Vec<_> = BASELINE_MIGRATIONS
        .iter()
        .map(|(name, _)| ((*name).to_owned(), 1))
        .collect();
    expected.sort();
    assert_eq!(history, expected);
    assert_eq!(history.len(), 9);
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        SCHEMA_VERSION
    );
}

/// Reapplying baseline migration is idempotent and does not duplicate migration history.
#[test]
fn baseline_migration_is_idempotent() {
    let mut connection = Connection::open_in_memory().unwrap();
    create_baseline(&mut connection);
    let schema_before = schema_definitions(&connection);
    let history_before = migration_history(&connection);

    migrate(&mut connection).unwrap();

    assert_eq!(schema_definitions(&connection), schema_before);
    assert_eq!(migration_history(&connection), history_before);
}

/// Reject populated version-zero databases without modifying their existing objects.
#[test]
fn rejects_populated_version_zero_database() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE foreign_data (value TEXT);")
        .unwrap();
    assert!(matches!(
        assert_migration_rolls_back(&mut connection),
        StorageError::InvalidData(_)
    ));
}

/// Reject supported version one without migration history and preserve the database.
#[test]
fn rejects_version_one_without_migration_history() {
    let mut connection = Connection::open_in_memory().unwrap();
    create_baseline(&mut connection);
    connection.execute_batch("DROP TABLE migrations;").unwrap();
    assert!(matches!(
        assert_migration_rolls_back(&mut connection),
        StorageError::InvalidData(_)
    ));
}

/// Reject version two without history and preserve the otherwise empty database.
#[test]
fn rejects_version_two_without_migration_history() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    assert!(matches!(
        assert_migration_rolls_back(&mut connection),
        StorageError::NewerSchema(2)
    ));
}

/// Reject schema version two when migration history already exists.
#[test]
fn rejects_version_two_with_migration_history() {
    let mut connection = Connection::open_in_memory().unwrap();
    create_baseline(&mut connection);
    connection.pragma_update(None, "user_version", 2).unwrap();
    assert!(matches!(
        assert_migration_rolls_back(&mut connection),
        StorageError::NewerSchema(2)
    ));
}

/// Reject unsupported future and negative versions without modifying the database.
#[test]
fn rejects_unknown_and_negative_schema_versions() {
    for schema_version in [3, -1] {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "user_version", schema_version)
            .unwrap();
        assert_migration_rolls_back(&mut connection);
    }
}

/// Reject incomplete and altered migration histories without modifying schema or version.
#[test]
fn rejects_incomplete_or_modified_migration_history() {
    for history_change in [
        "DELETE FROM migrations WHERE migration = '0009_create_message_scope_table.sql'",
        "UPDATE migrations SET batch = 2 WHERE migration = '0001_create_migrations_table.sql'",
        "UPDATE migrations SET migration = 'unknown.sql' WHERE migration = '0001_create_migrations_table.sql'",
    ] {
        let mut connection = Connection::open_in_memory().unwrap();
        create_baseline(&mut connection);
        connection.execute(history_change, []).unwrap();
        assert_migration_rolls_back(&mut connection);
    }
}

/// Reject a current schema whose columns, constraints, indexes, triggers, or SQL literals drift.
#[test]
fn rejects_malformed_current_schemas() {
    let changes = [
        (
            "ALTER TABLE messages DROP COLUMN subject",
            "partial summary column set",
        ),
        (
            "CREATE TABLE scopes_copy AS SELECT * FROM scopes; DROP TABLE scopes; ALTER TABLE scopes_copy RENAME TO scopes",
            "missing scope constraints and index",
        ),
        ("DROP INDEX messages_received", "missing required index"),
        (
            "CREATE TRIGGER extra_message_trigger AFTER INSERT ON messages BEGIN SELECT 1; END",
            "additional trigger",
        ),
        (
            "PRAGMA writable_schema = ON; UPDATE sqlite_schema SET sql = replace(sql, '''[]''', '''[ ]''') WHERE type = 'table' AND name = 'messages'; PRAGMA schema_version = 999; PRAGMA writable_schema = OFF",
            "changed quoted literal",
        ),
        (
            "PRAGMA writable_schema = ON; UPDATE sqlite_schema SET sql = replace(sql, 'subject TEXT NOT NULL', 'subject TEXTNOTNULL') WHERE type = 'table' AND name = 'messages'; PRAGMA schema_version = 999; PRAGMA writable_schema = OFF",
            "merged SQL keywords",
        ),
    ];
    for (schema_change, description) in changes {
        let mut connection = Connection::open_in_memory().unwrap();
        create_baseline(&mut connection);
        connection
            .execute_batch(schema_change)
            .unwrap_or_else(|error| panic!("{description} mutation should apply: {error}"));
        assert_migration_rolls_back(&mut connection);
    }
}

/// Open a fresh database twice and verify each open preserves migration history and data.
#[test]
fn persistent_database_reopens_with_stable_migration_history() {
    let database_path = temporary_database_path("reopen");
    let first_storage = Storage::open(&database_path).unwrap();
    first_storage.health().unwrap();
    let first_message = message();
    assert_eq!(
        first_storage.insert_message(&first_message, &[]).unwrap(),
        sandpost_core::MessageSequence(1)
    );
    drop(first_storage);

    let second_storage = Storage::open(&database_path).unwrap();
    let second_message = message();
    assert_eq!(
        second_storage.insert_message(&second_message, &[]).unwrap(),
        sandpost_core::MessageSequence(2)
    );
    let connection = second_storage.connection().unwrap();
    assert_eq!(migration_history(&connection).unwrap().len(), 9);
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        1
    );
    drop(connection);
    drop(second_storage);
    remove_temporary_database(&database_path);
}

/// Concurrently initialize one empty file from two threads and verify one baseline history.
#[test]
fn concurrent_open_of_empty_database_records_one_baseline() {
    let database_path = temporary_database_path("concurrent");
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let worker_path = database_path.clone();
            let worker_barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                worker_barrier.wait();
                let storage = Storage::open(worker_path).unwrap();
                storage.health().unwrap();
                let connection = storage.connection().unwrap();
                migration_history(&connection).unwrap().len()
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), 9);
    }
    remove_temporary_database(&database_path);
}

/// Construct a unique temporary database path using only the standard library.
fn temporary_database_path(label: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sandpost-migrations-{label}-{}-{timestamp}.sqlite",
        std::process::id()
    ))
}

/// Remove a temporary database and SQLite sidecar files after a persistent test.
fn remove_temporary_database(database_path: &PathBuf) {
    let _ = fs::remove_file(database_path);
    let mut sidecar_path = database_path.as_os_str().to_os_string();
    sidecar_path.push("-wal");
    let _ = fs::remove_file(PathBuf::from(sidecar_path));
    let mut sidecar_path = database_path.as_os_str().to_os_string();
    sidecar_path.push("-shm");
    let _ = fs::remove_file(PathBuf::from(sidecar_path));
}
