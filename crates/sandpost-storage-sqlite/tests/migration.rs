//! Baseline schema and version-guard tests for the flattened SQLite schema.
//!
//! SandPost has no incremental migration history: a fresh database is created directly at the
//! current baseline, and any other stored schema version is rejected rather than migrated.
use rusqlite::Connection;
use sandpost_core::GlobalRole;
use sandpost_storage::{NewUser, Storage};
use sandpost_storage_sqlite::SqliteStorage;
use std::path::PathBuf;
use std::sync::Arc;

/// Own an isolated temporary data directory for the duration of a test.
struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sandpost-schema-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("sandpost.sqlite3")
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Read the stored schema version from a database file.
fn stored_version(path: &std::path::Path) -> i64 {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

/// A fresh database is created directly at the current baseline with its history and seeds.
#[test]
fn fresh_database_is_created_at_the_current_baseline() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).unwrap());

    assert_eq!(stored_version(&path), 1);
    let connection = Connection::open(&path).unwrap();
    let history: Vec<String> = connection
        .prepare("SELECT migration FROM migrations ORDER BY migration")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        history,
        [
            "0001_create_migrations_table.sql",
            "0002_create_endpoints_table.sql",
            "0003_create_users_table.sql",
            "0004_create_scopes_table.sql",
            "0005_create_scope_memberships_table.sql",
            "0006_create_endpoint_memberships_table.sql",
            "0007_create_views_table.sql",
            "0008_create_mail_table.sql",
            "0009_create_mail_recipients_table.sql",
            "0010_create_mail_headers_table.sql",
            "0011_create_mail_attachments_table.sql",
            "0012_create_search_outbox_state_table.sql",
            "0013_create_search_outbox_table.sql",
            "0014_create_mail_immutability_triggers.sql",
            "0015_create_mail_search_outbox_triggers.sql",
            "0016_create_mail_child_fact_triggers.sql",
        ]
    );
    let endpoints: i64 = connection
        .query_row("SELECT COUNT(*) FROM endpoints", [], |row| row.get(0))
        .unwrap();
    assert_eq!(endpoints, 1, "the default endpoint is seeded");
    let outbox_rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM search_outbox_state", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(outbox_rows, 1);
}

/// Reopening an existing baseline validates the schema and preserves committed data.
#[tokio::test]
async fn reopening_an_existing_baseline_preserves_data() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let identifier = {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&path).unwrap());
        let user = storage
            .create_first_owner(&NewUser {
                name: "Owner".into(),
                email: "owner@example.test".into(),
                password_hash: "hash".into(),
                global_role: GlobalRole::Owner,
                personal_filter: None,
            })
            .await
            .unwrap();
        user.identifier
    };
    let reopened: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&path).unwrap());
    let user = reopened.get_user(identifier).await.unwrap().unwrap();
    assert_eq!(user.global_role, GlobalRole::Owner);
    assert_eq!(reopened.count_users().await.unwrap(), 1);
}

/// A newer stored schema version is rejected rather than migrated.
#[test]
fn a_newer_schema_version_is_rejected() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 3)
        .unwrap();
    assert!(SqliteStorage::open(&path).is_err());
}

/// An unversioned database with unrelated objects is rejected instead of being treated as fresh.
#[test]
fn an_unversioned_non_empty_database_is_rejected() {
    let directory = TestDirectory::new();
    let path = directory.database();
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (value TEXT);")
        .unwrap();
    assert!(SqliteStorage::open(&path).is_err());
}

/// A tampered baseline schema is detected and refused on the next open.
#[test]
fn a_tampered_schema_is_rejected() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE views;")
        .unwrap();
    assert!(SqliteStorage::open(&path).is_err());
}
