//! Snapshot consistency and transaction rollback tests for SQLite message storage.
use super::*;
use sandpost_core::{Attachment, Mailbox};
use sandpost_storage::{MessageStorage, SearchSynchronizationStorage, StorageError};
use sqlx::{Row, SqlitePool};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

/// Keep a temporary file database alive until its pools have closed.
struct TestDirectory(PathBuf);

impl TestDirectory {
    /// Create a unique directory for one file-backed pool test.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sandpost-message-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// Return the database file inside this test's directory.
    fn database(&self) -> PathBuf {
        self.0.join("mail.sqlite3")
    }
}

impl Drop for TestDirectory {
    /// Remove the temporary database and WAL files after pool shutdown.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build a message with recipient, header, and attachment rows for hydration and rollback checks.
fn message(subject: &str) -> Message {
    Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            envelope_from: Some(mailbox("sender@example.test")),
            envelope_to: vec![mailbox("recipient@example.test")],
            from: vec![mailbox("sender@example.test")],
            to: vec![mailbox("old@example.test")],
            subject: subject.to_owned(),
            size: 4,
            headers: [("x-test".to_owned(), vec!["initial".to_owned()])]
                .into_iter()
                .collect(),
            ..Default::default()
        },
        raw_message: b"raw\0".to_vec(),
        attachments: vec![Attachment {
            filename: Some("file.bin".to_owned()),
            content_type: "application/octet-stream".to_owned(),
            size: 3,
            content_hash: "test-hash".to_owned(),
        }],
    }
}

/// Split a mailbox address into the stored address and domain fields.
fn mailbox(address: &str) -> Mailbox {
    let domain = address.split_once('@').unwrap().1;
    Mailbox {
        address: address.to_owned(),
        domain: domain.to_owned(),
    }
}

/// Count committed mail rows and each normalized child table from one query.
async fn stored_row_counts(pool: &SqlitePool) -> (i64, i64, i64, i64, i64) {
    let row = sqlx::query(
        "SELECT (SELECT COUNT(*) FROM mail) AS mail_count, (SELECT COUNT(*) FROM mail_recipients) AS recipient_count, (SELECT COUNT(*) FROM mail_headers) AS header_count, (SELECT COUNT(*) FROM mail_attachments) AS attachment_count, (SELECT COUNT(*) FROM search_outbox) AS outbox_count",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (
        row.try_get("mail_count").unwrap(),
        row.try_get("recipient_count").unwrap(),
        row.try_get("header_count").unwrap(),
        row.try_get("attachment_count").unwrap(),
        row.try_get("outbox_count").unwrap(),
    )
}

/// A read transaction keeps child hydration on the same snapshot after a writer commits.
#[tokio::test]
async fn child_hydration_uses_the_parent_read_snapshot() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let directory = TestDirectory::new();
        let storage = SqliteStorage::open(directory.database()).await.unwrap();
        let mail = message("generation-0");
        let sequence = storage.insert_message(&mail).await.unwrap();

        let mut snapshot = storage.read_pool.begin().await.unwrap();
        let parent = sqlx::query(
            "SELECT subject,search_revision FROM mail WHERE identifier=?",
        )
        .bind(mail.identifier.to_string())
        .fetch_one(&mut *snapshot)
        .await
        .unwrap();
        let parent_subject: String = parent.try_get("subject").unwrap();
        let parent_revision: i64 = parent.try_get("search_revision").unwrap();

        let mut writer = storage
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .unwrap();
        sqlx::query("UPDATE mail SET subject='generation-1' WHERE identifier=?")
            .bind(mail.identifier.to_string())
            .execute(&mut *writer)
            .await
            .unwrap();
        sqlx::query("DELETE FROM mail_recipients WHERE mail_sequence=? AND recipient_type='to'")
            .bind(i64::try_from(sequence.0).unwrap())
            .execute(&mut *writer)
            .await
            .unwrap();
        sqlx::query("INSERT INTO mail_recipients(mail_sequence,recipient_type,ordinal,address,domain) VALUES(?, 'to', 0, 'new@example.test', 'example.test')")
            .bind(i64::try_from(sequence.0).unwrap())
            .execute(&mut *writer)
            .await
            .unwrap();
        writer.commit().await.unwrap();

        let stored_sequence = i64::try_from(sequence.0).unwrap();
        let recipients = parts::load_recipients(&mut snapshot, &[stored_sequence])
        .await
        .unwrap();
        let recipients = recipients.get(&stored_sequence).unwrap();
        assert_eq!(parent_subject, "generation-0");
        assert_eq!(parent_revision, 1);
        assert_eq!(recipients.to, vec![mailbox("old@example.test")]);

        let snapshot_subject: String = sqlx::query_scalar("SELECT subject FROM mail WHERE identifier=?")
            .bind(mail.identifier.to_string())
            .fetch_one(&mut *snapshot)
            .await
            .unwrap();
        assert_eq!(snapshot_subject, "generation-0");
        snapshot.commit().await.unwrap();

        let current = storage
            .indexed_messages(&[mail.identifier])
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert!(current.revision > parent_revision as u64);
        assert_eq!(current.facts.subject, "generation-1");
        assert_eq!(current.facts.to, vec![mailbox("new@example.test")]);
        storage.close().await;
    })
    .await
    .expect("snapshot hydration must finish within its bound");
}

