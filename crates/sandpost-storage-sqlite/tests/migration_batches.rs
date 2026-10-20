//! Integration coverage for migration batch startup, inspection, and rollback behavior.
#[allow(dead_code)]
mod common;

use common::connection;
use sandpost_storage_sqlite::{AppliedMigration, SqliteStorage};
use sqlx::{Connection, SqliteConnection};
use std::path::{Path, PathBuf};

/// Own an isolated temporary directory for one SQLite database fixture.
struct TestDirectory(PathBuf);

impl TestDirectory {
    /// Create a temporary directory with a process-unique name.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sandpost-migration-batches-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create migration test directory");
        Self(path)
    }

    /// Return the database file path inside this isolated directory.
    fn database(&self) -> PathBuf {
        self.0.join("sandpost.sqlite3")
    }
}

impl Drop for TestDirectory {
    /// Remove the database and its SQLite sidecar files after each test.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Return the checked-in forward migration paths in lexical application order.
fn migration_paths() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .expect("read migration directory")
        .map(|entry| entry.expect("read migration directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    paths.sort();
    paths
}

/// Apply a migration prefix directly and record an optional later batch boundary.
async fn apply_prefix(
    path: &Path,
    migration_count: usize,
    first_batch: i64,
    later_batch: Option<(usize, i64)>,
) {
    let paths = migration_paths();
    assert!(migration_count <= paths.len());
    let mut database = connection(path).await;
    for (index, migration_path) in paths.iter().take(migration_count).enumerate() {
        let name = migration_path
            .file_name()
            .expect("migration filename")
            .to_string_lossy();
        let script = std::fs::read_to_string(migration_path).expect("read migration script");
        sqlx::raw_sql(sqlx::AssertSqlSafe(script.as_str()))
            .execute(&mut database)
            .await
            .unwrap_or_else(|error| panic!("execute migration prefix script {name}: {error}"));
        let batch = later_batch
            .filter(|(first_index, _)| index >= *first_index)
            .map_or(first_batch, |(_, batch)| batch);
        sqlx::query("INSERT INTO migrations (migration, batch) VALUES (?1, ?2)")
            .bind(name.as_ref())
            .bind(batch)
            .execute(&mut database)
            .await
            .expect("record migration fixture");
    }
    database.close().await.expect("close migration fixture");
}

/// Read the names and batch numbers recorded by the database in application order.
async fn recorded_history(database: &mut SqliteConnection) -> Vec<AppliedMigration> {
    sqlx::query_as::<_, (String, i64)>("SELECT migration, batch FROM migrations ORDER BY migration")
        .fetch_all(database)
        .await
        .expect("read fixture migration history")
        .into_iter()
        .map(|(migration, batch)| AppliedMigration { migration, batch })
        .collect()
}

/// Open a database through the production migrator and immediately close its pools.
async fn open_and_close(path: &Path) {
    SqliteStorage::open(path)
        .await
        .expect("open and migrate database")
        .close()
        .await;
}

/// Number newly applied suffixes by startup run and leave a current database unchanged.
#[tokio::test]
async fn opening_batches_suffix_migrations_and_reopening_is_a_no_op() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 12, 1, None).await;

    open_and_close(&path).await;
    let first_history = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(first_history.len(), 53);
    assert!(first_history[..12].iter().all(|record| record.batch == 1));
    assert!(first_history[12..].iter().all(|record| record.batch == 2));

    open_and_close(&path).await;
    assert_eq!(
        SqliteStorage::migration_history(&path).await.unwrap(),
        first_history
    );
}

/// Inspecting a valid prefix reports its history without applying pending migrations.
#[tokio::test]
async fn history_inspection_does_not_upgrade_a_prefix() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 9, 1, None).await;

    let reported = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(reported.len(), 9);
    assert!(reported.iter().all(|record| record.batch == 1));
    let mut database = connection(&path).await;
    assert_eq!(recorded_history(&mut database).await, reported);
    let pending_table_exists: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'user_message_flags'",
    )
    .fetch_one(&mut database)
    .await
    .unwrap();
    assert_eq!(pending_table_exists, 0);
    database.close().await.unwrap();
}

