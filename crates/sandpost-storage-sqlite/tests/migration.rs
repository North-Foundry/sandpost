//! Schema creation, forward migration, and version-guard tests for the SQLite backend.
//!
//! A fresh database applies the version 1 baseline and every later batch; version 1, 2, 3, and 4
//! databases are upgraded in place without losing data; any other stored schema version is
//! rejected.
use rusqlite::Connection;
use sandpost_core::{
    GlobalRole, MailAccess, Message, MessageFacts, MessageIdentifier, SmtpAuthenticationMechanism,
    UserIdentifier,
};
use sandpost_storage::{MessageListQuery, NewUser, Storage};
use sandpost_storage_sqlite::SqliteStorage;
use std::path::PathBuf;
use std::sync::Arc;

/// Own an isolated temporary data directory for the duration of a test.
struct TestDirectory(PathBuf);
impl TestDirectory {
    /// Create an isolated directory for schema and upgrade fixtures.
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
fn stored_version(path: &std::path::Path) -> i64 {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

/// The checked-in version 1 baseline scripts, applied to build legacy databases in tests.
const VERSION_ONE_SCRIPTS: [(&str, &str); 16] = [
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

/// The endpoints script of pre-release version 1 builds with per-endpoint SMTP listeners.
const ENDPOINT_LISTENER_SCRIPT: &str =
    include_str!("../migrations/variants/0002_create_endpoints_table_with_listeners.sql");

/// Create a version 1 database exactly as an earlier build did, with its history recorded.
///
/// `endpoint_listeners` selects the pre-release layout whose endpoints carry SMTP listener columns.
fn create_version_one_database(path: &std::path::Path, endpoint_listeners: bool) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    for (name, script) in VERSION_ONE_SCRIPTS {
        let script = if endpoint_listeners && name == "0002_create_endpoints_table.sql" {
            ENDPOINT_LISTENER_SCRIPT
        } else {
            script
        };
        connection.execute_batch(script).unwrap();
        connection
            .execute(
                "INSERT INTO migrations (migration, batch) VALUES (?1, 1)",
                [name],
            )
            .unwrap();
    }
    connection.pragma_update(None, "user_version", 1).unwrap();
    connection
}

/// The version 2 scripts, applied to build databases written by the previous release.
const VERSION_TWO_SCRIPTS: [(&str, &str); 3] = [
    (
        "0017_create_smtp_server_table.sql",
        include_str!("../migrations/0017_create_smtp_server_table.sql"),
    ),
    (
        "0018_remove_endpoint_listener_columns.sql",
        include_str!("../migrations/0018_remove_endpoint_listener_columns.sql"),
    ),
    (
        "0019_create_smtp_accesses_table.sql",
        include_str!("../migrations/0019_create_smtp_accesses_table.sql"),
    ),
];

/// The version 3 scripts, applied over version 2 to build a version 3 database.
const VERSION_THREE_SCRIPTS: [(&str, &str); 6] = [
    (
        "0020_rebuild_users_with_mail_access.sql",
        include_str!("../migrations/0020_rebuild_users_with_mail_access.sql"),
    ),
    (
        "0021_rebuild_scopes_without_endpoints.sql",
        include_str!("../migrations/0021_rebuild_scopes_without_endpoints.sql"),
    ),
    (
        "0022_rebuild_views_without_endpoints.sql",
        include_str!("../migrations/0022_rebuild_views_without_endpoints.sql"),
    ),
    (
        "0023_rebuild_smtp_accesses_without_endpoints.sql",
        include_str!("../migrations/0023_rebuild_smtp_accesses_without_endpoints.sql"),
    ),
    (
        "0024_rebuild_mail_without_endpoints.sql",
        include_str!("../migrations/0024_rebuild_mail_without_endpoints.sql"),
    ),
    (
        "0025_drop_endpoints.sql",
        include_str!("../migrations/0025_drop_endpoints.sql"),
    ),
];

/// The stable identifier of the endpoint seeded by version 1.
const DEFAULT_ENDPOINT: &str = "00000000-0000-0000-0000-000000000002";

/// Accounts and records written into a legacy database with endpoints.
struct LegacyFixture {
    owner: UserIdentifier,
    /// Member with `all` mail access on only one of the two endpoints.
    partial_member: UserIdentifier,
    /// Member with `all` mail access on every endpoint.
    full_member: UserIdentifier,
    /// Member whose only endpoint role is `viewer`.
    viewer: UserIdentifier,
    /// Member administering one endpoint with scoped access and one assigned scope.
    endpoint_admin: UserIdentifier,
    /// Global administrator without any endpoint membership.
    global_admin: UserIdentifier,
    scope: String,
}

/// Populate a legacy database (version 1 or 2) with two endpoints, every kind of membership, a
/// scope assignment, a view, and captured mail on both endpoints.
///
/// In the version 1 listener layout the second endpoint listens on 127.0.0.1:2600.
fn populate_legacy_database(connection: &Connection, endpoint_listeners: bool) -> LegacyFixture {
    let staging = uuid::Uuid::new_v4().to_string();
    let insert_endpoint = if endpoint_listeners {
        "INSERT INTO endpoints(identifier, name, smtp_host, smtp_port) VALUES (?1, 'Staging', '127.0.0.1', 2600)"
    } else {
        "INSERT INTO endpoints(identifier, name) VALUES (?1, 'Staging')"
    };
    connection.execute(insert_endpoint, [&staging]).unwrap();
    let insert_user = |role: &str| {
        let identifier = UserIdentifier::new();
        connection
            .execute(
                "INSERT INTO users(identifier,name,email,password_hash,global_role,created_at,updated_at) VALUES (?1,'Legacy',?1 || '@example.test','hash',?2,1,1)",
                [identifier.to_string(), role.to_owned()],
            )
            .unwrap();
        identifier
    };
    let membership = |user: UserIdentifier, endpoint: &str, role: &str, access: &str| {
        connection
            .execute(
                "INSERT INTO endpoint_memberships(user_identifier,endpoint_identifier,role,mail_access) VALUES (?1,?2,?3,?4)",
                [user.to_string(), endpoint.to_owned(), role.to_owned(), access.to_owned()],
            )
            .unwrap();
    };
    let fixture = LegacyFixture {
        owner: insert_user("owner"),
        partial_member: insert_user("member"),
        full_member: insert_user("member"),
        viewer: insert_user("member"),
        endpoint_admin: insert_user("member"),
        global_admin: insert_user("admin"),
        scope: uuid::Uuid::new_v4().to_string(),
    };
    membership(fixture.partial_member, &staging, "member", "all");
    membership(fixture.full_member, &staging, "member", "all");
    membership(fixture.full_member, DEFAULT_ENDPOINT, "member", "all");
    membership(fixture.viewer, DEFAULT_ENDPOINT, "viewer", "all");
    membership(fixture.viewer, &staging, "viewer", "all");
    membership(fixture.endpoint_admin, &staging, "admin", "scoped");
    connection
        .execute(
            "INSERT INTO scopes(identifier,parent_identifier,name,description,filter,position,policy_version,endpoint_identifier) VALUES (?1,NULL,'Invoices',NULL,'subject contains \"invoice\"',0,1,?2)",
            [&fixture.scope, &staging],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO scope_memberships(user_identifier,scope_identifier) VALUES (?1,?2)",
            [fixture.endpoint_admin.to_string(), fixture.scope.clone()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO views(identifier,endpoint_identifier,owner_identifier,name,filter) VALUES (?1,?2,?3,'Receipts','subject contains \"receipt\"')",
            [
                uuid::Uuid::new_v4().to_string(),
                staging.clone(),
                fixture.partial_member.to_string(),
            ],
        )
        .unwrap();
    for (subject, endpoint) in [
        ("Staging invoice", staging.as_str()),
        ("Default receipt", DEFAULT_ENDPOINT),
        ("Deleted newest", DEFAULT_ENDPOINT),
    ] {
        connection
            .execute(
                "INSERT INTO mail(identifier,subject,text_body,markup_body,raw_message,received_at,size,endpoint_identifier) VALUES (?1,?2,'','',X'41',1,1,?3)",
                [uuid::Uuid::new_v4().to_string(), subject.to_owned(), endpoint.to_owned()],
            )
            .unwrap();
    }
    // Deleting the newest mail leaves an AUTOINCREMENT high-water mark the upgrade must keep.
    connection
        .execute("DELETE FROM mail WHERE subject = 'Deleted newest'", [])
        .unwrap();
    fixture
}

/// Upgrade a populated legacy database and verify data and the converted permissions.
async fn upgrade_and_verify(path: &std::path::Path, fixture: &LegacyFixture) -> Arc<dyn Storage> {
    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(path).unwrap());
    assert_eq!(stored_version(path), 5);
    for (user, role, access) in [
        (fixture.owner, GlobalRole::Owner, MailAccess::All),
        (
            fixture.partial_member,
            GlobalRole::Member,
            MailAccess::Scoped,
        ),
        (fixture.full_member, GlobalRole::Member, MailAccess::All),
        (fixture.viewer, GlobalRole::Viewer, MailAccess::All),
        (
            fixture.endpoint_admin,
            GlobalRole::Member,
            MailAccess::Scoped,
        ),
        (fixture.global_admin, GlobalRole::Admin, MailAccess::Scoped),
    ] {
        let account = storage.get_user(user).await.unwrap().unwrap();
        assert_eq!((account.global_role, account.mail_access), (role, access));
    }
    assert_eq!(
        storage
            .list_user_scopes(fixture.endpoint_admin)
            .await
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![fixture.scope.clone()],
        "scope assignments survive the upgrade"
    );
    assert_eq!(storage.list_scopes().await.unwrap().len(), 1);
    let views = storage.list_views(fixture.partial_member).await.unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Receipts");
    let mut subjects: Vec<String> = storage
        .list_messages(MessageListQuery {
            filter: None,
            before: None,
            limit: 10,
        })
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.subject)
        .collect();
    subjects.sort();
    assert_eq!(subjects, ["Default receipt", "Staging invoice"]);
    let connection = Connection::open(path).unwrap();
    for table in ["endpoints", "endpoint_memberships"] {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = ?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists, "{table} must be dropped");
    }
    let sequence = storage
        .insert_message(&Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: "After upgrade".into(),
                ..MessageFacts::default()
            },
            raw_message: Vec::new(),
            attachments: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(sequence.0, 4, "deleted mail sequences are never reused");
    assert!(
        storage
            .assign_user_to_scope(UserIdentifier::new(), fixture.scope.parse().unwrap())
            .await
            .is_err(),
        "foreign keys are enforced again after the upgrade"
    );
    storage
}

