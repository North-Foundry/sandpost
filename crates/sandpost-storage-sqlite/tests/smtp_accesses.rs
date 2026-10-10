//! SMTP credential creation under contention between independent SQLite connections.
use rusqlite::{Connection, TransactionBehavior};
use sandpost_core::SmtpAuthenticationMechanism;
use sandpost_storage::{NewSmtpAccess, SmtpServerStorage, StorageError};
use sandpost_storage_sqlite::SqliteStorage;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Remove a temporary database and its WAL sidecars even when a test fails.
struct TestDatabase(PathBuf);

impl TestDatabase {
    /// Reserve a unique temporary path without sharing state with another test.
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "sandpost-smtp-access-{}.sqlite",
            uuid::Uuid::new_v4()
        )))
    }
}

impl Drop for TestDatabase {
    /// Clean up the database after all test connections have been dropped.
    fn drop(&mut self) {
        for path in [
            self.0.clone(),
            self.0.with_extension("sqlite-wal"),
            self.0.with_extension("sqlite-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Build one submission credential without plaintext password data.
fn new_access(username: &str) -> NewSmtpAccess {
    NewSmtpAccess {
        name: "concurrent SMTP access".to_owned(),
        username: username.to_owned(),
        password_hash: "test-password-hash".to_owned(),
        created_at: 1_700_000_000,
        requires_encryption: false,
        allowed_mechanisms: SmtpAuthenticationMechanism::ALL.to_vec(),
    }
}

/// A contending writer must be awaited before uniqueness is checked, preserving the semantic error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_smtp_creators_wait_and_report_duplicate_usernames() {
    let database = TestDatabase::new();
    let first = SqliteStorage::open(&database.0).expect("open first storage connection");
    let second = SqliteStorage::open(&database.0).expect("open second storage connection");
    let mut writer = Connection::open(&database.0).expect("open contending writer");
    let transaction = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("reserve writer");
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut creators = Vec::new();
    for storage in [first, second] {
        let barrier = Arc::clone(&barrier);
        creators.push(tokio::spawn(async move {
            barrier.wait().await;
            storage
                .create_smtp_access(&new_access("same-username"))
                .await
        }));
    }
    barrier.wait().await;
    // Deferred read-then-write transactions fail immediately against this held writer;
    // immediate transactions wait and inspect current state after the writer is released.
    tokio::time::sleep(Duration::from_millis(100)).await;
    transaction.commit().expect("release contending writer");
    let mut created = 0;
    let mut duplicates = 0;
    for creator in creators {
        match creator.await.expect("creator task completes") {
            Ok(_) => created += 1,
            Err(StorageError::DuplicateSmtpUsername) => duplicates += 1,
            Err(error) => panic!("unexpected SMTP creation error: {error}"),
        }
    }
    assert_eq!(created, 1, "only one credential is created");
    assert_eq!(
        duplicates, 1,
        "the other creator receives the documented duplicate error"
    );
}
