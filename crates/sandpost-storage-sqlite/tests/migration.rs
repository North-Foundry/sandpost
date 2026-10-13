//! Fresh schema initialization, ordered migration history, reopening, and schema guard tests.
#[allow(dead_code)]
mod common;

use common::connection as open_connection;
use sandpost_core::{
    GlobalRole, ImapAccountIdentifier, MailAccess, Message, MessageFacts, MessageIdentifier,
    ScopeIdentifier, UserIdentifier,
};
use sandpost_storage::{
    ImapStorage, MessageListQuery, MessageStorage, NewImapAccount, NewUser, ScopeStorage,
    StorageError, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;
use sqlx::Connection;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

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
    let mut expected: Vec<String> = std::fs::read_dir(migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    expected.sort();

    let mut connection = open_connection(&path).await;
    let actual = migration_history(&mut connection).await;
    assert_eq!(expected.len(), 39);
    assert_eq!(actual, expected);
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('migrations') ORDER BY cid")
            .fetch_all(&mut connection)
            .await
            .unwrap();
    assert_eq!(columns, ["migration"]);
    assert_eq!(user_version(&path).await, 847);
    connection.close().await.unwrap();
}

/// Upgrade the first 25 scripts while preserving existing folders, mail, and mailbox state.
#[tokio::test]
async fn first_25_scripts_migrate_forward_without_losing_imap_state() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let mut connection = open_connection(&path).await;
    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut initial_scripts: Vec<_> = std::fs::read_dir(&migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    initial_scripts.sort();
    initial_scripts.truncate(25);
    for migration_path in initial_scripts {
        let name = migration_path.file_name().unwrap().to_string_lossy();
        let script = std::fs::read_to_string(&migration_path).unwrap();
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
    let user_identifier = uuid::Uuid::new_v4().to_string();
    let account_identifier = uuid::Uuid::new_v4().to_string();
    let peer_account_identifier = uuid::Uuid::new_v4().to_string();
    let folder_identifier = uuid::Uuid::new_v4().to_string();
    let peer_folder_identifier = uuid::Uuid::new_v4().to_string();
    let mailbox_identifier = uuid::Uuid::new_v4().to_string();
    let peer_mailbox_identifier = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO users(identifier, name, email, password_hash, global_role, mail_access, created_at, updated_at) VALUES (?, 'Folder owner', 'folder-owner@example.test', 'hash', 'member', 'all', 1, 1)",
    )
    .bind(&user_identifier)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO imap_accounts(identifier, owner_identifier, name, username, password_hash, created_at, updated_at) VALUES (?, ?, 'Peer account', 'peer-account', 'hash', 1, 1)",
    )
    .bind(&peer_account_identifier)
    .bind(&user_identifier)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO imap_accounts(identifier, owner_identifier, name, username, password_hash, created_at, updated_at) VALUES (?, ?, 'Existing account', 'existing-account', 'hash', 1, 1)",
    )
    .bind(&account_identifier)
    .bind(&user_identifier)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES (?, ?, 'Foo', 'subject contains \"peer\"', 0, 2, 2)",
    )
    .bind(&peer_folder_identifier)
    .bind(&peer_account_identifier)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES (?, ?, 'Foo', 'subject contains \"existing\"', 1, 1, 1)",
    )
    .bind(&folder_identifier)
    .bind(&account_identifier)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO mail(identifier, subject, text_body, markup_body, raw_message, received_at, size) VALUES ('migration-mail', 'Migration mail', '', '', X'00', 1, 1)",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    let mail_sequence: i64 =
        sqlx::query_scalar("SELECT sequence FROM mail WHERE identifier = 'migration-mail'")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO imap_mailboxes(identifier, account_identifier, source, folder_identifier, uid_validity, uid_next, recent_through, examined_sequence, definition_fingerprint) VALUES (?, ?, 'folder', ?, 77, 6, 4, ?, 'folder-filter')",
    )
    .bind(&mailbox_identifier)
    .bind(&account_identifier)
    .bind(&folder_identifier)
    .bind(mail_sequence)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence) VALUES (?, 5, ?)",
    )
    .bind(&mailbox_identifier)
    .bind(mail_sequence)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO imap_mailboxes(identifier, account_identifier, source, folder_identifier, uid_validity, uid_next, recent_through, examined_sequence, definition_fingerprint) VALUES (?, ?, 'folder', ?, 88, 10, 8, ?, 'peer-folder-filter')",
    )
    .bind(&peer_mailbox_identifier)
    .bind(&peer_account_identifier)
    .bind(&peer_folder_identifier)
    .bind(mail_sequence)
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence) VALUES (?, 9, ?)",
    )
    .bind(&peer_mailbox_identifier)
    .bind(mail_sequence)
    .execute(&mut connection)
    .await
    .unwrap();
    connection.close().await.unwrap();

    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    let history = migration_history(&mut connection).await;
    let mut expected: Vec<_> = std::fs::read_dir(&migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    expected.sort();
    assert_eq!(history.len(), expected.len());
    assert_eq!(history, expected);
    let table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='user_message_deletions')",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(table_exists, 1);
    let migrated_folder: (String, String) =
        sqlx::query_as("SELECT owner_identifier, name FROM imap_user_folders WHERE identifier = ?")
            .bind(&folder_identifier)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(migrated_folder, (user_identifier.clone(), "Foo".into()));
    let mapped_folder: (String, String) = sqlx::query_as(
        "SELECT account_identifier, local_folder_identifier FROM imap_user_folder_accounts WHERE folder_identifier = ?",
    )
    .bind(&folder_identifier)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        mapped_folder,
        (account_identifier.clone(), folder_identifier.clone())
    );
    let per_account_projection_counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT account_identifier, count(*) FROM imap_user_folder_accounts GROUP BY account_identifier ORDER BY account_identifier",
    )
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(per_account_projection_counts.len(), 2);
    assert!(
        per_account_projection_counts
            .iter()
            .all(|(_, count)| *count == 2)
    );
    let preserved_original_projections: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM imap_user_folder_accounts WHERE (account_identifier = ?1 AND local_folder_identifier = ?2) OR (account_identifier = ?3 AND local_folder_identifier = ?4)",
    )
    .bind(&account_identifier)
    .bind(&folder_identifier)
    .bind(&peer_account_identifier)
    .bind(&peer_folder_identifier)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_original_projections, 2);
    let preserved_membership: (i64, i64, i64) = sqlx::query_as(
        "SELECT membership.uid, mailbox.uid_validity, mailbox.recent_through FROM imap_mailbox_messages AS membership JOIN imap_mailboxes AS mailbox ON mailbox.identifier = membership.mailbox_identifier WHERE mailbox.identifier = ?",
    )
    .bind(&mailbox_identifier)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_membership, (5, 77, 4));

    let preserved_peer_membership: (i64, i64, i64) = sqlx::query_as(
        "SELECT membership.uid, mailbox.uid_validity, mailbox.recent_through FROM imap_mailbox_messages AS membership JOIN imap_mailboxes AS mailbox ON mailbox.identifier = membership.mailbox_identifier WHERE mailbox.identifier = ?",
    )
    .bind(&peer_mailbox_identifier)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_peer_membership, (9, 88, 8));
    let owner_definitions: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, filter FROM imap_user_folders WHERE owner_identifier = ? ORDER BY filter",
    )
    .bind(&user_identifier)
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        owner_definitions,
        [
            ("Foo".into(), "subject contains \"existing\"".into()),
            ("Foo".into(), "subject contains \"peer\"".into()),
        ]
    );
    connection.close().await.unwrap();

    // The migrated owner definitions are visible from both legacy accounts, despite sharing a
    // display name, and account operations continue to project those definitions consistently.
    let storage = SqliteStorage::open(&path).await.unwrap();
    let account_one = ImapAccountIdentifier::from_str(&account_identifier).unwrap();
    let account_two = ImapAccountIdentifier::from_str(&peer_account_identifier).unwrap();
    let expected_folders = [
        ("Foo".to_owned(), "subject contains \"existing\"".to_owned()),
        ("Foo".to_owned(), "subject contains \"peer\"".to_owned()),
    ];
    for account in [account_one, account_two] {
        let folders = storage.list_imap_local_folders(account).await.unwrap();
        let mut projected: Vec<_> = folders
            .into_iter()
            .map(|folder| (folder.name, folder.filter))
            .collect();
        projected.sort();
        assert_eq!(projected, expected_folders);
    }

    let account_three = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: UserIdentifier::from_str(&user_identifier).unwrap(),
            name: "New account".into(),
            username: format!("new-{}", uuid::Uuid::new_v4()),
            password_hash: "hash".into(),
            mirror_views: false,
            created_at: 3,
        })
        .await
        .unwrap();
    let folders = storage
        .list_imap_local_folders(account_three.identifier)
        .await
        .unwrap();
    let mut projected: Vec<_> = folders
        .into_iter()
        .map(|folder| (folder.name, folder.filter))
        .collect();
    projected.sort();
    assert_eq!(projected, expected_folders);

    assert!(storage.delete_imap_account(account_one).await.unwrap());
    for account in [account_two, account_three.identifier] {
        let folders = storage.list_imap_local_folders(account).await.unwrap();
        let mut projected: Vec<_> = folders
            .into_iter()
            .map(|folder| (folder.name, folder.filter))
            .collect();
        projected.sort();
        assert_eq!(projected, expected_folders);
    }
    assert!(
        storage
            .list_imap_local_folders(account_one)
            .await
            .unwrap()
            .is_empty()
    );
    drop(storage);

    let mut connection = open_connection(&path).await;
    let remaining_definitions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM imap_user_folders WHERE owner_identifier = ? AND name = 'Foo'",
    )
    .bind(user_identifier)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(remaining_definitions, 2);
    let peer_membership_after_delete: (i64, i64, i64) = sqlx::query_as(
        "SELECT membership.uid, mailbox.uid_validity, mailbox.recent_through FROM imap_mailbox_messages AS membership JOIN imap_mailboxes AS mailbox ON mailbox.identifier = membership.mailbox_identifier WHERE mailbox.identifier = ?",
    )
    .bind(peer_mailbox_identifier)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(peer_membership_after_delete, (9, 88, 8));
    connection.close().await.unwrap();
}

