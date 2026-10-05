//! Message ingestion, retrieval, ordering, filtering, deletion, and index conformance checks.
use std::collections::BTreeSet;

use sandpost_core::{
    EndpointIdentifier, Message, MessageFacts, MessageIdentifier, MessageSequence,
};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_storage::{MessageListQuery, MessageSummary, Storage};

use crate::ConformanceFailure;
use crate::support::{
    attachment, failure, headers, mailbox, save_endpoint, simple_message, unwrap_storage, verify,
    verify_equal,
};

/// Verify atomic ingestion and full, metadata, and raw retrieval of one message.
pub async fn message_ingestion_and_retrieval(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "message_ingestion_and_retrieval";
    let endpoint = save_endpoint(storage, CHECK, "ingestion endpoint").await?;
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
        storage.insert_message(&message, endpoint).await,
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

    let (read_endpoint, with_endpoint) = unwrap_storage(
        CHECK,
        "get_message_with_endpoint",
        storage.get_message_with_endpoint(message.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "get_message_with_endpoint returned none"))?;
    verify_equal(CHECK, "message endpoint", read_endpoint, endpoint)?;
    verify_equal(
        CHECK,
        "message with endpoint facts",
        &with_endpoint.facts,
        &facts,
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

    let (metadata_endpoint, _, _) = unwrap_storage(
        CHECK,
        "get_message_metadata_with_endpoint",
        storage
            .get_message_metadata_with_endpoint(message.identifier)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "get_message_metadata_with_endpoint returned none"))?;
    verify_equal(CHECK, "metadata endpoint", metadata_endpoint, endpoint)?;
    verify_equal(
        CHECK,
        "message_endpoint",
        unwrap_storage(
            CHECK,
            "message_endpoint",
            storage.message_endpoint(message.identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "message_endpoint returned none"))?,
        endpoint,
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
    let endpoint = save_endpoint(storage, CHECK, "ordering endpoint").await?;
    let mut sequences = Vec::new();
    for index in 0..5u64 {
        let message = Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: format!("ordering {index}"),
                received_at: 1_700_100_000 + index as i64,
                ..MessageFacts::default()
            },
            raw_message: vec![index as u8],
            attachments: Vec::new(),
        };
        sequences.push(unwrap_storage(
            CHECK,
            "insert ordering message",
            storage.insert_message(&message, endpoint).await,
        )?);
    }

    let listed = list_endpoint_messages(storage, CHECK, endpoint, None, usize::MAX).await?;
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
        list_endpoint_messages(storage, CHECK, endpoint, Some(sequences[2]), usize::MAX).await?;
    verify_equal(
        CHECK,
        "before cursor page",
        earlier
            .iter()
            .map(|summary| summary.sequence)
            .collect::<Vec<_>>(),
        sequences[..2].iter().rev().copied().collect::<Vec<_>>(),
    )?;

    let limited = list_endpoint_messages(storage, CHECK, endpoint, None, 2).await?;
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
        list_endpoint_messages(storage, CHECK, endpoint, None, 0)
            .await?
            .is_empty(),
        "a zero limit must return no rows",
    )?;
    verify(
        CHECK,
        list_endpoint_messages(
            storage,
            CHECK,
            endpoint,
            Some(MessageSequence(0)),
            usize::MAX,
        )
        .await?
        .is_empty(),
        "a before cursor of zero must return no rows",
    )?;
    Ok(())
}