/// A canonical version 1 database is upgraded through every batch, keeping its data, converting
/// endpoint memberships, and using the default SMTP address version 1 had.
#[tokio::test]
async fn a_version_one_database_is_upgraded_without_losing_data() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let fixture = {
        let connection = create_version_one_database(&path, false);
        populate_legacy_database(&connection, false)
    };
    let storage = upgrade_and_verify(&path, &fixture).await;
    assert_eq!(
        storage.smtp_listen_address().await.unwrap(),
        Some("127.0.0.1:1025".parse().unwrap())
    );
    drop(storage);
    drop(SqliteStorage::open(&path).expect("an upgraded database reopens"));
}

/// A version 1 database with per-endpoint listeners keeps its data and its primary listener
/// becomes the global server address.
#[tokio::test]
async fn a_version_one_database_with_endpoint_listeners_is_upgraded() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let fixture = {
        let connection = create_version_one_database(&path, true);
        connection
            .execute(
                "UPDATE endpoints SET smtp_host = '0.0.0.0', smtp_port = 2525 WHERE identifier = ?1",
                [DEFAULT_ENDPOINT],
            )
            .unwrap();
        populate_legacy_database(&connection, true)
    };
    let storage = upgrade_and_verify(&path, &fixture).await;
    assert_eq!(
        storage.smtp_listen_address().await.unwrap(),
        Some("0.0.0.0:2525".parse().unwrap()),
        "the default endpoint's listener becomes the global server"
    );
}