/// Upgrade the first 27 scripts while preserving mailbox and deletion state.
#[tokio::test]
async fn first_27_scripts_migrate_forward_without_losing_imap_state() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut scripts: Vec<_> = std::fs::read_dir(&migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    scripts.sort();
    assert_eq!(scripts.len(), 39);

    let owner = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    let peer_account = uuid::Uuid::new_v4().to_string();
    let folder = uuid::Uuid::new_v4().to_string();
    let peer_folder = uuid::Uuid::new_v4().to_string();
    let mailbox = uuid::Uuid::new_v4().to_string();
    let (live_sequence, deleted_sequence): (i64, i64);
    {
        let mut connection = open_connection(&path).await;
        for migration_path in scripts.iter().take(27) {
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
        sqlx::query("INSERT INTO users(identifier, name, email, password_hash, global_role, mail_access, created_at, updated_at) VALUES (?1, 'Owner', 'first-27-owner@example.test', 'hash', 'member', 'all', 1, 1)")
            .bind(&owner).execute(&mut connection).await.unwrap();
        for (identifier, username) in [
            (&account, "first-27-primary"),
            (&peer_account, "first-27-peer"),
        ] {
            sqlx::query("INSERT INTO imap_accounts(identifier, owner_identifier, name, username, password_hash, created_at, updated_at) VALUES (?1, ?2, ?3, ?3, 'hash', 1, 1)")
                .bind(identifier).bind(&owner).bind(username).execute(&mut connection).await.unwrap();
        }
        sqlx::query("INSERT INTO imap_user_folders(identifier, owner_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES (?1, ?2, 'Legacy folder', 'true', 1, 1, 1)")
            .bind(&folder).bind(&owner).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES (?1, ?2, 'Legacy folder', 'true', 1, 1, 1)")
            .bind(&folder).bind(&account).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at) VALUES (?1, ?2, 'Legacy folder', 'true', 1, 1, 1)")
            .bind(&peer_folder).bind(&peer_account).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_user_folder_accounts(account_identifier, folder_identifier, local_folder_identifier) VALUES (?1, ?2, ?3), (?4, ?2, ?5)")
            .bind(&account).bind(&folder).bind(&folder).bind(&peer_account).bind(&peer_folder).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO mail(identifier, subject, text_body, markup_body, raw_message, received_at, size) VALUES ('live-first-27-mail', 'Live', '', '', X'00', 1, 1), ('deleted-first-27-mail', 'Deleted', '', '', X'00', 2, 1)")
            .execute(&mut connection).await.unwrap();
        live_sequence =
            sqlx::query_scalar("SELECT sequence FROM mail WHERE identifier = 'live-first-27-mail'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        deleted_sequence = sqlx::query_scalar(
            "SELECT sequence FROM mail WHERE identifier = 'deleted-first-27-mail'",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();
        sqlx::query("INSERT INTO imap_mailboxes(identifier, account_identifier, source, folder_identifier, uid_validity, uid_next, recent_through, examined_sequence, definition_fingerprint) VALUES (?1, ?2, 'folder', ?3, 77, 6, 4, ?4, 'legacy-fingerprint')")
            .bind(&mailbox).bind(&account).bind(&folder).bind(live_sequence).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence, deleted, modification) VALUES (?1, 5, ?2, 1, 9), (?1, 6, ?3, 0, 0)")
            .bind(&mailbox).bind(live_sequence).bind(deleted_sequence).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_message_flags(account_identifier, mail_sequence, seen, answered, flagged, draft, keywords, modification) VALUES (?1, ?2, 1, 1, 0, 1, '$Work', 77)")
            .bind(&account).bind(live_sequence).execute(&mut connection).await.unwrap();
        sqlx::query(
            "INSERT INTO user_message_deletions(user_identifier, mail_sequence) VALUES (?1, ?2)",
        )
        .bind(&owner)
        .bind(deleted_sequence)
        .execute(&mut connection)
        .await
        .unwrap();
        let member_count: i64 = sqlx::query_scalar("SELECT count(*) FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND mail_sequence = ?")
            .bind(&mailbox).bind(deleted_sequence).fetch_one(&mut connection).await.unwrap();
        assert_eq!(
            member_count, 0,
            "the deletion tombstone trigger removes membership"
        );
        connection.close().await.unwrap();
    }

    drop(SqliteStorage::open(&path).await.unwrap());
    let mut connection = open_connection(&path).await;
    let history = migration_history(&mut connection).await;
    assert_eq!(history.len(), 39);
    assert_eq!(history[27], "0028_add_imap_local_folder_kinds.sql");

    let preserved_folder: (String, String, String) = sqlx::query_as(
        "SELECT identifier, kind, definition_identifier FROM imap_local_folders WHERE identifier = ?",
    )
    .bind(&folder)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        preserved_folder,
        (folder.clone(), "smart".into(), folder.clone())
    );
    let preserved_mailbox: (String, String, i64, i64, i64, i64, String) = sqlx::query_as(
        "SELECT identifier, folder_identifier, uid_validity, uid_next, recent_through, examined_sequence, definition_fingerprint FROM imap_mailboxes WHERE identifier = ?",
    )
    .bind(&mailbox)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        preserved_mailbox,
        (
            mailbox.clone(),
            folder.clone(),
            77,
            6,
            4,
            live_sequence,
            "legacy-fingerprint".into()
        )
    );
    let preserved_member: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT uid, mail_sequence, deleted, modification FROM imap_mailbox_messages WHERE mailbox_identifier = ?",
    )
    .bind(&mailbox)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_member, (5, live_sequence, 1, 9));
    let preserved_flags: (i64, i64, i64, i64, String, i64) = sqlx::query_as(
        "SELECT seen, answered, flagged, draft, keywords, modification FROM imap_message_flags WHERE account_identifier = ? AND mail_sequence = ?",
    )
    .bind(&account)
    .bind(live_sequence)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_flags, (1, 1, 0, 1, "$Work".into(), 77));
    let preserved_tombstone: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_message_deletions WHERE user_identifier = ? AND mail_sequence = ?",
    )
    .bind(&owner)
    .bind(deleted_sequence)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_tombstone, 1);
    let preserved_projection: (String, String) = sqlx::query_as(
        "SELECT account_identifier, local_folder_identifier FROM imap_user_folder_accounts WHERE folder_identifier = ? AND account_identifier = ?",
    )
    .bind(&folder)
    .bind(&account)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_projection, (account.clone(), folder.clone()));
    let peer_projection: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM imap_user_folder_accounts WHERE folder_identifier = ? AND account_identifier = ?",
    )
    .bind(&folder)
    .bind(&peer_account)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(peer_projection, 1);
    let preserved_peer_projection: String = sqlx::query_scalar(
        "SELECT local_folder_identifier FROM imap_user_folder_accounts WHERE folder_identifier = ? AND account_identifier = ?",
    )
    .bind(&folder)
    .bind(&peer_account)
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(preserved_peer_projection, peer_folder);
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
    sqlx::query("DELETE FROM migrations WHERE migration = '0013_create_search_outbox_table.sql'")
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