/// Roll back exactly the newest batch in reverse order while retaining earlier rows.
#[tokio::test]
async fn rollback_reverts_only_the_latest_batch_and_reapplication_uses_next_batch() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 20, 1, Some((10, 2))).await;
    let mut database = connection(&path).await;
    sqlx::query("INSERT INTO scopes(identifier, name, filter, position, policy_version) VALUES ('old-scope', 'Older batch', '', 0, 1)")
        .execute(&mut database)
        .await
        .unwrap();
    database.close().await.unwrap();

    let before = SqliteStorage::migration_history(&path).await.unwrap();
    let expected_reverted: Vec<_> = before
        .iter()
        .filter(|record| record.batch == 2)
        .rev()
        .cloned()
        .collect();
    let reverted = SqliteStorage::rollback_last_migration_batch(&path)
        .await
        .unwrap();
    assert_eq!(reverted, expected_reverted);
    let remaining = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(remaining, before[..10]);

    let mut database = connection(&path).await;
    let scope_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scopes WHERE identifier = 'old-scope'")
            .fetch_one(&mut database)
            .await
            .unwrap();
    let removed_table_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'imap_accounts'",
    )
    .fetch_one(&mut database)
    .await
    .unwrap();
    assert_eq!(scope_count, 1);
    assert_eq!(removed_table_count, 0);
    database.close().await.unwrap();

    open_and_close(&path).await;
    let reapplied = SqliteStorage::migration_history(&path).await.unwrap();
    assert!(reapplied[..10].iter().all(|record| record.batch == 1));
    assert!(reapplied[10..].iter().all(|record| record.batch == 2));
}

/// Treat rollback on a database without migration history as an empty operation.
#[tokio::test]
async fn rollback_of_empty_database_is_a_no_op() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let database = connection(&path).await;
    database.close().await.unwrap();

    assert!(
        SqliteStorage::rollback_last_migration_batch(&path)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        SqliteStorage::migration_history(&path)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Serialize simultaneous startup upgrades so every missing script belongs to one batch.
#[tokio::test]
async fn concurrent_upgrades_record_one_shared_batch() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 11, 1, None).await;

    let (first, second) = tokio::join!(SqliteStorage::open(&path), SqliteStorage::open(&path));
    first.unwrap().close().await;
    second.unwrap().close().await;
    let history = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(history.len(), 53);
    assert!(history[..11].iter().all(|record| record.batch == 1));
    assert!(history[11..].iter().all(|record| record.batch == 2));
}

/// Reject a decreasing batch sequence and preserve the invalid fixture exactly as found.
#[tokio::test]
async fn malformed_batch_sequence_is_rejected_without_changes() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 8, 1, Some((4, 2))).await;
    let mut database = connection(&path).await;
    sqlx::query("UPDATE migrations SET batch = 1 WHERE migration = (SELECT migration FROM migrations ORDER BY migration DESC LIMIT 1)")
        .execute(&mut database)
        .await
        .unwrap();
    let before = recorded_history(&mut database).await;
    let schema_objects_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'")
            .fetch_one(&mut database)
            .await
            .unwrap();
    database.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    let mut database = connection(&path).await;
    assert_eq!(recorded_history(&mut database).await, before);
    let schema_objects_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'")
            .fetch_one(&mut database)
            .await
            .unwrap();
    assert_eq!(schema_objects_after, schema_objects_before);
    database.close().await.unwrap();
}

/// Convert supported name-only history in place while preserving its names and existing data.
#[tokio::test]
async fn legacy_name_only_history_is_assigned_batch_one_on_open() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 10, 1, None).await;
    let mut database = connection(&path).await;
    sqlx::query("INSERT INTO scopes(identifier, name, filter, position, policy_version) VALUES ('legacy-scope', 'Legacy record', '', 0, 1)")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE migrations DROP COLUMN batch")
        .execute(&mut database)
        .await
        .unwrap();
    database.close().await.unwrap();

    let legacy_status = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(legacy_status.len(), 10);
    assert!(legacy_status.iter().all(|record| record.batch == 1));
    open_and_close(&path).await;
    let upgraded = SqliteStorage::migration_history(&path).await.unwrap();
    assert!(upgraded[..10].iter().all(|record| record.batch == 1));
    assert!(upgraded[10..].iter().all(|record| record.batch == 2));

    let mut database = connection(&path).await;
    let scope_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scopes WHERE identifier = 'legacy-scope'")
            .fetch_one(&mut database)
            .await
            .unwrap();
    assert_eq!(scope_count, 1);
    database.close().await.unwrap();
}

/// Upgrade fully current legacy history without changing its validated physical schema.
#[tokio::test]
async fn fully_applied_legacy_history_remains_batch_one_after_open() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 47, 1, None).await;
    let mut database = connection(&path).await;
    sqlx::query("ALTER TABLE migrations DROP COLUMN batch")
        .execute(&mut database)
        .await
        .unwrap();
    let legacy_script: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'migrations'",
    )
    .fetch_one(&mut database)
    .await
    .unwrap();
    database.close().await.unwrap();

    let before = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(before.len(), 47);
    assert!(before.iter().all(|record| record.batch == 1));
    open_and_close(&path).await;
    let after = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(after.len(), 53);
    assert_eq!(&after[..47], before.as_slice());
    assert_eq!(
        after[47].migration,
        "0048_promote_admin_users_to_owners.sql"
    );
    assert!(after[47..].iter().all(|record| record.batch == 2));

    let mut database = connection(&path).await;
    let current_script: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'migrations'",
    )
    .fetch_one(&mut database)
    .await
    .unwrap();
    assert_ne!(current_script, legacy_script);
    let batch_column_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pragma_table_info('migrations') WHERE name = 'batch' AND type = 'INTEGER'",
    )
    .fetch_one(&mut database)
    .await
    .unwrap();
    assert_eq!(batch_column_count, 1);
    database.close().await.unwrap();
}

