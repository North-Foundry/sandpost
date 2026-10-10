//! Search index projection, revision, and hydration checks.
use sandpost_core::{Message, MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_storage::Storage;

use crate::ConformanceFailure;
use crate::support::{attachment, failure, headers, unwrap_storage, verify, verify_equal};

/// Verify index reads, indexed lookups, and hydration.
pub async fn index_messages_revisions(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "index_messages_revisions";
    let mut inserted = Vec::new();
    for index in 0..3u64 {
        let attachments = match index {
            1 => vec![attachment("index.bin", 7, "hash-index")],
            2 => vec![
                attachment("index-first.bin", 7, "hash-index-first"),
                attachment("index-second.bin", 8, "hash-index-second"),
            ],
            _ => Vec::new(),
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
            storage.insert_message(&message).await,
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
        verify_equal(CHECK, "indexed facts", &record.facts, &message.facts)?;
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
            .indexed_messages(&[
                first_message.identifier,
                second_message.identifier,
                inserted[2].0.identifier,
            ])
            .await,
    )?;
    let mut actual_identifiers: Vec<MessageIdentifier> =
        indexed.iter().map(|record| record.identifier).collect();
    actual_identifiers.sort();
    let mut expected_identifiers = vec![
        first_message.identifier,
        second_message.identifier,
        inserted[2].0.identifier,
    ];
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
    for message in [&inserted[0].0, &inserted[1].0, &inserted[2].0] {
        let record = indexed
            .iter()
            .find(|record| record.identifier == message.identifier)
            .ok_or_else(|| failure(CHECK, "indexed_messages omitted a message"))?;
        verify_equal(
            CHECK,
            "indexed lookup attachment count",
            record.facts.attachment_count,
            message.attachments.len() as u64,
        )?;
    }
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
