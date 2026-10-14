//! Pool concurrency, WAL snapshots, cancellation, and lifecycle regression tests.
use super::*;
use sandpost_core::{Message, MessageFacts, MessageIdentifier};
use sandpost_storage::{MessageListQuery, MessageStorage, SearchSynchronizationStorage};
use sqlx::{ConnectOptions, Connection};
use std::sync::Arc;

/// Own an isolated directory until its database handles have been closed.
struct TestDirectory(std::path::PathBuf);
impl TestDirectory {
    /// Create a unique temporary directory for a pooled file database.
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("sandpost-pool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
    /// Return this test's database path.
    fn database(&self) -> std::path::PathBuf {
        self.0.join("mail.sqlite3")
    }
}
impl Drop for TestDirectory {
    /// Remove the database and its sidecars after storage closes.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build mail whose identity and opaque MIME can be checked after concurrent writes.
fn message(index: usize) -> Message {
    Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            subject: index.to_string(),
            size: 4,
            ..Default::default()
        },
        raw_message: vec![0xff, 0, 0xfe, index as u8],
        attachments: Vec::new(),
    }
}

/// All configured read connections execute SELECTs while held simultaneously, and reject writes.
#[tokio::test]
async fn concurrent_readers_use_distinct_read_only_connections() {
    let directory = TestDirectory::new();
    let storage = SqliteStorage::open(directory.database()).await.unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(READ_CONNECTIONS as usize));
    let mut tasks = Vec::new();
    for _ in 0..READ_CONNECTIONS {
        let pool = storage.read_pool.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            let mut connection = pool.acquire().await.unwrap();
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mail")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
            assert_eq!(count, 0);
            barrier.wait().await;
            assert!(
                sqlx::query("DELETE FROM mail")
                    .execute(&mut *connection)
                    .await
                    .is_err()
            );
        }));
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        for task in tasks {
            task.await.unwrap();
        }
    })
    .await
    .expect("readers must not serialize at the pool boundary");
    storage.close().await;
}

/// WAL permits a writer commit while an earlier reader retains a coherent transaction snapshot.
#[tokio::test]
async fn readers_and_writer_overlap_with_committed_visibility() {
    let directory = TestDirectory::new();
    let storage = SqliteStorage::open(directory.database()).await.unwrap();
    let mail = message(0);
    storage.insert_message(&mail).await.unwrap();
    let mut snapshot = storage.read_pool.begin().await.unwrap();
    let original: String = sqlx::query_scalar("SELECT subject FROM mail")
        .fetch_one(&mut *snapshot)
        .await
        .unwrap();
    let mut writer = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .unwrap();
    sqlx::query("UPDATE mail SET subject='committed' WHERE identifier=?")
        .bind(mail.identifier.to_string())
        .execute(&mut *writer)
        .await
        .unwrap();
    let before_commit =
        tokio::time::timeout(Duration::from_secs(5), storage.get_message(mail.identifier))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    assert_eq!(before_commit.facts.subject, original);
    writer.commit().await.unwrap();
    let unchanged: String = sqlx::query_scalar("SELECT subject FROM mail")
        .fetch_one(&mut *snapshot)
        .await
        .unwrap();
    assert_eq!(unchanged, original);
    snapshot.commit().await.unwrap();
    assert_eq!(
        storage
            .get_message(mail.identifier)
            .await
            .unwrap()
            .unwrap()
            .facts
            .subject,
        "committed"
    );
    storage.close().await;
}

/// Concurrent inserts commit complete MIME records in unique sequence order with one revision each.
#[tokio::test]
async fn concurrent_inserts_preserve_order_mime_and_outbox() {
    let directory = TestDirectory::new();
    let storage = SqliteStorage::open(directory.database()).await.unwrap();
    let mut tasks = Vec::new();
    for index in 0..32 {
        let storage = storage.clone();
        tasks.push(tokio::spawn(async move {
            let mail = message(index);
            let sequence = storage.insert_message(&mail).await.unwrap();
            assert_eq!(
                storage
                    .get_message(mail.identifier)
                    .await
                    .unwrap()
                    .unwrap()
                    .raw_message,
                mail.raw_message
            );
            sequence
        }));
    }
    let mut sequences = Vec::new();
    for task in tasks {
        sequences.push(task.await.unwrap().0);
    }
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=32).collect::<Vec<_>>());
    let status = storage.search_status().await.unwrap();
    assert_eq!(status.latest_sequence, 32);
    let summaries = storage
        .list_messages(MessageListQuery {
            filter: None,
            before: None,
            limit: 32,
        })
        .await
        .unwrap();
    assert!(
        summaries
            .windows(2)
            .all(|pair| pair[0].sequence > pair[1].sequence)
    );
    storage.close().await;
}