/// A version 2 database, with endpoints and SMTP accesses that target them, is upgraded to the
/// single mail pool without losing accesses or mail.
#[tokio::test]
async fn a_version_two_database_is_upgraded_without_losing_data() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let fixture = {
        let connection = create_version_one_database(&path, false);
        for (name, script) in VERSION_TWO_SCRIPTS {
            connection.execute_batch(script).unwrap();
            connection
                .execute(
                    "INSERT INTO migrations (migration, batch) VALUES (?1, 2)",
                    [name],
                )
                .unwrap();
        }
        connection.pragma_update(None, "user_version", 2).unwrap();
        let fixture = populate_legacy_database(&connection, false);
        connection
            .execute(
                "INSERT INTO smtp_accesses(identifier,name,username,password_hash,endpoint_identifier,enabled,created_at,updated_at) VALUES (?1,'Laravel','laravel-1','$argon2id$hash',?2,1,5,5)",
                [uuid::Uuid::new_v4().to_string(), DEFAULT_ENDPOINT.to_owned()],
            )
            .unwrap();
        fixture
    };
    let storage = upgrade_and_verify(&path, &fixture).await;
    let accesses = storage.list_smtp_accesses().await.unwrap();
    assert_eq!(accesses.len(), 1);
    assert_eq!(accesses[0].username, "laravel-1");
    assert_eq!(
        storage
            .get_smtp_access_credential_by_username("laravel-1")
            .await
            .unwrap()
            .map(|credential| credential.password_hash),
        Some("$argon2id$hash".to_owned())
    );
}