/// Verify that a storage filter selects exactly the rows the canonical evaluator selects.
pub async fn message_filter_equivalence(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "message_filter_equivalence";
    let endpoint = save_endpoint(storage, CHECK, "filter endpoint").await?;
    let mut inserted = Vec::new();
    for message in filter_fixtures() {
        unwrap_storage(
            CHECK,
            "insert filter message",
            storage.insert_message(&message, endpoint).await,
        )?;
        let (facts, _) = unwrap_storage(
            CHECK,
            "load filter facts",
            storage.get_message_metadata(message.identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "an inserted filter message is missing"))?;
        inserted.push((message.identifier, facts));
    }

    for expression in representative_filters() {
        let expected: BTreeSet<MessageIdentifier> = inserted
            .iter()
            .filter(|(_, facts)| expression.evaluate(facts))
            .map(|(identifier, _)| *identifier)
            .collect();
        let rows = unwrap_storage(
            CHECK,
            "filtered list_messages",
            storage
                .list_messages(MessageListQuery {
                    endpoint: Some(endpoint),
                    filter: Some(expression.clone()),
                    before: None,
                    limit: usize::MAX,
                })
                .await,
        )?;
        let actual: BTreeSet<MessageIdentifier> =
            rows.iter().map(|summary| summary.identifier).collect();
        verify_equal(
            CHECK,
            &format!("filter {expression:?} selection"),
            actual,
            expected,
        )?;
        for row in &rows {
            let (facts, _) = unwrap_storage(
                CHECK,
                "load filtered facts",
                storage.get_message_metadata(row.identifier).await,
            )?
            .ok_or_else(|| failure(CHECK, "a filtered row is missing its facts"))?;
            verify(
                CHECK,
                expression.evaluate(&facts),
                format!(
                    "filter {expression:?} returned {} whose facts do not satisfy it",
                    row.identifier
                ),
            )?;
        }
    }
    Ok(())
}

