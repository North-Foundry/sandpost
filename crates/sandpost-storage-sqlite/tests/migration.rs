//! Fresh schema initialization, ordered migration history, reopening, and schema guard tests.
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

/// Read SQLite's user version to verify startup leaves the pragma untouched.
async fn user_version(path: &Path) -> i64 {
    let mut connection = open_connection(path).await;
    let version = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    version
}

/// Read applied migration names in insertion order to check script ordering.
async fn migration_history(connection: &mut sqlx::SqliteConnection) -> Vec<String> {
    sqlx::query_scalar("SELECT migration FROM migrations ORDER BY rowid")
        .fetch_all(connection)
        .await
        .unwrap()
}

/// A fresh database applies all ordered scripts regardless of its initial SQLite user version.
#[tokio::test]
async fn a_fresh_database_ignores_arbitrary_user_version() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let mut connection = open_connection(&path).await;
    sqlx::query("PRAGMA user_version = 847")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    drop(SqliteStorage::open(&path).await.unwrap());

    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut expected: Vec<String> = std::fs::read_dir(&migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    expected.sort();
    for (index, migration_name) in expected.iter().enumerate() {
        assert_eq!(
            migration_name.split('_').next().unwrap(),
            format!("{:04}", index + 1),
            "migration numbering must be consecutive"
        );
        let script = std::fs::read_to_string(migration_directory.join(migration_name)).unwrap();
        assert!(
            !script.to_ascii_uppercase().contains("ALTER TABLE"),
            "forward migration {migration_name} must define the baseline schema directly"
        );
    }
    let users_script =
        std::fs::read_to_string(migration_directory.join("0002_create_users_table.sql")).unwrap();
    assert!(users_script.contains("message_flag_modification"));

    let mut connection = open_connection(&path).await;
    let actual = migration_history(&mut connection).await;
    assert_eq!(expected.len(), 46);
    assert_eq!(actual, expected);
    let legacy_smtp_server_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'smtp_server'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(legacy_smtp_server_table, 0);
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('migrations') ORDER BY cid")
            .fetch_all(&mut connection)
            .await
            .unwrap();
    assert_eq!(columns, ["migration", "batch"]);
    assert_eq!(user_version(&path).await, 847);
    connection.close().await.unwrap();
}