/// A version 3 database keeps its SMTP accesses, which accept PLAIN and LOGIN on any connection.
#[tokio::test]
async fn a_version_three_database_keeps_smtp_accesses_with_permissive_rules() {
    let directory = TestDirectory::new();
    let path = directory.database();
    {
        let connection = create_version_one_database(&path, false);
        for (batch, scripts) in [
            (2, &VERSION_TWO_SCRIPTS[..]),
            (3, &VERSION_THREE_SCRIPTS[..]),
        ] {
            for (name, script) in scripts {
                connection.execute_batch(script).unwrap();
                connection
                    .execute(
                        "INSERT INTO migrations (migration, batch) VALUES (?1, ?2)",
                        rusqlite::params![name, batch],
                    )
                    .unwrap();
            }
        }
        connection.pragma_update(None, "user_version", 3).unwrap();
        connection
            .execute(
                "INSERT INTO smtp_accesses(identifier,name,username,password_hash,enabled,created_at,updated_at) VALUES (?1,'Laravel','laravel-3','$argon2id$hash',1,5,5)",
                [uuid::Uuid::new_v4().to_string()],
            )
            .unwrap();
    }
    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&path).unwrap());
    assert_eq!(stored_version(&path), 5);
    let accesses = storage.list_smtp_accesses().await.unwrap();
    assert_eq!(accesses.len(), 1);
    assert_eq!(accesses[0].username, "laravel-3");
    assert!(!accesses[0].requires_encryption);
    assert_eq!(
        accesses[0].allowed_mechanisms,
        SmtpAuthenticationMechanism::ALL.to_vec()
    );
}

/// Without a default listener, the first remaining endpoint listener becomes the global server,
/// and without any listener the server stays disabled.
#[tokio::test]
async fn listener_upgrade_falls_back_to_another_listener_or_none() {
    for (clear_staging, expected) in [(false, Some("127.0.0.1:2600")), (true, None)] {
        let directory = TestDirectory::new();
        let path = directory.database();
        {
            let connection = create_version_one_database(&path, true);
            populate_legacy_database(&connection, true);
            connection
                .execute(
                    "UPDATE endpoints SET smtp_host = NULL, smtp_port = NULL WHERE identifier = ?1 OR ?2",
                    rusqlite::params![DEFAULT_ENDPOINT, clear_staging],
                )
                .unwrap();
        }
        let storage = SqliteStorage::open(&path).unwrap();
        assert_eq!(
            sandpost_storage::SmtpServerStorage::smtp_listen_address(&storage)
                .await
                .unwrap(),
            expected.map(|address| address.parse().unwrap())
        );
    }
}