/// Verify message deletion and revision-guarded deletion.
pub async fn message_deletion(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "message_deletion";
    let endpoint = save_endpoint(storage, CHECK, "deletion endpoint").await?;
    let message = simple_message("deletion target");
    unwrap_storage(
        CHECK,
        "insert deletion target",
        storage.insert_message(&message, endpoint).await,
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
        storage.insert_message(&guarded, endpoint).await,
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

/// Verify index reads, indexed lookups, and hydration.
pub async fn index_messages_revisions(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "index_messages_revisions";
    let endpoint = save_endpoint(storage, CHECK, "index endpoint").await?;
    let mut inserted = Vec::new();
    for index in 0..3u64 {
        let attachments = if index == 1 {
            vec![attachment("index.bin", 7, "hash-index")]
        } else {
            Vec::new()
        };
        let header_value = index.to_string();
        let message = Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: format!("index {index}"),
                text: format!("index body {index}"),
                received_at: 1_700_600_000 + index as i64,
                size: 10 * index,
                attachment_count: attachments.len() as u64,
                headers: headers(&[("x-index", &[header_value.as_str()])]),
                ..MessageFacts::default()
            },
            raw_message: vec![index as u8],
            attachments,
        };
        let sequence = unwrap_storage(
            CHECK,
            "insert index message",
            storage.insert_message(&message, endpoint).await,
        )?;
        inserted.push((message, sequence));
    }

    let records = unwrap_storage(
        CHECK,
        "index_messages",
        storage.index_messages(MessageSequence(0), None, 64).await,
    )?;
    verify(
        CHECK,
        records
            .windows(2)
            .all(|window| window[0].sequence < window[1].sequence),
        "index_messages must be ordered by ascending sequence",
    )?;
    for (message, sequence) in &inserted {
        let record = records
            .iter()
            .find(|candidate| candidate.identifier == message.identifier)
            .ok_or_else(|| {
                failure(
                    CHECK,
                    format!("index_messages omitted {}", message.identifier),
                )
            })?;
        verify_equal(CHECK, "indexed sequence", record.sequence, *sequence)?;
        verify(
            CHECK,
            record.revision >= 1,
            "an indexed record must have a positive revision",
        )?;
        verify_equal(
            CHECK,
            "indexed endpoint",
            record.endpoint_identifier,
            endpoint,
        )?;
        verify_equal(CHECK, "indexed facts", &record.facts, &message.facts)?;
        verify_equal(
            CHECK,
            "indexed attachments",
            &record.attachments,
            &message.attachments,
        )?;
    }
    let ordered_revisions: Vec<u64> = inserted
        .iter()
        .map(|(message, _)| {
            records
                .iter()
                .find(|candidate| candidate.identifier == message.identifier)
                .map(|record| record.revision)
                .unwrap_or(0)
        })
        .collect();
    verify(
        CHECK,
        ordered_revisions
            .windows(2)
            .all(|window| window[0] < window[1]),
        "search revisions must advance with insertion order",
    )?;
    let first_sequence = inserted[0].1;
    let second_sequence = inserted[1].1;
    let window = unwrap_storage(
        CHECK,
        "bounded index window",
        storage
            .index_messages(first_sequence, Some(second_sequence), 64)
            .await,
    )?;
    verify_equal(
        CHECK,
        "bounded index window identifiers",
        window
            .iter()
            .map(|record| record.identifier)
            .collect::<Vec<_>>(),
        vec![inserted[1].0.identifier],
    )?;

    let first_message = &inserted[0].0;
    let second_message = &inserted[1].0;
    let indexed = unwrap_storage(
        CHECK,
        "indexed_messages lookup",
        storage
            .indexed_messages(&[first_message.identifier, second_message.identifier])
            .await,
    )?;
    let mut actual_identifiers: Vec<MessageIdentifier> =
        indexed.iter().map(|record| record.identifier).collect();
    actual_identifiers.sort();
    let mut expected_identifiers = vec![first_message.identifier, second_message.identifier];
    expected_identifiers.sort();
    verify_equal(
        CHECK,
        "indexed lookup identifiers",
        actual_identifiers,
        expected_identifiers,
    )?;
    let first_record = records
        .iter()
        .find(|candidate| candidate.identifier == first_message.identifier)
        .ok_or_else(|| failure(CHECK, "the first indexed record is missing"))?;
    let looked_up = indexed
        .iter()
        .find(|record| record.identifier == first_message.identifier)
        .ok_or_else(|| failure(CHECK, "indexed_messages omitted the first message"))?;
    verify_equal(
        CHECK,
        "indexed lookup revision",
        looked_up.revision,
        first_record.revision,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "indexed_messages empty",
            storage.indexed_messages(&[]).await,
        )?
        .is_empty(),
        "indexed_messages of no identifiers must be empty",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "indexed_messages unknown",
            storage.indexed_messages(&[MessageIdentifier::new()]).await,
        )?
        .is_empty(),
        "indexed_messages must omit unknown identifiers",
    )?;

    let hydrated = unwrap_storage(
        CHECK,
        "hydrate_messages",
        storage
            .hydrate_messages(&[
                second_message.identifier,
                first_message.identifier,
                MessageIdentifier::new(),
            ])
            .await,
    )?;
    verify_equal(
        CHECK,
        "hydrate_messages order",
        hydrated
            .iter()
            .map(|summary| summary.identifier)
            .collect::<Vec<_>>(),
        vec![second_message.identifier, first_message.identifier],
    )?;
    let current = unwrap_storage(
        CHECK,
        "hydrate_current_messages",
        storage
            .hydrate_current_messages(&[(first_message.identifier, first_record.revision)])
            .await,
    )?;
    verify_equal(
        CHECK,
        "hydrate_current_messages result",
        current
            .iter()
            .map(|summary| summary.identifier)
            .collect::<Vec<_>>(),
        vec![first_message.identifier],
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "hydrate_current_messages stale",
            storage
                .hydrate_current_messages(&[(first_message.identifier, first_record.revision + 1)])
                .await,
        )?
        .is_empty(),
        "hydrate_current_messages must omit a stale revision",
    )?;
    Ok(())
}

/// List messages restricted to one endpoint.
async fn list_endpoint_messages(
    storage: &dyn Storage,
    check: &'static str,
    endpoint: EndpointIdentifier,
    before: Option<MessageSequence>,
    limit: usize,
) -> Result<Vec<MessageSummary>, ConformanceFailure> {
    unwrap_storage(
        check,
        "list_messages",
        storage
            .list_messages(MessageListQuery {
                endpoint: Some(endpoint),
                filter: None,
                before,
                limit,
            })
            .await,
    )
}