/// Roll back a populated schema containing both Mirror and Independent folders.
#[tokio::test]
async fn rollback_succeeds_with_mode_separated_mailboxes_and_folders() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 47, 1, None).await;
    let mut database = connection(&path).await;
    sqlx::query("INSERT INTO users(identifier, name, email, password_hash, global_role, mail_access, created_at, updated_at) VALUES ('rollback-owner', 'Owner', 'rollback@example.test', 'hash', 'member', 'all', 1, 1)")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO imap_accounts(identifier, owner_identifier, name, username, password_hash, created_at, updated_at, mode) VALUES ('mirror-account', 'rollback-owner', 'Mirror account', 'mirror-account', 'hash', 1, 1, 'mirror'), ('independent-account', 'rollback-owner', 'Independent account', 'independent-account', 'hash', 1, 1, 'independent')")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO imap_user_folders(identifier, owner_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES ('mirror-definition', 'rollback-owner', 'Mirror folder', '', 0, 1, 1)")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at, kind, definition_identifier, organization_mode) VALUES ('mirror-folder', 'mirror-account', '__sandpost_owner_folder_mirror-definition', '', 0, 1, 1, 'smart', 'mirror-definition', 'mirror'), ('independent-folder', 'independent-account', 'Independent folder', '', 0, 1, 1, 'private', NULL, 'independent')")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO imap_user_folder_accounts(account_identifier, folder_identifier, local_folder_identifier) VALUES ('mirror-account', 'mirror-definition', 'mirror-folder')")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO imap_mailboxes(identifier, account_identifier, source, folder_identifier, uid_validity, definition_fingerprint, organization_mode) VALUES ('mirror-mailbox', 'mirror-account', 'folder', 'mirror-folder', 1, 'mirror-fingerprint', 'mirror'), ('independent-mailbox', 'independent-account', 'folder', 'independent-folder', 2, 'independent-fingerprint', 'independent')")
        .execute(&mut database)
        .await
        .unwrap();
    let before = recorded_history(&mut database).await;
    database.close().await.unwrap();

    let reverted = SqliteStorage::rollback_last_migration_batch(&path)
        .await
        .unwrap();
    assert_eq!(reverted, before.iter().rev().cloned().collect::<Vec<_>>());
    assert!(
        SqliteStorage::migration_history(&path)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Reject an unexpected schema object without modifying migration history or that object.
#[tokio::test]
async fn invalid_schema_is_rejected_without_changes() {
    let directory = TestDirectory::new();
    let path = directory.database();
    apply_prefix(&path, 10, 1, None).await;
    let mut database = connection(&path).await;
    sqlx::query("CREATE TABLE unexpected_fixture (value TEXT NOT NULL)")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO unexpected_fixture(value) VALUES ('preserved')")
        .execute(&mut database)
        .await
        .unwrap();
    let before = recorded_history(&mut database).await;
    database.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    let mut database = connection(&path).await;
    assert_eq!(recorded_history(&mut database).await, before);
    let preserved_value: String = sqlx::query_scalar("SELECT value FROM unexpected_fixture")
        .fetch_one(&mut database)
        .await
        .unwrap();
    assert_eq!(preserved_value, "preserved");
    database.close().await.unwrap();
}

/// Roll back the complete fresh batch, then initialize the now-empty database again.
#[tokio::test]
async fn rolling_back_fresh_database_allows_full_reinitialization() {
    let directory = TestDirectory::new();
    let path = directory.database();
    open_and_close(&path).await;
    let initial = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(initial.len(), 53);
    assert!(initial.iter().all(|record| record.batch == 1));
    open_and_close(&path).await;
    assert_eq!(
        SqliteStorage::migration_history(&path).await.unwrap(),
        initial
    );

    let reverted = SqliteStorage::rollback_last_migration_batch(&path)
        .await
        .unwrap();
    assert_eq!(reverted, initial.iter().rev().cloned().collect::<Vec<_>>());
    assert!(
        SqliteStorage::migration_history(&path)
            .await
            .unwrap()
            .is_empty()
    );

    open_and_close(&path).await;
    let reinitialized = SqliteStorage::migration_history(&path).await.unwrap();
    assert_eq!(reinitialized.len(), 53);
    assert!(reinitialized.iter().all(|record| record.batch == 1));
}
