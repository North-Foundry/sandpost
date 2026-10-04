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

/// Roundtrip every normalized mail child while preserving role order and deriving attachment count.
#[test]
fn normalized_mail_children_roundtrip_in_order() {
    let storage = Storage::memory().unwrap();
    let mut original = normalized_message();
    let expected_attachment_count = original.attachments.len() as u64;
    let scope = scope(1);
    storage.save_scope(&scope).unwrap();
    let sequence = storage
        .insert_message(&original, &[(scope.identifier, 1)])
        .unwrap();

    let mut expected_facts = original.facts.clone();
    expected_facts.attachment_count = expected_attachment_count;
    original.facts.attachment_count = expected_attachment_count;
    let saved = storage.get_message(original.identifier).unwrap().unwrap();
    assert_eq!(saved.identifier, original.identifier);
    assert_eq!(saved.facts, expected_facts);
    assert_eq!(saved.raw_message, original.raw_message);
    assert_eq!(saved.attachments, original.attachments);
    assert_eq!(
        storage.get_message_metadata(original.identifier).unwrap(),
        Some((expected_facts, original.attachments.clone()))
    );

    let listed = storage.list_messages(None, 10).unwrap();
    let visible = storage
        .list_visible_messages(&[scope.identifier], None, 10)
        .unwrap();
    for summary in [listed.first().unwrap(), visible.first().unwrap()] {
        assert_eq!(summary.sequence, sequence);
        assert_eq!(summary.from, original.facts.from);
        assert_eq!(summary.to, original.facts.to);
        assert_eq!(summary.attachment_count, expected_attachment_count);
    }

    let connection = storage.connection().unwrap();
    let recipients: Vec<(String, i64, String)> = connection
        .prepare(
            "SELECT recipient_type, ordinal, address FROM mail_recipients WHERE mail_sequence=?1 ORDER BY recipient_type, ordinal",
        )
        .unwrap()
        .query_map([sequence.0 as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        recipients,
        vec![
            ("carbon_copy".into(), 0, "reader@example.net".into()),
            ("carbon_copy".into(), 1, "reader@example.net".into()),
            ("envelope_to".into(), 0, "shared@example.test".into()),
            ("envelope_to".into(), 1, "envelope@example.net".into()),
            ("from".into(), 0, "shared@example.test".into()),
            ("from".into(), 1, "sender@example.org".into()),
            ("to".into(), 0, "shared@example.test".into()),
            ("to".into(), 1, "reader@example.net".into()),
        ]
    );
    let headers: Vec<(String, String, i64)> = connection
        .prepare(
            "SELECT name, value, ordinal FROM mail_headers WHERE mail_sequence=?1 ORDER BY name, ordinal",
        )
        .unwrap()
        .query_map([sequence.0 as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        headers,
        vec![
            ("subject".into(), "header subject".into(), 0),
            ("x-repeat".into(), "first".into(), 0),
            ("x-repeat".into(), "second".into(), 1),
        ]
    );
    let attachments: Vec<(i64, Option<String>, String, i64, String)> = connection
        .prepare(
            "SELECT ordinal, filename, content_type, size, content_hash FROM mail_attachments WHERE mail_sequence=?1 ORDER BY ordinal",
        )
        .unwrap()
        .query_map([sequence.0 as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        attachments,
        vec![
            (
                0,
                Some("first.bin".into()),
                "application/octet-stream".into(),
                7,
                "first-hash".into()
            ),
            (1, None, "text/plain".into(), 11, "second-hash".into()),
        ]
    );
}

/// Roll back the mail and earlier child rows when a later normalized child insert fails.
#[test]
fn child_write_failure_rolls_back_mail_and_children() {
    let storage = Storage::memory().unwrap();
    let original = normalized_message();
    storage
        .connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_mail_attachment BEFORE INSERT ON mail_attachments BEGIN SELECT RAISE(ABORT, 'attachment write failed'); END",
        )
        .unwrap();

    assert!(storage.insert_message(&original, &[]).is_err());
    let connection = storage.connection().unwrap();
    for table in [
        "mail",
        "mail_recipients",
        "mail_headers",
        "mail_attachments",
    ] {
        let row_count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(row_count, 0, "failed insert should leave {table} empty");
    }
}

/// Delete a mail row and verify every normalized child row cascades with it.
#[test]
fn deleting_mail_cascades_to_normalized_children() {
    let storage = Storage::memory().unwrap();
    let original = normalized_message();
    let saved_scope = scope(1);
    storage.save_scope(&saved_scope).unwrap();
    let sequence = storage
        .insert_message(&original, &[(saved_scope.identifier, 1)])
        .unwrap();
    let connection = storage.connection().unwrap();
    connection
        .execute("DELETE FROM mail WHERE sequence=?1", [sequence.0 as i64])
        .unwrap();
    for table in [
        "mail_recipients",
        "mail_headers",
        "mail_attachments",
        "mail_scope",
    ] {
        let row_count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(row_count, 0, "deleting mail should cascade to {table}");
    }
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