/// A version 1 database whose schema was tampered with is rejected instead of upgraded.
#[test]
fn a_tampered_version_one_database_is_not_upgraded() {
    for endpoint_listeners in [false, true] {
        let directory = TestDirectory::new();
        let path = directory.database();
        create_version_one_database(&path, endpoint_listeners)
            .execute_batch("DROP INDEX views_owner_name;")
            .unwrap();
        assert!(SqliteStorage::open(&path).is_err());
        assert_eq!(stored_version(&path), 1, "the failed upgrade rolled back");
    }
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
                mail_access: MailAccess::All,
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

/// A fresh database has no mail sequence row until its first insert, like a table created
/// directly, even though the migrations rebuild the mail table.
#[test]
fn a_fresh_database_records_no_mail_sequence() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    let rows: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_sequence WHERE name = 'mail'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

/// The version 4 schema script, applied to build version 4 databases in tests.
const VERSION_FOUR_SCRIPTS: [(&str, &str); 1] = [(
    "0026_add_smtp_access_authentication_rules.sql",
    include_str!("../migrations/0026_add_smtp_access_authentication_rules.sql"),
)];

/// A version 4 database gains the IMAP tables at version 5 and keeps its SMTP accesses.
#[tokio::test]
async fn a_version_four_database_gains_imap_tables() {
    let directory = TestDirectory::new();
    let path = directory.database();
    {
        let connection = create_version_one_database(&path, false);
        for (batch, scripts) in [
            (2, &VERSION_TWO_SCRIPTS[..]),
            (3, &VERSION_THREE_SCRIPTS[..]),
            (4, &VERSION_FOUR_SCRIPTS[..]),
        ] {
            for (name, script) in scripts {
                connection.execute_batch(script).unwrap();
                connection
                    .execute(
                        "INSERT INTO migrations (migration, batch) VALUES (?1, ?2)",
                        rusqlite::params![name, batch],
                    )
                    .unwrap();
            }
        }
        connection.pragma_update(None, "user_version", 4).unwrap();
        connection
            .execute(
                "INSERT INTO smtp_accesses(identifier,name,username,password_hash,enabled,created_at,updated_at) VALUES (?1,'Laravel','laravel-4','$argon2id$hash',1,5,5)",
                [uuid::Uuid::new_v4().to_string()],
            )
            .unwrap();
    }
    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&path).unwrap());
    assert_eq!(stored_version(&path), 5);
    assert_eq!(storage.list_smtp_accesses().await.unwrap().len(), 1);
    let owner = storage
        .create_user(&NewUser {
            name: "Owner".into(),
            email: "owner@example.test".into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Owner,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let account = storage
        .create_imap_account(&sandpost_storage::NewImapAccount {
            owner_identifier: owner.identifier,
            name: "Debug".into(),
            username: "debug".into(),
            password_hash: "hash".into(),
            mirror_views: true,
            created_at: 1,
        })
        .await
        .unwrap();
    let mailboxes = storage
        .reconcile_imap_mailboxes(
            account.identifier,
            &[sandpost_core::ImapMailboxSource::Inbox],
        )
        .await
        .unwrap();
    assert_eq!(mailboxes.len(), 1);
    drop(storage);
    let reopened = SqliteStorage::open(&path).unwrap();
    assert!(
        sandpost_storage::ImapStorage::get_imap_account(&reopened, account.identifier)
            .await
            .unwrap()
            .is_some(),
        "IMAP state persists across restarts"
    );
}

/// A newer stored schema version is rejected rather than migrated.
#[test]
fn a_newer_schema_version_is_rejected() {
    let directory = TestDirectory::new();
    let path = directory.database();
    drop(SqliteStorage::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 6)
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
