//! Public API regression tests for message persistence and pagination.
#[allow(dead_code)]
mod common;

use common::message;
use sandpost_core::{
    Attachment, GlobalRole, ImapMailboxSource, MailAccess, Mailbox, MessageIdentifier,
    MessageSequence,
};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_storage::{
    ImapMembershipDefinition, ImapStorage, MessageListQuery, MessageStorage, NewImapAccount,
    NewUser, Storage, StorageError, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;
use sqlx::{Connection, Row};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Open an in-memory SQLite backend as the backend-neutral contract object.
async fn open_storage() -> Arc<dyn Storage> {
    Arc::new(
        SqliteStorage::memory()
            .await
            .expect("create in-memory storage"),
    )
}

/// Build a message listing request.
fn list_request(
    filter: Option<Expression>,
    before: Option<MessageSequence>,
    limit: usize,
) -> MessageListQuery {
    MessageListQuery {
        filter,
        before,
        limit,
    }
}

/// Return a unique SQLite path in the system temporary directory.
fn temporary_database_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "sandpost-message-regression-{}.sqlite",
        MessageIdentifier::new()
    ))
}

/// Remove a temporary SQLite database and its write-ahead-log sidecars.
fn remove_temporary_database(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

/// Persist all normalized message parts once and retain later external child updates.
#[tokio::test]
async fn message_parts_roundtrip_and_external_updates_enqueue_revisions() {
    let path = temporary_database_path();
    let storage = SqliteStorage::open(&path)
        .await
        .expect("open temporary database");
    let mut fixture = message("complete message fixture");
    fixture.facts.envelope_from = Some(Mailbox {
        address: "sender@example.test".into(),
        domain: "example.test".into(),
    });
    fixture.facts.envelope_to = vec![Mailbox {
        address: "recipient@example.test".into(),
        domain: "example.test".into(),
    }];
    fixture.facts.from = fixture.facts.envelope_to.clone();
    fixture.facts.to = fixture.facts.envelope_to.clone();
    fixture.facts.carbon_copy = vec![Mailbox {
        address: "copy@example.test".into(),
        domain: "example.test".into(),
    }];
    fixture.facts.text = "plain body".into();
    fixture.facts.markup_body = "<p>formatted body</p>".into();
    fixture.facts.message_identifier = Some("<complete@example.test>".into());
    fixture.facts.received_at = 1_800_000_000;
    fixture.facts.size = 42;
    fixture.facts.headers =
        BTreeMap::from([("x-regression".into(), vec!["first".into(), "second".into()])]);
    fixture.attachments = vec![Attachment {
        filename: Some("report.txt".into()),
        content_type: "text/plain".into(),
        size: 12,
        content_hash: "attachment-hash".into(),
    }];
    fixture.facts.attachment_count = 1;
    fixture.raw_message = b"raw MIME fixture".to_vec();
    let identifier = fixture.identifier;

    let sequence = storage
        .insert_message(&fixture)
        .await
        .expect("insert complete fixture");
    let stored = storage
        .get_message(identifier)
        .await
        .expect("load complete fixture")
        .expect("fixture exists");
    assert_eq!(stored.facts, fixture.facts);
    assert_eq!(stored.attachments, fixture.attachments);
    assert_eq!(stored.raw_message, fixture.raw_message);

    let mut external_connection = common::connection(&path).await;
    let initial_row = sqlx::query(
        "SELECT mail.sequence, mail.search_revision, (SELECT COUNT(*) FROM search_outbox) FROM mail WHERE mail.identifier=?1",
    )
    .bind(identifier.to_string())
    .fetch_one(&mut external_connection)
    .await
    .expect("read initial mail revision and outbox");
    let stored_sequence: i64 = initial_row.try_get(0).unwrap();
    let initial_revision: i64 = initial_row.try_get(1).unwrap();
    let initial_outbox_count: i64 = initial_row.try_get(2).unwrap();
    assert_eq!(stored_sequence, sequence.0 as i64);
    assert_eq!((initial_revision, initial_outbox_count), (1, 1));

    sqlx::query(
        "UPDATE mail_headers SET value='externally changed' WHERE mail_sequence=?1 AND name='x-regression' AND ordinal=0",
    )
    .bind(stored_sequence)
    .execute(&mut external_connection)
    .await
    .expect("update child row through external connection");
    let updated_row = sqlx::query(
        "SELECT mail.search_revision, (SELECT COUNT(*) FROM search_outbox) FROM mail WHERE mail.sequence=?1",
    )
    .bind(stored_sequence)
    .fetch_one(&mut external_connection)
    .await
    .expect("read revision after external child update");
    let updated_revision: i64 = updated_row.try_get(0).unwrap();
    let updated_outbox_count: i64 = updated_row.try_get(1).unwrap();
    assert_eq!((updated_revision, updated_outbox_count), (2, 2));

    external_connection.close().await.unwrap();
    drop(storage);
    remove_temporary_database(&path);
}

/// Roll back duplicate and malformed writes, then accept a valid write.
#[tokio::test]
async fn failed_message_writes_leave_no_partial_rows_or_outbox_operations() {
    let path = temporary_database_path();
    let storage = SqliteStorage::open(&path)
        .await
        .expect("open temporary database");
    let mut fixture = message("transaction fixture");
    fixture.facts.envelope_to.push(Mailbox {
        address: "recipient@example.test".into(),
        domain: "example.test".into(),
    });
    fixture
        .facts
        .headers
        .insert("x-test".into(), vec!["value".into()]);
    fixture.attachments.push(Attachment {
        filename: Some("valid.bin".into()),
        content_type: "application/octet-stream".into(),
        size: 1,
        content_hash: "valid-hash".into(),
    });
    storage
        .insert_message(&fixture)
        .await
        .expect("insert original message");

    let mut duplicate = fixture.clone();
    duplicate.facts.headers.clear();
    assert!(matches!(
        storage.insert_message(&duplicate).await,
        Err(StorageError::DuplicateMessageIdentifier(_))
    ));

    let mut invalid_attachment = message("invalid attachment transaction");
    invalid_attachment.facts.envelope_to = fixture.facts.envelope_to.clone();
    invalid_attachment.facts.headers = fixture.facts.headers.clone();
    invalid_attachment.attachments = vec![
        fixture.attachments[0].clone(),
        Attachment {
            filename: Some("too-large.bin".into()),
            content_type: "application/octet-stream".into(),
            size: u64::MAX,
            content_hash: "too-large-hash".into(),
        },
    ];
    let invalid_identifier = invalid_attachment.identifier;
    assert!(matches!(
        storage.insert_message(&invalid_attachment).await,
        Err(StorageError::IntegerRange)
    ));

    let mut external_connection = common::connection(&path).await;
    let counts_row = sqlx::query(
        "SELECT (SELECT COUNT(*) FROM mail), (SELECT COUNT(*) FROM mail_recipients), (SELECT COUNT(*) FROM mail_headers), (SELECT COUNT(*) FROM mail_attachments), (SELECT COUNT(*) FROM search_outbox)",
    )
    .fetch_one(&mut external_connection)
    .await
    .expect("count rows after rolled-back inserts");
    let stored_counts = (
        counts_row.try_get::<i64, _>(0).unwrap(),
        counts_row.try_get::<i64, _>(1).unwrap(),
        counts_row.try_get::<i64, _>(2).unwrap(),
        counts_row.try_get::<i64, _>(3).unwrap(),
        counts_row.try_get::<i64, _>(4).unwrap(),
    );
    assert_eq!(stored_counts, (1, 1, 1, 1, 1));
    assert!(
        storage
            .get_message(invalid_identifier)
            .await
            .unwrap()
            .is_none()
    );

    let successful_sequence = storage
        .insert_message(&message("write after rollback"))
        .await
        .expect("foreign keys remain enabled after failed transactions");
    assert_eq!(successful_sequence, MessageSequence(2));

    external_connection.close().await.unwrap();
    drop(storage);
    remove_temporary_database(&path);
}

/// Allocate unique sequences across independent connections and never reuse a deleted sequence.
#[tokio::test]
async fn independent_connections_reserve_distinct_non_reusable_sequences() {
    let path = temporary_database_path();
    let first_storage = Arc::new(
        SqliteStorage::open(&path)
            .await
            .expect("open first connection"),
    );
    let second_storage = Arc::new(
        SqliteStorage::open(&path)
            .await
            .expect("open second connection"),
    );
    let first_message = message("first concurrent message");
    let second_message = message("second concurrent message");

    let (first_sequence, second_sequence) = tokio::join!(
        first_storage.insert_message(&first_message),
        second_storage.insert_message(&second_message),
    );
    let first_sequence = first_sequence.expect("insert through first connection");
    let second_sequence = second_sequence.expect("insert through second connection");
    assert_ne!(first_sequence, second_sequence);

    let (deleted_identifier, highest_sequence) = if first_sequence > second_sequence {
        (first_message.identifier, first_sequence)
    } else {
        (second_message.identifier, second_sequence)
    };
    assert!(
        first_storage
            .delete_message(deleted_identifier)
            .await
            .unwrap()
    );
    let reinserted_sequence = first_storage
        .insert_message(&message("reinserted message"))
        .await
        .expect("reinsert after delete");
    assert!(reinserted_sequence > highest_sequence);

    drop(first_storage);
    drop(second_storage);
    remove_temporary_database(&path);
}

/// Reject a mail size outside SQLite's integer range without storing a mail row.
#[tokio::test]
async fn oversized_mail_size_leaves_no_message() {
    let storage = open_storage().await;
    let mut invalid_message = message("oversized mail size");
    invalid_message.facts.size = u64::MAX;
    let identifier = invalid_message.identifier;

    assert!(matches!(
        storage.insert_message(&invalid_message).await,
        Err(StorageError::IntegerRange)
    ));
    assert!(
        storage
            .get_message(identifier)
            .await
            .expect("look up rejected message")
            .is_none()
    );
}

/// Roll back a mail row and earlier children when an attachment size exceeds SQLite's range.
#[tokio::test]
async fn oversized_attachment_size_rolls_back_mail_and_children() {
    let storage = open_storage().await;
    let mut invalid_message = message("oversized attachment size");
    invalid_message.attachments.push(Attachment {
        filename: Some("too-large.bin".into()),
        content_type: "application/octet-stream".into(),
        size: u64::MAX,
        content_hash: "hash".into(),
    });
    let identifier = invalid_message.identifier;

    assert!(matches!(
        storage.insert_message(&invalid_message).await,
        Err(StorageError::IntegerRange)
    ));
    assert!(
        storage
            .get_message(identifier)
            .await
            .expect("look up rolled-back mail")
            .is_none()
    );
}

/// Enforce newest-first cursors, empty zero-sized pages, and the maximum page size.
#[tokio::test]
async fn message_pages_obey_cursor_and_limit_bounds() {
    let storage = open_storage().await;
    for index in 0..105 {
        common::insert_message(storage.as_ref(), format!("message {index}")).await;
    }

    assert!(
        storage
            .list_messages(list_request(None, None, 0))
            .await
            .expect("list zero messages")
            .is_empty()
    );
    let maximum_page = storage
        .list_messages(list_request(None, None, usize::MAX))
        .await
        .expect("list maximum page");
    assert_eq!(maximum_page.len(), 100);
    assert_eq!(maximum_page.first().unwrap().sequence, MessageSequence(105));
    assert_eq!(maximum_page.last().unwrap().sequence, MessageSequence(6));

    let earlier_page = storage
        .list_messages(list_request(None, Some(MessageSequence(10)), 100))
        .await
        .expect("list before cursor");
    assert_eq!(
        earlier_page
            .iter()
            .map(|summary| summary.sequence)
            .collect::<Vec<_>>(),
        (1..10).rev().map(MessageSequence).collect::<Vec<_>>()
    );
    assert!(matches!(
        storage
            .list_messages(list_request(None, Some(MessageSequence(u64::MAX)), 1))
            .await,
        Err(StorageError::IntegerRange)
    ));
}

/// Return none for unknown identifiers in both public message lookup methods.
#[tokio::test]
async fn missing_message_lookups_return_none() {
    let storage = open_storage().await;
    let missing_identifier = MessageIdentifier::new();

    assert!(
        storage
            .get_message(missing_identifier)
            .await
            .expect("look up missing message")
            .is_none()
    );
    assert!(
        storage
            .get_message_metadata(missing_identifier)
            .await
            .expect("look up missing message metadata")
            .is_none()
    );
}

/// Re-express the removed inbox listing through a filtered message query.
#[tokio::test]
async fn recipient_listing_uses_a_message_filter() {
    let storage = open_storage().await;

    let mut matching = message("recipient match");
    matching.facts.envelope_to = vec![Mailbox {
        address: "inbox@sandpost.local".into(),
        domain: "sandpost.local".into(),
    }];
    let matching_identifier = matching.identifier;
    storage
        .insert_message(&matching)
        .await
        .expect("insert matching message");
    storage
        .insert_message(&message("recipient miss"))
        .await
        .expect("insert non-matching message");

    let filter = Expression::Predicate(Predicate {
        field: Field::EnvelopeToAddress,
        operator: Operator::Equal,
        value: Value::String("INBOX@sandpost.local".into()),
    });
    let rows = storage
        .list_messages(list_request(Some(filter), None, 100))
        .await
        .expect("list filtered messages");
    assert_eq!(
        rows.iter()
            .map(|summary| summary.identifier)
            .collect::<Vec<_>>(),
        vec![matching_identifier]
    );
}

/// Persist user tombstones and remove all of that user's IMAP memberships atomically.
#[tokio::test]
async fn user_message_deletions_persist_and_clear_owned_imap_memberships() {
    let path = temporary_database_path();
    let storage = SqliteStorage::open(&path)
        .await
        .expect("open temporary database");
    let deleting_user = storage
        .create_user(&NewUser {
            name: "deleting user".into(),
            email: "deleting-user@example.test".into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .expect("create deleting user");
    let other_user = storage
        .create_user(&NewUser {
            name: "other user".into(),
            email: "other-user@example.test".into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .expect("create other user");
    let fixture = message("per-user deletion fixture");
    let identifier = fixture.identifier;
    let sequence = storage
        .insert_message(&fixture)
        .await
        .expect("insert shared message");
    let oversized_batch = (0..257)
        .map(|_| MessageIdentifier::new())
        .collect::<Vec<_>>();
    assert!(matches!(
        storage
            .delete_user_messages(
                deleting_user.identifier,
                Some(&oversized_batch),
                &Expression::True,
            )
            .await,
        Err(StorageError::BatchLimitExceeded(256))
    ));
    let deleting_account = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: deleting_user.identifier,
            name: "deleting account".into(),
            username: "deleting-account".into(),
            password_hash: "hash".into(),
            mirror_views: false,
            created_at: 1,
        })
        .await
        .expect("create deleting user's account");
    let other_account = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: other_user.identifier,
            name: "other account".into(),
            username: "other-account".into(),
            password_hash: "hash".into(),
            mirror_views: false,
            created_at: 1,
        })
        .await
        .expect("create other user's account");
    let deleting_second_account = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: deleting_user.identifier,
            name: "second deleting account".into(),
            username: "deleting-account-two".into(),
            password_hash: "hash".into(),
            mirror_views: false,
            created_at: 1,
        })
        .await
        .expect("create second account for deleting user");
    let definition = ImapMembershipDefinition {
        filter: Expression::True,
        fingerprint: "all-mail".into(),
    };
    let deleting_mailbox = storage
        .reconcile_imap_mailboxes(deleting_account.identifier, &[ImapMailboxSource::Inbox])
        .await
        .expect("create deleting user's mailbox")[0]
        .identifier;
    let other_mailbox = storage
        .reconcile_imap_mailboxes(other_account.identifier, &[ImapMailboxSource::Inbox])
        .await
        .expect("create other user's mailbox")[0]
        .identifier;
    let deleting_second_mailbox = storage
        .reconcile_imap_mailboxes(
            deleting_second_account.identifier,
            &[ImapMailboxSource::Inbox],
        )
        .await
        .expect("create second mailbox for deleting user")[0]
        .identifier;
    storage
        .synchronize_imap_mailbox(deleting_mailbox, &definition)
        .await
        .expect("synchronize deleting user's mailbox");
    storage
        .synchronize_imap_mailbox(other_mailbox, &definition)
        .await
        .expect("synchronize other user's mailbox");
    storage
        .synchronize_imap_mailbox(deleting_second_mailbox, &definition)
        .await
        .expect("synchronize second mailbox for deleting user");
    assert_eq!(
        storage
            .imap_mailbox_members(deleting_mailbox, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        storage
            .imap_mailbox_members(other_mailbox, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        storage
            .imap_mailbox_members(deleting_second_mailbox, 0)
            .await
            .unwrap()
            .len(),
        1
    );

    assert_eq!(
        storage
            .delete_user_messages(
                deleting_user.identifier,
                Some(&[identifier]),
                &Expression::True,
            )
            .await
            .expect("tombstone shared message for one user"),
        1
    );
    assert!(
        storage
            .imap_mailbox_members(deleting_mailbox, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        storage
            .imap_mailbox_members(deleting_second_mailbox, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        storage
            .imap_mailbox_members(other_mailbox, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        storage
            .get_user_message(deleting_user.identifier, identifier)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .get_user_message(other_user.identifier, identifier)
            .await
            .unwrap()
            .is_some()
    );
    drop(storage);

    let reopened = SqliteStorage::open(&path)
        .await
        .expect("reopen database with tombstone");
    assert!(
        reopened
            .get_user_message(deleting_user.identifier, identifier)
            .await
            .unwrap()
            .is_none()
    );
    assert!(reopened.get_message(identifier).await.unwrap().is_some());
    assert!(sequence.0 > 0);
    drop(reopened);
    remove_temporary_database(&path);
}

/// Reject a user deletion request when its user no longer exists.
#[tokio::test]
async fn deleting_messages_for_an_unknown_user_returns_not_found() {
    let storage = open_storage().await;
    assert!(matches!(
        storage
            .delete_user_messages(
                sandpost_core::UserIdentifier::new(),
                None,
                &Expression::True,
            )
            .await,
        Err(StorageError::NotFound)
    ));
}
