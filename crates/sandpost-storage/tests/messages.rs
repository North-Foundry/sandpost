//! Public API regression tests for message persistence and pagination.
#[allow(dead_code)]
mod common;

use common::message;
use sandpost_core::{MessageIdentifier, MessageSequence};
use sandpost_storage::{Storage, StorageError};

/// Reject unsigned summary counts outside SQLite's integer range without storing a message.
#[test]
fn oversized_unsigned_counts_leave_no_message() {
    let storage = Storage::memory().expect("create in-memory storage");

    for oversized_field in ["size", "attachment_count"] {
        let mut invalid_message = message(oversized_field);
        if oversized_field == "size" {
            invalid_message.facts.size = u64::MAX;
        } else {
            invalid_message.facts.attachment_count = u64::MAX;
        }
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
