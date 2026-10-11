//! Fresh schema initialization, validation, reopening, and version guard tests.
//!
//! Version 1 is the only supported schema baseline. Older pre-release databases are deliberately
//! rejected because the flattened baseline was prepared before the first deployment.
#[allow(dead_code)]
mod common;

use common::connection as open_connection;
use sandpost_core::{
    GlobalRole, MailAccess, Message, MessageFacts, MessageIdentifier, ScopeIdentifier,
    UserIdentifier,
};
use sandpost_storage::{
    MessageListQuery, MessageStorage, NewUser, ScopeStorage, StorageError, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;
use sqlx::Connection;
use std::path::{Path, PathBuf};

/// Own an isolated temporary data directory for the duration of a test.
struct TestDirectory(PathBuf);

impl TestDirectory {
    /// Create an isolated directory for a database fixture.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sandpost-schema-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// Return the database path inside this test's isolated directory.
    fn database(&self) -> PathBuf {
        self.0.join("sandpost.sqlite3")
    }
}

impl Drop for TestDirectory {
    /// Remove database files and WAL sidecars after the test releases its connections.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Read the stored schema version from a database file.
async fn stored_version(path: &Path) -> i64 {
    let mut connection = open_connection(path).await;
    let version = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    version
}

/// Read migration records in insertion order to check baseline script ordering.
async fn migration_history(connection: &mut sqlx::SqliteConnection) -> Vec<(String, i64)> {
    sqlx::query_as("SELECT migration, batch FROM migrations ORDER BY rowid")
        .fetch_all(connection)
        .await
        .unwrap()
}

/// A fresh database applies each checked-in SQL operation once as schema version 1.
#[tokio::test]
async fn a_fresh_database_applies_the_ordered_baseline() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());

    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut expected: Vec<String> = std::fs::read_dir(migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    expected.sort();

    let mut connection = open_connection(&path).await;
    let actual = migration_history(&mut connection).await;
    assert_eq!(actual.len(), 25);
    assert_eq!(
        actual
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(actual.iter().all(|(_, batch)| *batch == 1));
    assert_eq!(stored_version(&path).await, 1);
    connection.close().await.unwrap();
}

/// Reopening a baseline preserves user and raw message data committed before shutdown.
#[tokio::test]
async fn reopening_an_existing_baseline_preserves_data() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let owner = {
        let storage = SqliteStorage::open(&path).await.unwrap();
        let owner = storage
            .create_first_owner(&NewUser {
                name: "Owner".into(),
                email: "owner@example.test".into(),
                password_hash: "hash".into(),
                global_role: GlobalRole::Owner,
                mail_access: MailAccess::All,
                personal_filter: None,
            })
            .await
            .unwrap();
        let message = Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: "Persisted message".into(),
                ..MessageFacts::default()
            },
            raw_message: vec![0x52, 0x00, 0xff],
            attachments: Vec::new(),
        };
        storage.insert_message(&message).await.unwrap();
        owner.identifier
    };

    let reopened = SqliteStorage::open(&path).await.unwrap();
    let stored_owner = reopened.get_user(owner).await.unwrap().unwrap();
    assert_eq!(stored_owner.global_role, GlobalRole::Owner);
    assert_eq!(reopened.count_users().await.unwrap(), 1);
    let summaries = reopened
        .list_messages(MessageListQuery {
            filter: None,
            before: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(summaries.len(), 1);
    let stored_message = reopened
        .get_message(summaries[0].identifier)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_message.raw_message, [0x52, 0x00, 0xff]);
}

/// A fresh database has no mail sequence row until its first insert.
#[tokio::test]
async fn a_fresh_database_records_no_mail_sequence() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_sequence WHERE name = 'mail'")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    connection.close().await.unwrap();
}

/// An unrelated unversioned database is rejected without adding schema objects.
#[tokio::test]
async fn a_nonempty_unversioned_database_is_rejected_unchanged() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let mut connection = open_connection(&path).await;
    sqlx::raw_sql("CREATE TABLE unrelated (value TEXT);")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    assert_eq!(stored_version(&path).await, 0);
    let mut connection = open_connection(&path).await;
    let objects: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY name",
    )
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(objects, ["unrelated"]);
    connection.close().await.unwrap();
}