/// Build the representative message fixtures used by the filter-equivalence check.
fn filter_fixtures() -> Vec<Message> {
    vec![
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                from: vec![mailbox("alice@example.test")],
                to: vec![mailbox("bob@example.test")],
                subject: "Alpha report".to_owned(),
                text: "quarterly numbers".to_owned(),
                size: 100,
                received_at: 1_700_100_000,
                headers: headers(&[("x-team", &["red"])]),
                ..MessageFacts::default()
            },
            raw_message: b"fixture one".to_vec(),
            attachments: Vec::new(),
        },
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                from: vec![mailbox("carol@other.test")],
                to: vec![mailbox("bob@example.test")],
                carbon_copy: vec![mailbox("dana@other.test")],
                subject: "beta news".to_owned(),
                text: "hello world".to_owned(),
                size: 5_000,
                received_at: 1_700_200_000,
                attachment_count: 1,
                headers: headers(&[("x-team", &["blue"])]),
                ..MessageFacts::default()
            },
            raw_message: b"fixture two".to_vec(),
            attachments: vec![attachment("two.bin", 42, "hash-two")],
        },
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                from: vec![mailbox("ALICE@EXAMPLE.TEST")],
                to: vec![mailbox("dave@example.test")],
                subject: "Gamma summary".to_owned(),
                text: "quarterly summary".to_owned(),
                size: 250,
                received_at: 1_700_300_000,
                attachment_count: 2,
                headers: headers(&[("x-priority", &["high"])]),
                ..MessageFacts::default()
            },
            raw_message: b"fixture three".to_vec(),
            attachments: vec![
                attachment("three-a.bin", 1, "hash-three-a"),
                attachment("three-b.bin", 2, "hash-three-b"),
            ],
        },
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                from: vec![mailbox("eve@example.test")],
                subject: "Delta".to_owned(),
                received_at: 1_700_400_000,
                ..MessageFacts::default()
            },
            raw_message: b"fixture four".to_vec(),
            attachments: Vec::new(),
        },
    ]
}

/// Build the representative filter expressions evaluated against the fixtures.
fn representative_filters() -> Vec<Expression> {
    let predicate = |field: Field, operator: Operator, value: Value| -> Expression {
        Expression::Predicate(Predicate {
            field,
            operator,
            value,
        })
    };
    vec![
        Expression::True,
        Expression::False,
        predicate(
            Field::Subject,
            Operator::Equal,
            Value::String("beta news".to_owned()),
        ),
        predicate(
            Field::Content,
            Operator::Contains,
            Value::String("quarterly".to_owned()),
        ),
        predicate(
            Field::FromAddress,
            Operator::Equal,
            Value::String("Alice@Example.TEST".to_owned()),
        ),
        predicate(
            Field::Header("x-team".to_owned()),
            Operator::Equal,
            Value::String("blue".to_owned()),
        ),
        predicate(
            Field::Size,
            Operator::GreaterThanOrEqual,
            Value::Number(250),
        ),
        predicate(Field::Size, Operator::LessThan, Value::Number(300)),
        predicate(Field::HasAttachments, Operator::Equal, Value::Boolean(true)),
        predicate(
            Field::AttachmentCount,
            Operator::GreaterThanOrEqual,
            Value::Number(2),
        ),
        Expression::Not(Box::new(predicate(
            Field::Subject,
            Operator::Equal,
            Value::String("beta news".to_owned()),
        ))),
        Expression::And(vec![
            predicate(
                Field::Content,
                Operator::Contains,
                Value::String("quarterly".to_owned()),
            ),
            predicate(
                Field::Size,
                Operator::GreaterThanOrEqual,
                Value::Number(200),
            ),
        ]),
        Expression::Or(vec![
            predicate(
                Field::Subject,
                Operator::Equal,
                Value::String("Delta".to_owned()),
            ),
            predicate(Field::HasAttachments, Operator::Equal, Value::Boolean(true)),
        ]),
    ]
}