/// Every pooled connection enforces foreign keys and preserves the five-second busy timeout.
#[tokio::test]
async fn pooled_connections_preserve_foreign_keys_and_timeout() {
    let directory = TestDirectory::new();
    let storage = SqliteStorage::open(directory.database()).await.unwrap();
    let mut readers = Vec::new();
    for _ in 0..READ_CONNECTIONS {
        readers.push(storage.read_pool.acquire().await.unwrap());
    }
    let mut writer = storage.write_pool.acquire().await.unwrap();
    for connection in readers.iter_mut().chain(std::iter::once(&mut writer)) {
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        let durability: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        assert_eq!((foreign_keys, busy_timeout, durability), (1, 5000, 2));
    }
    let error = sqlx::query("INSERT INTO scope_memberships(user_identifier,scope_identifier) VALUES('missing-user','missing-scope')")
        .execute(&mut *writer).await.unwrap_err();
    assert!(
        error
            .as_database_error()
            .unwrap()
            .is_foreign_key_violation()
    );
    drop(writer);
    drop(readers);
    storage.close().await;
}

/// An external writer lock eventually yields a semantic availability failure, then recovery.
#[tokio::test]
async fn writer_lock_contention_is_bounded_and_recoverable() {
    let directory = TestDirectory::new();
    let storage = SqliteStorage::open(directory.database()).await.unwrap();
    let mut external = connection_options()
        .filename(directory.database())
        .connect()
        .await
        .unwrap();
    let lock = external.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(15), storage.insert_message(&message(0)))
        .await
        .expect("busy handling must terminate");
    assert!(matches!(result, Err(StorageError::Unavailable)));
    lock.rollback().await.unwrap();
    assert_eq!(storage.insert_message(&message(1)).await.unwrap().0, 1);
    storage.close().await;
}

/// Dropping an uncommitted transaction rolls back its mail mutation and durable outbox operation.
#[tokio::test]
async fn abandoned_transactions_rollback_and_memory_handles_share_state() {
    let storage = SqliteStorage::memory().await.unwrap();
    let mail = message(0);
    storage.insert_message(&mail).await.unwrap();
    let clone = storage.clone();
    {
        let mut transaction = storage
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .unwrap();
        sqlx::query("DELETE FROM mail")
            .execute(&mut *transaction)
            .await
            .unwrap();
    }
    assert!(clone.get_message(mail.identifier).await.unwrap().is_some());
    assert_eq!(clone.search_status().await.unwrap().latest_sequence, 1);
    assert_eq!(storage.read_pool.options().get_max_connections(), 1);
    storage.close().await;
    assert!(matches!(
        clone.health().await,
        Err(StorageError::Unavailable)
    ));
}

/// Graceful close wakes a pending acquisition and waits for the checked-out writer to return.
#[tokio::test]
async fn pool_close_releases_waiters_and_shared_handles() {
    let storage = SqliteStorage::memory().await.unwrap();
    let connection = storage.write_pool.acquire().await.unwrap();
    let pool = storage.write_pool.clone();
    let waiter = tokio::spawn(async move { pool.acquire().await });
    let clone = storage.clone();
    let closer = tokio::spawn(async move { clone.close().await });
    let result = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(sqlx::Error::PoolClosed)));
    drop(connection);
    tokio::time::timeout(Duration::from_secs(5), closer)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        storage.health().await,
        Err(StorageError::Unavailable)
    ));
}

/// Cancelling a task inside a transaction rolls back both mail and outbox before pool reuse.
#[tokio::test]
async fn cancellation_rolls_back_before_reusing_memory_connection() {
    let storage = SqliteStorage::memory().await.unwrap();
    let mail = message(0);
    storage.insert_message(&mail).await.unwrap();
    let clone = storage.clone();
    let (started, entered) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut transaction = clone
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .unwrap();
        sqlx::query("DELETE FROM mail")
            .execute(&mut *transaction)
            .await
            .unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
        transaction.commit().await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), entered)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        storage
            .get_message(mail.identifier)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(storage.search_status().await.unwrap().latest_sequence, 1);
    storage.close().await;
}

/// Simultaneous first openers initialize one schema and reopen the same committed database.
#[tokio::test]
async fn concurrent_initialization_preserves_one_migration_history() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let (first, second) = tokio::join!(SqliteStorage::open(&path), SqliteStorage::open(&path));
    let first = first.unwrap();
    let second = second.unwrap();
    let history: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM migrations")
        .fetch_one(&first.read_pool)
        .await
        .unwrap();
    let expected_history = crate::migrations::MIGRATIONS.len() as i64;
    assert_eq!(history, expected_history);
    let mail = message(1);
    first.insert_message(&mail).await.unwrap();
    assert!(second.get_message(mail.identifier).await.unwrap().is_some());
    first.close().await;
    second.close().await;
}

