//! Public API regression tests for message persistence and pagination.
#[allow(dead_code)]
mod common;

use common::message;
use sandpost_core::{
    Attachment, EndpointIdentifier, Mailbox, MessageIdentifier, MessageSequence,
    default_endpoint_identifier,
};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_storage::{MessageListQuery, Storage, StorageError};
use sandpost_storage_sqlite::SqliteStorage;
use std::sync::Arc;

/// Open an in-memory SQLite backend as the backend-neutral contract object.
fn open_storage() -> Arc<dyn Storage> {
    Arc::new(SqliteStorage::memory().expect("create in-memory storage"))
}

/// Build a message listing request.
fn list_request(
    endpoint: Option<EndpointIdentifier>,
    filter: Option<Expression>,
    before: Option<MessageSequence>,
    limit: usize,
) -> MessageListQuery {
    MessageListQuery {
        endpoint,
        filter,
        before,
        limit,
    }
}

/// Reject a mail size outside SQLite's integer range without storing a mail row.
#[tokio::test]
async fn oversized_mail_size_leaves_no_message() {
    let storage = open_storage();
    let mut invalid_message = message("oversized mail size");
    invalid_message.facts.size = u64::MAX;
    let identifier = invalid_message.identifier;

    assert!(matches!(
        storage
            .insert_message(&invalid_message, default_endpoint_identifier())
            .await,
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
    let storage = open_storage();
    let mut invalid_message = message("oversized attachment size");
    invalid_message.attachments.push(Attachment {
        filename: Some("too-large.bin".into()),
        content_type: "application/octet-stream".into(),
        size: u64::MAX,
        content_hash: "hash".into(),
    });
    let identifier = invalid_message.identifier;

    assert!(matches!(
        storage
            .insert_message(&invalid_message, default_endpoint_identifier())
            .await,
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
    let storage = open_storage();
    for index in 0..105 {
        common::insert_message(storage.as_ref(), format!("message {index}")).await;
    }

    assert!(
        storage
            .list_messages(list_request(None, None, None, 0))
            .await
            .expect("list zero messages")
            .is_empty()
    );
    let maximum_page = storage
        .list_messages(list_request(None, None, None, usize::MAX))
        .await
        .expect("list maximum page");
    assert_eq!(maximum_page.len(), 100);
    assert_eq!(maximum_page.first().unwrap().sequence, MessageSequence(105));
    assert_eq!(maximum_page.last().unwrap().sequence, MessageSequence(6));

    let earlier_page = storage
        .list_messages(list_request(None, None, Some(MessageSequence(10)), 100))
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
            .list_messages(list_request(None, None, Some(MessageSequence(u64::MAX)), 1))
            .await,
        Err(StorageError::IntegerRange)
    ));
}

/// Return none for unknown identifiers in both public message lookup methods.
#[tokio::test]
async fn missing_message_lookups_return_none() {
    let storage = open_storage();
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
    let storage = open_storage();
    let endpoint = default_endpoint_identifier();

    let mut matching = message("recipient match");
    matching.facts.envelope_to = vec![Mailbox {
        address: "inbox@sandpost.local".into(),
        domain: "sandpost.local".into(),
    }];
    let matching_identifier = matching.identifier;
    storage
        .insert_message(&matching, endpoint)
        .await
        .expect("insert matching message");
    storage
        .insert_message(&message("recipient miss"), endpoint)
        .await
        .expect("insert non-matching message");

    let filter = Expression::Predicate(Predicate {
        field: Field::EnvelopeToAddress,
        operator: Operator::Equal,
        value: Value::String("INBOX@sandpost.local".into()),
    });
    let rows = storage
        .list_messages(list_request(Some(endpoint), Some(filter), None, 100))
        .await
        .expect("list filtered messages");
    assert_eq!(
        rows.iter()
            .map(|summary| summary.identifier)
            .collect::<Vec<_>>(),
        vec![matching_identifier]
    );
}
