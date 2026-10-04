//! Public API regression tests for message persistence and pagination.
#[allow(dead_code)]
mod common;

use common::message;
use sandpost_core::{Attachment, MessageIdentifier, MessageSequence};
use sandpost_storage::{Storage, StorageError};

/// Reject a mail size outside SQLite's integer range without storing a mail row.
#[test]
fn oversized_mail_size_leaves_no_message() {
    let storage = Storage::memory().expect("create in-memory storage");
    let mut invalid_message = message("oversized mail size");
    invalid_message.facts.size = u64::MAX;
    let identifier = invalid_message.identifier;

    assert!(matches!(
        storage.insert_message(&invalid_message, &[]),
        Err(StorageError::IntegerRange)
    ));
    assert!(
        storage
            .get_message(identifier)
            .expect("look up rejected message")
            .is_none()
    );
}

/// Roll back a mail row and earlier children when an attachment size exceeds SQLite's range.
#[test]
fn oversized_attachment_size_rolls_back_mail_and_children() {
    let storage = Storage::memory().expect("create in-memory storage");
    let mut invalid_message = message("oversized attachment size");
    invalid_message.attachments.push(Attachment {
        filename: Some("too-large.bin".into()),
        content_type: "application/octet-stream".into(),
        size: u64::MAX,
        content_hash: "hash".into(),
    });
    let identifier = invalid_message.identifier;

    assert!(matches!(
        storage.insert_message(&invalid_message, &[]),
        Err(StorageError::IntegerRange)
    ));
    assert!(
        storage
            .get_message(identifier)
            .expect("look up rolled-back mail")
            .is_none()
    );
}

/// Enforce newest-first cursors, empty zero-sized pages, and the maximum page size.
#[test]
fn message_pages_obey_cursor_and_limit_bounds() {
    let storage = Storage::memory().expect("create in-memory storage");
    for index in 0..105 {
        common::insert_message(&storage, format!("message {index}"));
    }

    assert!(
        storage
            .list_messages(None, 0)
            .expect("list zero messages")
            .is_empty()
    );
    let maximum_page = storage
        .list_messages(None, usize::MAX)
        .expect("list maximum page");
    assert_eq!(maximum_page.len(), 100);
    assert_eq!(maximum_page.first().unwrap().sequence, MessageSequence(105));
    assert_eq!(maximum_page.last().unwrap().sequence, MessageSequence(6));

    let earlier_page = storage
        .list_messages(Some(MessageSequence(10)), 100)
        .expect("list before cursor");
    assert_eq!(
        earlier_page
            .iter()
            .map(|summary| summary.sequence)
            .collect::<Vec<_>>(),
        (1..10).rev().map(MessageSequence).collect::<Vec<_>>()
    );
    assert!(matches!(
        storage.list_messages(Some(MessageSequence(u64::MAX)), 1),
        Err(StorageError::IntegerRange)
    ));
}

/// Return `None` for unknown identifiers in both public message lookup methods.
#[test]
fn missing_message_lookups_return_none() {
    let storage = Storage::memory().expect("create in-memory storage");
    let missing_identifier = MessageIdentifier::new();

    assert!(
        storage
            .get_message(missing_identifier)
            .expect("look up missing message")
            .is_none()
    );
    assert!(
        storage
            .get_message_metadata(missing_identifier)
            .expect("look up missing message metadata")
            .is_none()
    );
}