/// Public hydrators observe complete committed generations while a writer holds its update open.
#[tokio::test]
async fn public_hydrators_stay_coherent_during_parent_and_child_commits() {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let directory = TestDirectory::new();
        let storage = SqliteStorage::open(directory.database()).await.unwrap();
        let mail = message("generation-0");
        let sequence = storage.insert_message(&mail).await.unwrap();
        let initial = storage
            .indexed_messages(&[mail.identifier])
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(initial.facts.to, vec![mailbox("old@example.test")]);

        let reader_storage = storage.clone();
        let identifier = mail.identifier;
        let (request_sender, mut request_receiver) =
            mpsc::channel::<((String, String, u64), oneshot::Sender<()>)>(1);
        let reader = tokio::spawn(async move {
            while let Some(((subject, recipient, revision), finished)) =
                request_receiver.recv().await
            {
                let metadata = reader_storage
                    .get_message_metadata(identifier)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(metadata.0.subject, subject);
                assert_eq!(metadata.0.to, vec![mailbox(&recipient)]);

                let indexed = reader_storage
                    .indexed_messages(&[identifier])
                    .await
                    .unwrap()
                    .pop()
                    .unwrap();
                assert_eq!(indexed.revision, revision);
                assert_eq!(indexed.facts.subject, subject);
                assert_eq!(indexed.facts.to, vec![mailbox(&recipient)]);

                let hydrated = reader_storage
                    .hydrate_current_messages(&[(identifier, revision)])
                    .await
                    .unwrap();
                assert_eq!(hydrated.len(), 1);
                assert_eq!(hydrated[0].subject, subject);
                assert_eq!(hydrated[0].to, vec![mailbox(&recipient)]);
                finished.send(()).unwrap();
            }
        });

        let mut expected_subject = "generation-0".to_owned();
        let mut expected_recipient = "old@example.test".to_owned();
        let mut expected_revision = initial.revision;
        for generation in 1..=3 {
            let mut writer = storage
                .write_pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .unwrap();
            let next_subject = format!("generation-{generation}");
            let next_recipient = format!("recipient-{generation}@example.test");
            sqlx::query("UPDATE mail SET subject=? WHERE identifier=?")
                .bind(&next_subject)
                .bind(identifier.to_string())
                .execute(&mut *writer)
                .await
                .unwrap();
            sqlx::query("DELETE FROM mail_recipients WHERE mail_sequence=? AND recipient_type='to'")
                .bind(i64::try_from(sequence.0).unwrap())
                .execute(&mut *writer)
                .await
                .unwrap();
            sqlx::query("INSERT INTO mail_recipients(mail_sequence,recipient_type,ordinal,address,domain) VALUES(?, 'to', 0, ?, 'example.test')")
                .bind(i64::try_from(sequence.0).unwrap())
                .bind(&next_recipient)
                .execute(&mut *writer)
                .await
                .unwrap();

            let (finished_sender, finished_receiver) = oneshot::channel();
            request_sender
                .send((
                    (
                        expected_subject.clone(),
                        expected_recipient.clone(),
                        expected_revision,
                    ),
                    finished_sender,
                ))
                .await
                .unwrap();
            finished_receiver.await.unwrap();
            writer.commit().await.unwrap();

            let current = storage
                .indexed_messages(&[identifier])
                .await
                .unwrap()
                .pop()
                .unwrap();
            expected_subject = next_subject;
            expected_recipient = next_recipient;
            expected_revision = current.revision;
            assert_eq!(current.facts.subject, expected_subject);
            assert_eq!(current.facts.to, vec![mailbox(&expected_recipient)]);
        }
        drop(request_sender);
        reader.await.unwrap();

        let stale = storage
            .hydrate_current_messages(&[(identifier, initial.revision)])
            .await
            .unwrap();
        assert!(stale.is_empty());
        let latest = storage
            .indexed_messages(&[identifier])
            .await
            .unwrap()
            .pop()
            .unwrap();
        let current = storage
            .hydrate_current_messages(&[(identifier, latest.revision)])
            .await
            .unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].subject, expected_subject);
        assert_eq!(current[0].to, vec![mailbox(&expected_recipient)]);
        storage.close().await;
    })
    .await
    .expect("concurrent public reads must finish within their bound");
}