/// Newer schema versions return their precise unsupported-version error.
#[tokio::test]
async fn a_newer_schema_version_is_rejected() {
    for version in [2, 5] {
        let directory = TestDirectory::new();
        let path = directory.database();
        drop(SqliteStorage::open(&path).await.unwrap());
        let mut connection = open_connection(&path).await;
        let history_before = migration_history(&mut connection).await;
        let version_statement = match version {
            2 => "PRAGMA user_version = 2",
            5 => "PRAGMA user_version = 5",
            _ => unreachable!("test version is selected from the explicit cases above"),
        };
        sqlx::query(version_statement)
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();

        assert!(matches!(
            SqliteStorage::open(&path).await,
            Err(StorageError::NewerSchema(found)) if found == version
        ));
        assert_eq!(stored_version(&path).await, version);
        let mut connection = open_connection(&path).await;
        assert_eq!(migration_history(&mut connection).await, history_before);
        connection.close().await.unwrap();
    }
}

/// A negative stored schema version is rejected and left unchanged.
#[tokio::test]
async fn a_negative_schema_version_is_rejected() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    sqlx::query("PRAGMA user_version = -1")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    assert_eq!(stored_version(&path).await, -1);
}

/// A version 0 database with migration history is rejected as unsupported.
#[tokio::test]
async fn version_zero_with_history_is_rejected() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    sqlx::query("PRAGMA user_version = 0")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    assert_eq!(stored_version(&path).await, 0);
}

/// A version 1 database without its migration history table is rejected.
#[tokio::test]
async fn version_one_missing_history_is_rejected() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    sqlx::raw_sql("DROP TABLE migrations;")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    assert_eq!(stored_version(&path).await, 1);
}

/// Migration history must match the complete baseline before a database can be reopened.
#[tokio::test]
async fn incomplete_migration_history_is_rejected_without_repair() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    sqlx::query("DELETE FROM migrations WHERE migration = (SELECT migration FROM migrations ORDER BY migration LIMIT 1)")
        .execute(&mut connection)
        .await
        .unwrap();
    let history_before = migration_history(&mut connection).await;
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    let mut connection = open_connection(&path).await;
    assert_eq!(migration_history(&mut connection).await, history_before);
    assert_eq!(stored_version(&path).await, 1);
    connection.close().await.unwrap();
}

/// Same-version schema mismatches in an index or trigger are rejected.
#[tokio::test]
async fn same_version_index_and_trigger_mismatches_are_rejected() {
    for (object, drop_statement) in [
        ("views_owner_name index", "DROP INDEX views_owner_name;"),
        (
            "mail_identifier_immutable trigger",
            "DROP TRIGGER mail_identifier_immutable;",
        ),
    ] {
        let directory = TestDirectory::new();
        let path = directory.database();
        drop(SqliteStorage::open(&path).await.unwrap());
        let mut connection = open_connection(&path).await;
        sqlx::raw_sql(drop_statement)
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();

        assert!(SqliteStorage::open(&path).await.is_err(), "{object}");
        assert_eq!(stored_version(&path).await, 1);
    }
}

/// Foreign keys remain enabled after startup and reject references to missing records.
#[tokio::test]
async fn foreign_keys_are_enforced_after_initialization() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let storage = SqliteStorage::open(&path).await.unwrap();
    assert!(
        storage
            .assign_user_to_scope(UserIdentifier::new(), ScopeIdentifier::new())
            .await
            .is_err(),
        "storage connections reject missing foreign-key parents"
    );
    let mut connection = open_connection(&path).await;
    let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(enabled, 1);
    assert!(
        sqlx::query(
            "INSERT INTO scope_memberships(user_identifier, scope_identifier) VALUES (?1, ?2)"
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(&mut connection)
        .await
        .is_err()
    );
    connection.close().await.unwrap();
    drop(storage);
}

/// Startup rejects an existing orphan row detected by SQLite's foreign-key checker.
#[tokio::test]
async fn a_foreign_key_violation_is_rejected_on_open() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO scope_memberships(user_identifier, scope_identifier) VALUES (?1, ?2)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    assert_eq!(stored_version(&path).await, 1);
}