/// Upgrade an early baseline while preserving mail, mailbox UIDs, and message flags.
#[tokio::test]
async fn a_baseline_prefix_upgrade_preserves_imap_state_and_uses_final_table_shapes() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut scripts: Vec<_> = std::fs::read_dir(&migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    scripts.sort();
    assert_eq!(scripts.len(), 46);

    let owner = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    let folder = uuid::Uuid::new_v4().to_string();
    let mailbox = uuid::Uuid::new_v4().to_string();
    let message_sequence: i64;
    {
        let mut connection = open_connection(&path).await;
        for migration_path in scripts.iter().take(25) {
            let name = migration_path.file_name().unwrap().to_string_lossy();
            let script = std::fs::read_to_string(migration_path).unwrap();
            sqlx::raw_sql(sqlx::AssertSqlSafe(script.as_str()))
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO migrations(migration) VALUES (?1)")
                .bind(name.as_ref())
                .execute(&mut connection)
                .await
                .unwrap();
        }

        for (table, expected_columns) in [
            ("users", vec!["message_flag_modification"]),
            ("imap_accounts", vec!["mode", "mode_generation"]),
            (
                "imap_local_folders",
                vec!["kind", "definition_identifier", "organization_mode"],
            ),
            ("imap_mailboxes", vec!["organization_mode"]),
        ] {
            let columns: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                    .bind(table)
                    .fetch_all(&mut connection)
                    .await
                    .unwrap();
            for expected_column in expected_columns {
                assert!(
                    columns.iter().any(|column| column == expected_column),
                    "{table} is missing final baseline column {expected_column}"
                );
            }
        }

        sqlx::query("INSERT INTO users(identifier, name, email, password_hash, global_role, mail_access, created_at, updated_at) VALUES (?1, 'Owner', 'baseline-owner@example.test', 'hash', 'owner', 'all', 1, 1)")
            .bind(&owner).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_accounts(identifier, owner_identifier, name, username, password_hash, created_at, updated_at) VALUES (?1, ?2, 'Account', 'baseline-account', 'hash', 1, 1)")
            .bind(&account).bind(&owner).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES (?1, ?2, 'Projects', 'true', 0, 1, 1)")
            .bind(&folder).bind(&account).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO mail(identifier, subject, text_body, markup_body, raw_message, received_at, size) VALUES ('baseline-mail', 'Persisted baseline mail', 'body', '', X'00', 1, 1)")
            .execute(&mut connection).await.unwrap();
        message_sequence =
            sqlx::query_scalar("SELECT sequence FROM mail WHERE identifier = 'baseline-mail'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        sqlx::query("INSERT INTO imap_mailboxes(identifier, account_identifier, source, folder_identifier, uid_validity, uid_next, recent_through, examined_sequence, definition_fingerprint) VALUES (?1, ?2, 'folder', ?3, 77, 6, 4, ?4, 'baseline-fingerprint')")
            .bind(&mailbox).bind(&account).bind(&folder).bind(message_sequence).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence, deleted, modification) VALUES (?1, 5, ?2, 1, 9)")
            .bind(&mailbox).bind(message_sequence).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_message_flags(account_identifier, mail_sequence, seen, answered, flagged, draft, keywords, modification) VALUES (?1, ?2, 1, 1, 0, 1, '$Work', 77)")
            .bind(&account).bind(message_sequence).execute(&mut connection).await.unwrap();
        connection.close().await.unwrap();
    }

    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    let history = migration_history(&mut connection).await;
    let expected: Vec<String> = scripts
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(history, expected);

    let preserved_membership: (i64, i64, i64, i64, String) = sqlx::query_as(
        "SELECT membership.uid, mailbox.uid_validity, mailbox.uid_next, membership.modification, mailbox.organization_mode FROM imap_mailbox_messages AS membership JOIN imap_mailboxes AS mailbox ON mailbox.identifier = membership.mailbox_identifier WHERE mailbox.identifier = ?",
    )
    .bind(&mailbox)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_membership, (5, 77, 6, 9, "independent".into()));
    let preserved_flags: (i64, i64, i64, i64, String, i64) = sqlx::query_as(
        "SELECT seen, answered, flagged, draft, keywords, modification FROM imap_message_flags WHERE account_identifier = ? AND mail_sequence = ?",
    )
    .bind(&account)
    .bind(message_sequence)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_flags, (1, 1, 0, 1, "$Work".into(), 77));
    let preserved_mail: (String, String, Vec<u8>) =
        sqlx::query_as("SELECT identifier, subject, raw_message FROM mail WHERE sequence = ?")
            .bind(message_sequence)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(
        preserved_mail,
        (
            "baseline-mail".into(),
            "Persisted baseline mail".into(),
            vec![0]
        )
    );
    connection.close().await.unwrap();
}

/// Reopening a migrated database preserves user and raw message data committed before shutdown.
#[tokio::test]
async fn reopening_an_existing_database_preserves_data() {
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

/// An unknown future filename makes migration history invalid without changing the database.
#[tokio::test]
async fn an_unknown_migration_filename_is_rejected_without_repair() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    sqlx::query("INSERT INTO migrations(migration) VALUES ('9999_future.sql')")
        .execute(&mut connection)
        .await
        .unwrap();
    let history_before = migration_history(&mut connection).await;
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    let mut connection = open_connection(&path).await;
    assert_eq!(migration_history(&mut connection).await, history_before);
    connection.close().await.unwrap();
}

/// A missing middle filename is rejected even when every remaining name is known.
#[tokio::test]
async fn a_missing_middle_migration_is_rejected_without_repair() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    sqlx::query("DELETE FROM migrations WHERE migration = '0012_create_search_outbox_table.sql'")
        .execute(&mut connection)
        .await
        .unwrap();
    let history_before = migration_history(&mut connection).await;
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    let mut connection = open_connection(&path).await;
    assert_eq!(migration_history(&mut connection).await, history_before);
    connection.close().await.unwrap();
}