/// A failed final script rolls back every schema and history change in the suffix transaction.
#[tokio::test]
async fn a_failed_migration_suffix_rolls_back_atomically() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let migration_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut scripts: Vec<_> = std::fs::read_dir(&migration_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    scripts.sort();

    let account = uuid::Uuid::new_v4().to_string();
    let sequence: i64;
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
        sqlx::query("INSERT INTO mail(identifier, subject, text_body, markup_body, raw_message, received_at, size) VALUES ('rollback-mail', 'Rollback', '', '', X'00', 1, 1)")
            .execute(&mut connection).await.unwrap();
        sequence =
            sqlx::query_scalar("SELECT sequence FROM mail WHERE identifier = 'rollback-mail'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO imap_mailboxes(identifier, account_identifier, source, uid_validity, uid_next, recent_through, examined_sequence, definition_fingerprint) VALUES ('rollback-trash', ?1, 'trash', 1, 1, 0, ?2, 'rollback')")
            .bind(&account).bind(sequence).execute(&mut connection).await.unwrap();
        sqlx::query("INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence) VALUES ('rollback-trash', 1, ?1)")
            .bind(sequence).execute(&mut connection).await.unwrap();
        connection.close().await.unwrap();
    }

    assert!(matches!(
        SqliteStorage::open(&path).await,
        Err(StorageError::Migration(message)) if message.contains("0034_backfill_imap_trash_copies.sql")
    ));
    let mut connection = open_connection(&path).await;
    assert_eq!(migration_history(&mut connection).await.len(), 25);
    let placements_column: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pragma_table_info('imap_local_folders') WHERE name = 'kind'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    let copies_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'imap_message_copies'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    let deletions_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'user_message_deletions'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    let folders_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'imap_user_folders'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(
        (
            placements_column,
            copies_table,
            deletions_table,
            folders_table
        ),
        (0, 0, 0, 0)
    );
    let mail_count: i64 = sqlx::query_scalar("SELECT count(*) FROM mail WHERE sequence = ?")
        .bind(sequence)
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(mail_count, 1);
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
