//! Message ingestion, retrieval, ordering, and deletion checks.
use sandpost_core::{Message, MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_storage::{MessageListQuery, MessageSummary, Storage};

use crate::ConformanceFailure;
use crate::support::{
    attachment, failure, headers, mailbox, simple_message, unwrap_storage, verify, verify_equal,
};

/// Verify atomic ingestion and full, metadata, and raw retrieval of one message.
pub async fn message_ingestion_and_retrieval(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "message_ingestion_and_retrieval";
    let attachments = vec![
        attachment("first.bin", 11, "hash-first"),
        attachment("second.bin", 22, "hash-second"),
    ];
    let facts = MessageFacts {
        envelope_from: Some(mailbox("sender@ingest.test")),
        envelope_to: vec![mailbox("envelope@ingest.test")],
        from: vec![mailbox("from@ingest.test")],
        to: vec![mailbox("to@ingest.test"), mailbox("second-to@ingest.test")],
        carbon_copy: vec![mailbox("cc@ingest.test")],
        subject: "Ingested message".to_owned(),
        text: "Ingested body".to_owned(),
        markup_body: "<p>Ingested body</p>".to_owned(),
        message_identifier: Some("ingest-message@ingest.test".to_owned()),
        received_at: 1_700_000_001,
        size: 4_096,
        attachment_count: attachments.len() as u64,
        headers: headers(&[("x-ingest", &["first", "second"]), ("x-trace", &["one"])]),
    };
    let message = Message {
        identifier: MessageIdentifier::new(),
        facts: facts.clone(),
        raw_message: b"raw ingested bytes".to_vec(),
        attachments: attachments.clone(),
    };
    let sequence = unwrap_storage(
        CHECK,
        "insert_message",
        storage.insert_message(&message).await,
    )?;
    verify(
        CHECK,
        sequence.0 >= 1,
        "insert_message must return a positive sequence",
    )?;

    let loaded = unwrap_storage(
        CHECK,
        "get_message",
        storage.get_message(message.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "get_message returned none for an inserted message"))?;
    verify_equal(
        CHECK,
        "message identifier",
        loaded.identifier,
        message.identifier,
    )?;
    verify_equal(CHECK, "message facts", &loaded.facts, &facts)?;
    verify_equal(
        CHECK,
        "message raw bytes",
        &loaded.raw_message,
        &message.raw_message,
    )?;
    verify_equal(
        CHECK,
        "message attachments",
        &loaded.attachments,
        &attachments,
    )?;

    let (metadata_facts, metadata_attachments) = unwrap_storage(
        CHECK,
        "get_message_metadata",
        storage.get_message_metadata(message.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "get_message_metadata returned none"))?;
    verify_equal(CHECK, "metadata facts", &metadata_facts, &facts)?;
    verify_equal(
        CHECK,
        "metadata attachments",
        &metadata_attachments,
        &attachments,
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "max_message_sequence",
            storage.max_message_sequence().await,
        )?
        .0 >= sequence.0,
        "max_message_sequence must include the inserted message",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_message for an unknown identifier",
            storage.get_message(MessageIdentifier::new()).await,
        )?
        .is_none(),
        "an unknown message must read as none",
    )?;
    Ok(())
}

