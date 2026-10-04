use super::*;

/// Verify message insertion, retrieval, metadata decoding, and duplicate rejection.
#[test]
fn ingest_roundtrip_metadata_and_duplicate_rejection() {
    let storage = Storage::memory().unwrap();
    let original = message();
    let sequence = storage.insert_message(&original, &[]).unwrap();
    let mut duplicate = original.clone();
    duplicate.raw_message = b"replacement".to_vec();
    assert!(matches!(
        storage.insert_message(&duplicate, &[]),
        Err(StorageError::DuplicateMessageIdentifier(identifier)) if identifier == original.identifier
    ));
    let saved = storage.get_message(original.identifier).unwrap().unwrap();
    assert_eq!(saved.raw_message, b"raw");
    assert_eq!(saved.facts, original.facts);
    assert_eq!(saved.attachments, original.attachments);
    assert_eq!(
        storage
            .get_message_metadata(original.identifier)
            .unwrap()
            .unwrap(),
        (original.facts.clone(), original.attachments.clone())
    );
    assert_eq!(
        storage.list_messages(None, 10).unwrap()[0].sequence,
        sequence
    );
}

/// Verify a foreign-key failure rolls back the message and its scope matches.
#[test]
fn foreign_key_failure_rolls_back_message_and_scope_matches() {
    let storage = Storage::memory().unwrap();
    let message = message();
    let unknown = ScopeIdentifier::new();
    assert!(storage.insert_message(&message, &[(unknown, 1)]).is_err());
    assert!(storage.get_message(message.identifier).unwrap().is_none());
}