/// Unchanged IMAP reconciliation stays readable while another database connection owns a writer lock.
#[tokio::test]
async fn unchanged_imap_membership_does_not_require_a_write_lock() {
    use sandpost_core::{GlobalRole, ImapMailboxSource, MailAccess};
    use sandpost_storage::{
        ImapMembershipDefinition, ImapStorage, NewImapAccount, NewUser, UserStorage,
    };
    let directory = TestDirectory::new();
    let storage = SqliteStorage::open(directory.database()).await.unwrap();
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
    let account = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: owner.identifier,
            name: "Account".into(),
            username: "owner".into(),
            password_hash: "hash".into(),
            mirror_views: false,
            mode: sandpost_core::ImapAccountMode::Independent,
            initial_authorization: sandpost_query::Expression::True,
            created_at: 0,
        })
        .await
        .unwrap();
    let states = storage
        .reconcile_imap_mailboxes(
            account.identifier,
            &[ImapMailboxSource::Inbox],
            Some(account.mode_generation),
        )
        .await
        .unwrap();
    let definition = ImapMembershipDefinition {
        expected_mode_generation: Some(account.mode_generation),
        filter: sandpost_query::Expression::True,
        fingerprint: "all".into(),
    };
    let mailbox = storage
        .synchronize_imap_mailbox(states[0].identifier, &definition)
        .await
        .unwrap()
        .unwrap();
    let mut external = connection_options()
        .filename(directory.database())
        .connect()
        .await
        .unwrap();
    let lock = external.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        assert_eq!(
            storage
                .reconcile_imap_mailboxes(
                    account.identifier,
                    &[ImapMailboxSource::Inbox],
                    Some(account.mode_generation)
                )
                .await
                .unwrap(),
            vec![mailbox]
        );
        assert_eq!(
            storage
                .synchronize_imap_mailbox(mailbox.identifier, &definition)
                .await
                .unwrap(),
            Some(mailbox)
        );
    })
    .await;
    lock.rollback().await.unwrap();
    result.expect("unchanged mailbox reconciliation must only read");
    external.close().await.unwrap();
    storage.close().await;
}

/// Construction futures must be movable onto the runtime used by native application startup.
#[test]
fn initialization_futures_are_send() {
    /// Require the same Send bound imposed by Tokio and the native asynchronous adapter.
    fn require_send(_: impl Send) {}
    require_send(SqliteStorage::open(std::path::PathBuf::from(
        "unused.sqlite3",
    )));
    require_send(SqliteStorage::memory());
}

/// Bulk child inserts preserve ordinal ordering across every bounded parameter batch.
#[tokio::test]
async fn child_fact_batches_preserve_large_message_roundtrips() {
    use sandpost_core::{Attachment, Mailbox};
    let storage = SqliteStorage::memory().await.unwrap();
    let mut mail = message(0);
    mail.facts.to = (0..300)
        .map(|index| Mailbox {
            address: format!("recipient-{index}@example.test"),
            domain: "example.test".into(),
        })
        .collect();
    mail.facts.headers.insert(
        "x-repeat".into(),
        (0..257).map(|index| index.to_string()).collect(),
    );
    mail.attachments = (0..129)
        .map(|index| Attachment {
            filename: Some(format!("file-{index}")),
            content_type: "application/octet-stream".into(),
            size: index,
            content_hash: index.to_string(),
        })
        .collect();
    mail.facts.attachment_count = 129;
    storage.insert_message(&mail).await.unwrap();
    let restored = storage.get_message(mail.identifier).await.unwrap().unwrap();
    assert_eq!(restored.facts, mail.facts);
    assert_eq!(restored.attachments, mail.attachments);
    storage.close().await;
}

/// The compact recipient projection preserves arbitrary text and rejects invalid stored BLOB fields.
#[tokio::test]
async fn recipient_projection_preserves_text_and_reports_invalid_stored_values() {
    use sandpost_core::Mailbox;
    let storage = SqliteStorage::memory().await.unwrap();
    let mut mail = message(0);
    mail.facts.to = vec![Mailbox {
        address: "quoted\"\\\n\0é@example.test".into(),
        domain: "example.test".into(),
    }];
    storage.insert_message(&mail).await.unwrap();
    assert_eq!(
        storage
            .get_message(mail.identifier)
            .await
            .unwrap()
            .unwrap()
            .facts
            .to,
        mail.facts.to
    );
    sqlx::query("UPDATE mail_recipients SET address=x'FF' WHERE recipient_type='to'")
        .execute(&storage.write_pool)
        .await
        .unwrap();
    assert!(matches!(
        storage.get_message(mail.identifier).await,
        Err(StorageError::InvalidData(_))
    ));
    storage.close().await;
}