/// Verify newest-first ordering and keyset pagination.
pub async fn message_ordering_and_pagination(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "message_ordering_and_pagination";
    // All mail shares one pool, so this check isolates its rows by a unique subject marker.
    let marker = format!("ordering-{}", MessageIdentifier::new());
    let mut sequences = Vec::new();
    for index in 0..5u64 {
        let message = Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: format!("{marker} {index}"),
                received_at: 1_700_100_000 + index as i64,
                ..MessageFacts::default()
            },
            raw_message: vec![index as u8],
            attachments: Vec::new(),
        };
        sequences.push(unwrap_storage(
            CHECK,
            "insert ordering message",
            storage.insert_message(&message).await,
        )?);
    }

    let listed =
        list_messages_with_subject_marker(storage, CHECK, &marker, None, usize::MAX).await?;
    verify_equal(
        CHECK,
        "ordering page",
        listed
            .iter()
            .map(|summary| summary.sequence)
            .collect::<Vec<_>>(),
        sequences.iter().rev().copied().collect::<Vec<_>>(),
    )?;
    verify(
        CHECK,
        listed
            .windows(2)
            .all(|window| window[0].sequence > window[1].sequence),
        "list_messages must return rows newest first",
    )?;

    let earlier =
        list_messages_with_subject_marker(storage, CHECK, &marker, Some(sequences[2]), usize::MAX)
            .await?;
    verify_equal(
        CHECK,
        "before cursor page",
        earlier
            .iter()
            .map(|summary| summary.sequence)
            .collect::<Vec<_>>(),
        sequences[..2].iter().rev().copied().collect::<Vec<_>>(),
    )?;

    let limited = list_messages_with_subject_marker(storage, CHECK, &marker, None, 2).await?;
    verify_equal(
        CHECK,
        "limited page",
        limited
            .iter()
            .map(|summary| summary.sequence)
            .collect::<Vec<_>>(),
        vec![sequences[4], sequences[3]],
    )?;
    verify(
        CHECK,
        list_messages_with_subject_marker(storage, CHECK, &marker, None, 0)
            .await?
            .is_empty(),
        "a zero limit must return no rows",
    )?;
    verify(
        CHECK,
        list_messages_with_subject_marker(
            storage,
            CHECK,
            &marker,
            Some(MessageSequence(0)),
            usize::MAX,
        )
        .await?
        .is_empty(),
        "a before cursor of zero must return no rows",
    )?;
    Ok(())
}

/// Verify message deletion and revision-guarded deletion.
pub async fn message_deletion(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "message_deletion";
    let message = simple_message("deletion target");
    unwrap_storage(
        CHECK,
        "insert deletion target",
        storage.insert_message(&message).await,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_message",
            storage.delete_message(message.identifier).await,
        )?,
        "delete_message must report true for an existing message",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_message after delete",
            storage.get_message(message.identifier).await,
        )?
        .is_none(),
        "a deleted message must be gone",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "delete_message twice",
            storage.delete_message(message.identifier).await,
        )?,
        "delete_message must report false when absent",
    )?;

    let guarded = simple_message("revision guarded target");
    unwrap_storage(
        CHECK,
        "insert revision guarded target",
        storage.insert_message(&guarded).await,
    )?;
    let record = unwrap_storage(
        CHECK,
        "indexed_messages",
        storage.indexed_messages(&[guarded.identifier]).await,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| failure(CHECK, "indexed_messages omitted an inserted message"))?;
    verify(
        CHECK,
        record.revision >= 1,
        "an inserted message must have a positive revision",
    )?;

    let stale = storage
        .delete_message_at_revision(guarded.identifier, record.revision + 1)
        .await;
    verify(
        CHECK,
        matches!(&stale, Ok(false)),
        format!("a stale revision must not delete, got {stale:?}"),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_message after stale delete",
            storage.get_message(guarded.identifier).await,
        )?
        .is_some(),
        "a stale revision must leave the message in place",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_message_at_revision",
            storage
                .delete_message_at_revision(guarded.identifier, record.revision)
                .await,
        )?,
        "the exact revision must delete the message",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_message after revision delete",
            storage.get_message(guarded.identifier).await,
        )?
        .is_none(),
        "a revision deletion must remove the message",
    )?;
    verify(
        CHECK,
        matches!(
            storage
                .delete_message_at_revision(MessageIdentifier::new(), 1)
                .await,
            Ok(false)
        ),
        "revision deletion of an unknown message must report false",
    )?;
    Ok(())
}

/// List messages whose subject contains a check-specific marker.
async fn list_messages_with_subject_marker(
    storage: &dyn Storage,
    check: &'static str,
    marker: &str,
    before: Option<MessageSequence>,
    limit: usize,
) -> Result<Vec<MessageSummary>, ConformanceFailure> {
    unwrap_storage(
        check,
        "list_messages",
        storage
            .list_messages(MessageListQuery {
                filter: Some(Expression::Predicate(Predicate {
                    field: Field::Subject,
                    operator: Operator::Contains,
                    value: Value::String(marker.to_owned()),
                })),
                before,
                limit,
            })
            .await,
    )
}
