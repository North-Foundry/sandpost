//! Pre-deployment schema rejection and file-backed storage compatibility tests.
use sandpost_storage_sqlite::SqliteStorage;
use sqlx::{ConnectOptions, Connection, SqliteConnection, sqlite::SqliteConnectOptions};

/// The pre-deployment database facts that must survive a rejected open unchanged.
#[derive(Debug, PartialEq, Eq)]
struct DatabaseSnapshot {
    version: i64,
    schema_objects: Vec<(String, String)>,
    migration_history: Vec<(String, i64)>,
    mail_count: i64,
    owner_count: i64,
}

/// Read immutable schema, history, version, and row counts from a database file.
async fn database_snapshot(connection: &mut SqliteConnection) -> DatabaseSnapshot {
    let version = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    let objects = sqlx::query_as(
        "SELECT name,sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY name",
    )
    .fetch_all(&mut *connection)
    .await
    .unwrap();
    let history = sqlx::query_as("SELECT migration,batch FROM migrations ORDER BY migration")
        .fetch_all(&mut *connection)
        .await
        .unwrap();
    let mail_count = sqlx::query_scalar("SELECT count(*) FROM mail")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    let owner_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE email = 'owner@example.test' AND global_role = 'owner'",
    )
    .fetch_one(connection)
    .await
    .unwrap();
    DatabaseSnapshot {
        version,
        schema_objects: objects,
        migration_history: history,
        mail_count,
        owner_count,
    }
}

/// The frozen pre-deployment version 5 database is rejected without schema or data changes.
#[tokio::test]
async fn rejects_legacy_database_without_mutating_it() {
    let directory = std::env::temp_dir().join(format!(
        "sandpost-legacy-rejection-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("mail.sqlite3");
    std::fs::write(
        &database,
        include_bytes!("fixtures/pre-sqlx-rusqlite.sqlite3"),
    )
    .unwrap();

    let mut connection = SqliteConnectOptions::new()
        .filename(&database)
        .read_only(true)
        .connect()
        .await
        .unwrap();
    let before = database_snapshot(&mut connection).await;
    assert_eq!(before.version, 5);
    assert_eq!(before.migration_history.len(), 35);
    assert_eq!(before.mail_count, 12);
    assert_eq!(before.owner_count, 1);
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&database).await.is_err());

    let mut connection = SqliteConnectOptions::new()
        .filename(&database)
        .read_only(true)
        .connect()
        .await
        .unwrap();
    assert_eq!(database_snapshot(&mut connection).await, before);
    connection.close().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

/// The file-backed storage pools continue to satisfy the shared storage contract.
#[tokio::test]
async fn file_backed_pools_satisfy_storage_conformance() {
    let directory = std::env::temp_dir().join(format!(
        "sandpost-file-conformance-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let storage = SqliteStorage::open(directory.join("mail.sqlite3"))
        .await
        .unwrap();
    sandpost_storage_conformance::run_all(&storage)
        .await
        .unwrap();
    storage.close().await;
    std::fs::remove_dir_all(directory).unwrap();
}