/// An unrelated database with application objects is not mistaken for an empty database.
#[tokio::test]
async fn a_nonempty_unrelated_database_is_rejected_unchanged() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let mut connection = open_connection(&path).await;
    sqlx::raw_sql("CREATE TABLE unrelated (value TEXT);")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
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

/// An existing schema without migration history is rejected instead of being initialized again.
#[tokio::test]
async fn a_database_missing_migration_history_is_rejected() {
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
}

/// The former batch-column history format is rejected without rewriting its records.
#[tokio::test]
async fn an_old_batch_format_history_is_rejected_unchanged() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).await.unwrap());
    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut scripts: Vec<_> = std::fs::read_dir(migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    scripts.sort();

    let mut connection = open_connection(&path).await;
    sqlx::raw_sql(
        "DROP TABLE migrations; CREATE TABLE migrations (migration TEXT PRIMARY KEY NOT NULL, batch INTEGER NOT NULL);",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    for (index, migration_path) in scripts.iter().enumerate() {
        let migration = migration_path.file_name().unwrap().to_string_lossy();
        let batch = if index < 25 {
            1
        } else if index < 27 {
            2
        } else {
            3
        };
        sqlx::query("INSERT INTO migrations(migration, batch) VALUES (?1, ?2)")
            .bind(migration.as_ref())
            .bind(batch)
            .execute(&mut connection)
            .await
            .unwrap();
    }
    let history_before = migration_history(&mut connection).await;
    connection.close().await.unwrap();

    assert!(SqliteStorage::open(&path).await.is_err());
    let mut connection = open_connection(&path).await;
    assert_eq!(migration_history(&mut connection).await, history_before);
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('migrations') ORDER BY cid")
            .fetch_all(&mut connection)
            .await
            .unwrap();
    assert_eq!(columns, ["migration", "batch"]);
    connection.close().await.unwrap();
}

/// A foreign-key violation during upgrade rolls back suffix schema and migration history.
#[tokio::test]
async fn a_foreign_key_violation_rolls_back_the_migration_suffix() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut scripts: Vec<_> = std::fs::read_dir(&migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    scripts.sort();

    {
        let mut connection = open_connection(&path).await;
        for migration_path in scripts.iter().take(24) {
            let name = migration_path.file_name().unwrap().to_string_lossy();
            let script = std::fs::read_to_string(migration_path).unwrap();
            sqlx::raw_sql(sqlx::AssertSqlSafe(script.as_str()))
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO migrations(migration) VALUES (?1)")
                .bind(name.as_ref())
                .execute(&mut connection)
                .await
                .unwrap();
        }
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO imap_mailboxes(identifier, account_identifier, source, uid_validity, uid_next, recent_through, examined_sequence) VALUES ('orphan-mailbox', 'missing-account', 'trash', 1, 1, 0, 0)")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
    }

    assert!(matches!(
        SqliteStorage::open(&path).await,
        Err(StorageError::Migration(_))
    ));
    let mut connection = open_connection(&path).await;
    assert_eq!(migration_history(&mut connection).await.len(), 24);
    for (object_type, object_name) in [
        ("table", "user_message_deletions"),
        ("table", "imap_user_folders"),
        ("table", "imap_account_messages"),
        ("trigger", "user_message_deletions_remove_imap_memberships"),
    ] {
        let object_exists: i64 =
            sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE type = ?1 AND name = ?2")
                .bind(object_type)
                .bind(object_name)
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(
            object_exists, 0,
            "suffix object {object_name} survived rollback"
        );
    }
    let mailbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM imap_mailboxes WHERE identifier = 'orphan-mailbox'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(mailbox_count, 1);
    connection.close().await.unwrap();
}

/// Schema mismatches in an index or trigger are rejected.
#[tokio::test]
async fn same_schema_index_and_trigger_mismatches_are_rejected() {
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
}