/// A failing attachment insert rolls back earlier children, mail, and the outbox revision.
#[tokio::test]
async fn failed_child_insert_rolls_back_the_complete_message() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let directory = TestDirectory::new();
        let storage = SqliteStorage::open(directory.database()).await.unwrap();
        let mut connection = storage.write_pool.acquire().await.unwrap();
        sqlx::query("CREATE TEMP TRIGGER fail_test_attachment BEFORE INSERT ON mail_attachments WHEN NEW.filename='fail.bin' BEGIN SELECT RAISE(ABORT, 'forced attachment failure'); END")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);

        let mut failed = message("must-rollback");
        failed.attachments[0].filename = Some("fail.bin".to_owned());
        assert!(matches!(
            storage.insert_message(&failed).await,
            Err(StorageError::ConstraintViolation(_))
        ));
        assert_eq!(stored_row_counts(&storage.read_pool).await, (0, 0, 0, 0, 0));
        let status = storage.search_status().await.unwrap();
        assert_eq!(
            (status.latest_sequence, status.indexed_sequence, status.pending_operations),
            (0, 0, 0)
        );

        let mut connection = storage.write_pool.acquire().await.unwrap();
        sqlx::query("DROP TRIGGER fail_test_attachment")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        assert_eq!(storage.insert_message(&message("valid-after-rollback")).await.unwrap().0, 1);
        assert_eq!(stored_row_counts(&storage.read_pool).await, (1, 4, 1, 1, 1));
        storage.close().await;
    })
    .await
    .expect("the insertion rollback test must finish within its bound");
}

/// An abort during cascading child deletion restores the parent, children, and outbox state.
#[tokio::test]
async fn failed_child_delete_rolls_back_message_deletion() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let directory = TestDirectory::new();
        let storage = SqliteStorage::open(directory.database()).await.unwrap();
        let mail = message("must-survive");
        storage.insert_message(&mail).await.unwrap();
        let before = storage.search_status().await.unwrap();
        let counts_before = stored_row_counts(&storage.read_pool).await;

        let mut connection = storage.write_pool.acquire().await.unwrap();
        sqlx::query("CREATE TEMP TRIGGER fail_test_recipient_delete BEFORE DELETE ON mail_recipients BEGIN SELECT RAISE(ABORT, 'forced recipient delete failure'); END")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);

        assert!(matches!(
            storage.delete_message(mail.identifier).await,
            Err(StorageError::ConstraintViolation(_))
        ));
        assert_eq!(stored_row_counts(&storage.read_pool).await, counts_before);
        assert!(storage.get_message(mail.identifier).await.unwrap().is_some());
        assert_eq!(storage.search_status().await.unwrap(), before);

        let mut connection = storage.write_pool.acquire().await.unwrap();
        sqlx::query("DROP TRIGGER fail_test_recipient_delete")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        assert!(storage.delete_message(mail.identifier).await.unwrap());
        assert_eq!(stored_row_counts(&storage.read_pool).await, (0, 0, 0, 0, 2));
        assert_eq!(storage.search_status().await.unwrap().latest_sequence, 2);
        storage.close().await;
    })
    .await
    .expect("the deletion rollback test must finish within its bound");
}
