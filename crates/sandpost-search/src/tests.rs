//! Differential and persistence tests for the Tantivy candidate compiler.

use crate::{IndexedDocument, MessageQuery, SearchError, SearchIndex};
use sandpost_core::{Mailbox, MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    sync::{Arc, Barrier, mpsc},
    thread,
};

/// Build message facts with representative scalar, collection, optional, and Unicode values.
fn fixture_facts(index: usize) -> MessageFacts {
    let mailbox = |address: &str, domain: &str| Mailbox {
        address: address.to_owned(),
        domain: domain.to_owned(),
    };
    MessageFacts {
        envelope_from: (index != 0).then(|| mailbox("Env@Example.test", "example.test")),
        envelope_to: if index == 2 {
            Vec::new()
        } else {
            vec![
                mailbox("one@example.test", "example.test"),
                mailbox("other@sample.test", "sample.test"),
            ]
        },
        from: vec![mailbox("from@example.test", "example.test")],
        to: vec![
            mailbox("abcX@one.test", "one.test"),
            mailbox("Xbca@two.test", "two.test"),
        ],
        carbon_copy: if index == 1 {
            Vec::new()
        } else {
            vec![mailbox("copy@example.test", "example.test")]
        },
        subject: if index == 0 {
            String::new()
        } else {
            "Test Café\nSubject".to_owned()
        },
        text: if index == 0 {
            "abcX\nXbca".to_owned()
        } else {
            "alpha\nabcab café🙂".to_owned()
        },
        markup_body: "<b>HTML</b>\nline".to_owned(),
        message_identifier: (index != 0).then(|| "<Mail-ID@example.test>".to_owned()),
        received_at: [i64::MIN, 1_791_072_000, i64::MAX][index],
        size: [u64::MAX, 1024, 0][index],
        attachment_count: [u64::MAX, 2, 0][index],
        headers: if index == 2 {
            BTreeMap::new()
        } else {
            BTreeMap::from([(
                "x-app".to_owned(),
                vec![
                    "bflow".to_owned(),
                    "second\nValue".to_owned(),
                    "literal 50%_ marker".to_owned(),
                ],
            )])
        },
    }
}

/// Create an in-memory index populated and committed with the shared differential fixtures.
fn fixture_index() -> (SearchIndex, Vec<IndexedDocument>) {
    let index = SearchIndex::memory().expect("memory index");
    let documents: Vec<_> = (0..3)
        .map(|position| IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(position as u64 + 1),
            revision: 0,
            facts: fixture_facts(position),
        })
        .collect();
    index.upsert_batch(&documents).expect("index fixtures");
    index.commit(3).expect("commit fixtures");
    (index, documents)
}

/// Build an unrestricted query for one expression.
fn request(expression: Expression, limit: usize) -> MessageQuery {
    MessageQuery {
        authorization: None,
        filter: expression,
        message_identifier: None,
        before: None,
        limit,
    }
}

/// Compare index results to direct evaluation for one expression.
fn assert_matches_evaluator(
    index: &SearchIndex,
    documents: &[IndexedDocument],
    expression: Expression,
) {
    let mut expected: Vec<_> = documents
        .iter()
        .filter(|document| expression.evaluate(&document.facts))
        .map(|document| (document.sequence.0, document.identifier))
        .collect();
    expected.sort_by_key(|(sequence, _)| std::cmp::Reverse(*sequence));
    let expected: Vec<_> = expected
        .into_iter()
        .map(|(_, identifier)| identifier)
        .collect();
    assert_eq!(
        index.search(&request(expression, 256)).expect("search"),
        expected
    );
}

/// Exercise every DSL field and operator against the canonical evaluator.
#[test]
fn every_field_and_operator_matches_canonical_evaluator() {
    let (index, documents) = fixture_index();
    let string_fields = [
        Field::EnvelopeFromAddress,
        Field::EnvelopeFromDomain,
        Field::EnvelopeToAddress,
        Field::EnvelopeToDomain,
        Field::FromAddress,
        Field::FromDomain,
        Field::ToAddress,
        Field::ToDomain,
        Field::CarbonCopyAddress,
        Field::CarbonCopyDomain,
        Field::Subject,
        Field::Text,
        Field::MarkupBody,
        Field::Content,
        Field::MessageIdentifier,
        Field::Header("x-app".to_owned()),
        Field::Header("missing".to_owned()),
    ];
    let string_values = [
        "", "test", "abc", "abcab", "Café", "?", "*", "\n", "🙂", "50%_",
    ];
    let string_operators = [
        Operator::Equal,
        Operator::NotEqual,
        Operator::GreaterThan,
        Operator::GreaterThanOrEqual,
        Operator::LessThan,
        Operator::LessThanOrEqual,
        Operator::Contains,
        Operator::StartsWith,
        Operator::EndsWith,
        Operator::Matches,
    ];
    for field in string_fields {
        for operator in string_operators {
            for value in string_values {
                assert_matches_evaluator(
                    &index,
                    &documents,
                    Expression::Predicate(Predicate {
                        field: field.clone(),
                        operator,
                        value: Value::String(value.to_owned()),
                    }),
                );
            }
        }
    }
    for field in [Field::ReceivedAt, Field::Size, Field::AttachmentCount] {
        for operator in [
            Operator::Equal,
            Operator::NotEqual,
            Operator::GreaterThan,
            Operator::GreaterThanOrEqual,
            Operator::LessThan,
            Operator::LessThanOrEqual,
        ] {
            for value in [i64::MIN, -1, 0, 1, 2, 1024, 1_791_072_000, i64::MAX] {
                assert_matches_evaluator(
                    &index,
                    &documents,
                    Expression::Predicate(Predicate {
                        field: field.clone(),
                        operator,
                        value: Value::Number(value),
                    }),
                );
            }
        }
    }
    for operator in [Operator::Equal, Operator::NotEqual] {
        for value in [Value::Boolean(false), Value::Boolean(true)] {
            assert_matches_evaluator(
                &index,
                &documents,
                Expression::Predicate(Predicate {
                    field: Field::HasAttachments,
                    operator,
                    value: value.clone(),
                }),
            );
        }
    }
}

/// Verify numeric equality candidates read the indexed numeric fields, including unsigned edges.
#[test]
fn numeric_equality_and_inequality_match_canonical_values() {
    let (index, documents) = fixture_index();
    for (field, value) in [
        (Field::ReceivedAt, i64::MIN),
        (Field::ReceivedAt, 1_791_072_000),
        (Field::ReceivedAt, i64::MAX),
        (Field::Size, 1024),
        (Field::Size, 0),
        (Field::Size, i64::MAX),
        (Field::AttachmentCount, 2),
        (Field::AttachmentCount, 0),
        (Field::AttachmentCount, i64::MAX),
    ] {
        for operator in [Operator::Equal, Operator::NotEqual] {
            assert_matches_evaluator(
                &index,
                &documents,
                Expression::Predicate(Predicate {
                    field: field.clone(),
                    operator,
                    value: Value::Number(value),
                }),
            );
        }
    }
    for field in [Field::Size, Field::AttachmentCount] {
        for operator in [Operator::Equal, Operator::NotEqual] {
            assert_matches_evaluator(
                &index,
                &documents,
                Expression::Predicate(Predicate {
                    field: field.clone(),
                    operator,
                    value: Value::Number(-1),
                }),
            );
        }
    }
}

/// Verify directly constructed empty boolean nodes match their canonical truth values.
#[test]
fn empty_and_or_ast_nodes_match_canonical_evaluator() {
    let (index, documents) = fixture_index();
    assert_matches_evaluator(&index, &documents, Expression::And(Vec::new()));
    assert_matches_evaluator(&index, &documents, Expression::Or(Vec::new()));
    assert_matches_evaluator(
        &index,
        &documents,
        Expression::And(vec![Expression::Or(Vec::new()), Expression::True]),
    );
    assert_matches_evaluator(
        &index,
        &documents,
        Expression::Or(vec![Expression::And(Vec::new()), Expression::False]),
    );
}

/// Verify long body equality uses complete candidate grams without whole-body terms.
#[test]
fn long_body_equality_survives_tokenizer_default_length_boundaries() {
    let (index, mut documents) = fixture_index();
    documents[0].facts.text = "Long NeedLe body %_ ".repeat(9);
    index.upsert(&documents[0]).expect("replace indexed body");
    index.commit(3).expect("commit body replacement");
    let expression = Expression::Predicate(Predicate {
        field: Field::Text,
        operator: Operator::Equal,
        value: Value::String(documents[0].facts.text.clone()),
    });
    assert_matches_evaluator(&index, &documents, expression);
}

/// Preserve exact negation and collection `!=` semantics through candidate narrowing.
#[test]
fn boolean_and_missing_value_semantics_are_exact() {
    let (index, documents) = fixture_index();
    let mailbox_inequality = Expression::Predicate(Predicate {
        field: Field::EnvelopeToDomain,
        operator: Operator::NotEqual,
        value: Value::String("example.test".to_owned()),
    });
    assert_matches_evaluator(&index, &documents, mailbox_inequality.clone());
    assert_matches_evaluator(
        &index,
        &documents,
        Expression::Not(Box::new(mailbox_inequality)),
    );
    assert_matches_evaluator(
        &index,
        &documents,
        Expression::And(vec![
            Expression::True,
            Expression::Not(Box::new(Expression::False)),
        ]),
    );
    assert_matches_evaluator(
        &index,
        &documents,
        Expression::Or(vec![
            Expression::False,
            Expression::Predicate(Predicate {
                field: Field::Text,
                operator: Operator::Contains,
                value: Value::String("café🙂".to_owned()),
            }),
        ]),
    );
}

/// Enforce authorization expressions and all direct request restrictions together.
#[test]
fn authorization_message_identifier_and_cursor_compose() {
    let index = SearchIndex::memory().expect("memory index");
    let mut documents = Vec::new();
    for position in 1..=3 {
        let mut facts = fixture_facts(1);
        if position == 2 {
            facts.subject = "Hidden".to_owned();
        }
        documents.push(IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(position),
            revision: 0,
            facts,
        });
    }
    index.upsert_batch(&documents).expect("batch index");
    index.commit(3).expect("commit");
    let request = MessageQuery {
        authorization: Some(Expression::Not(Box::new(Expression::Predicate(
            Predicate {
                field: Field::Subject,
                operator: Operator::Contains,
                value: Value::String("Hidden".to_owned()),
            },
        )))),
        filter: Expression::True,
        message_identifier: None,
        before: None,
        limit: 256,
    };
    assert_eq!(
        index.search(&request).expect("authorized search"),
        vec![documents[2].identifier, documents[0].identifier]
    );
    let mut unrestricted = request.clone();
    unrestricted.authorization = None;
    assert_eq!(index.search(&unrestricted).expect("all mail").len(), 3);
    let mut denied = request.clone();
    denied.authorization = Some(Expression::False);
    assert!(index.search(&denied).expect("deny all").is_empty());
    let mut direct = request.clone();
    direct.message_identifier = Some(documents[1].identifier);
    assert!(
        index.search(&direct).expect("direct id").is_empty(),
        "authorization applies to direct identifiers"
    );
    direct.message_identifier = Some(documents[0].identifier);
    assert_eq!(
        index.search(&direct).expect("direct id"),
        vec![documents[0].identifier]
    );
    let mut cursor = request;
    cursor.before = Some(MessageSequence(3));
    assert_eq!(
        index.search(&cursor).expect("keyset page"),
        vec![documents[0].identifier]
    );
}

/// Continue keyset candidate retrieval past a full page of false-positive trigrams.
#[test]
fn substring_false_positives_do_not_hide_later_exact_matches() {
    let index = SearchIndex::memory().expect("memory index");
    let mut documents = Vec::new();
    for sequence in 1..=257 {
        let mut facts = fixture_facts(0);
        facts.to = vec![
            Mailbox {
                address: "abcX".into(),
                domain: "one.test".into(),
            },
            Mailbox {
                address: "Xbca".into(),
                domain: "two.test".into(),
            },
        ];
        documents.push(IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(sequence),
            revision: 0,
            facts,
        });
    }
    let mut exact_facts = fixture_facts(0);
    exact_facts.to = vec![Mailbox {
        address: "abcab".into(),
        domain: "one.test".into(),
    }];
    let exact = IndexedDocument {
        identifier: MessageIdentifier::new(),
        sequence: MessageSequence(0),
        revision: 0,
        facts: exact_facts,
    };
    documents.push(exact.clone());
    index
        .upsert_batch(&documents[..256])
        .expect("first bounded batch");
    index
        .upsert_batch(&documents[256..])
        .expect("second bounded batch");
    index.commit(257).expect("commit");
    let query = request(
        Expression::Predicate(Predicate {
            field: Field::ToAddress,
            operator: Operator::Contains,
            value: Value::String("abcab".to_owned()),
        }),
        1,
    );
    assert_eq!(
        index.search(&query).expect("substring query"),
        vec![exact.identifier]
    );
}

/// Keep upserts and deletions idempotent and persist the commit sequence on reopen.
#[test]
fn upsert_delete_and_watermark_survive_reopen() {
    let path = std::env::temp_dir().join(format!("sandpost-search-{}", MessageIdentifier::new()));
    let index = SearchIndex::create(&path).expect("create index");
    let mut document = IndexedDocument {
        identifier: MessageIdentifier::new(),
        sequence: MessageSequence(41),
        revision: 17,
        facts: fixture_facts(1),
    };
    document.facts.subject = "original subject".to_owned();
    index.upsert(&document).expect("first upsert");
    index.commit(40).expect("initial commit");
    document.facts.subject = "replaced subject".to_owned();
    index.upsert(&document).expect("repeated upsert");
    assert_eq!(
        index
            .search(&request(
                Expression::Predicate(Predicate {
                    field: Field::Subject,
                    operator: Operator::Equal,
                    value: Value::String("original subject".to_owned()),
                }),
                10,
            ))
            .expect("reader sees committed generation"),
        vec![document.identifier]
    );
    index.commit(41).expect("commit");
    assert_eq!(
        index.search(&request(Expression::True, 10)).expect("read"),
        vec![document.identifier]
    );
    assert_eq!(
        index
            .search_hits(&request(Expression::True, 10))
            .expect("read indexed revision"),
        vec![(document.identifier, 17)]
    );
    drop(index);
    let reopened = SearchIndex::open(&path).expect("reopen index");
    assert_eq!(reopened.committed_sequence().expect("watermark"), 41);
    assert_eq!(
        reopened
            .search(&request(Expression::True, 10))
            .expect("reopened read"),
        vec![document.identifier]
    );
    assert_eq!(
        reopened
            .search_hits(&request(Expression::True, 10))
            .expect("reopened revision"),
        vec![(document.identifier, 17)]
    );
    reopened.delete(document.identifier).expect("delete");
    reopened.delete(document.identifier).expect("repeat delete");
    reopened.commit(42).expect("delete commit");
    assert!(
        reopened
            .search(&request(Expression::True, 10))
            .expect("after delete")
            .is_empty()
    );
    assert_eq!(
        reopened.committed_sequence().expect("updated watermark"),
        42
    );
    drop(reopened);
    fs::remove_dir_all(path).expect("remove index directory");
}

/// Reject corrupted metadata and cap every returned search batch at 256 identifiers.
#[test]
fn corrupt_index_errors_and_search_batch_is_bounded() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-search-corrupt-{}",
        MessageIdentifier::new()
    ));
    let index = SearchIndex::create(&path).expect("create index");
    let documents: Vec<_> = (1..=257)
        .map(|sequence| IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(sequence),
            revision: 0,
            facts: fixture_facts(1),
        })
        .collect();
    index.upsert_batch(&documents[..256]).expect("first batch");
    index.upsert(&documents[256]).expect("last document");
    assert!(matches!(
        index.upsert_batch(&documents),
        Err(SearchError::BatchTooLarge(257))
    ));
    index.commit(257).expect("commit");
    assert_eq!(
        index
            .search(&request(Expression::True, usize::MAX))
            .expect("bounded search")
            .len(),
        256
    );
    drop(index);
    let segment = fs::read_dir(&path)
        .expect("index directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "store")
        })
        .expect("stored segment file");
    let mut segment_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(segment)
        .expect("open segment for corruption");
    let mut byte = [0_u8; 1];
    segment_file
        .read_exact(&mut byte)
        .expect("read segment byte");
    byte[0] ^= 1;
    segment_file
        .seek(SeekFrom::Start(0))
        .expect("rewind segment");
    segment_file.write_all(&byte).expect("corrupt segment");
    drop(segment_file);
    assert!(matches!(
        SearchIndex::open(&path),
        Err(SearchError::CorruptIndex(_))
    ));
    fs::remove_dir_all(path).expect("remove index directory");
}

/// Generate bounded mixed boolean expressions from a reproducible seed.
fn deterministic_mixed_expression(seed: &mut u64, depth: usize) -> Expression {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    let choice = (*seed % 8) as usize;
    let predicates = [
        Predicate {
            field: Field::Subject,
            operator: Operator::Contains,
            value: Value::String("café".to_owned()),
        },
        Predicate {
            field: Field::EnvelopeToDomain,
            operator: Operator::NotEqual,
            value: Value::String("example.test".to_owned()),
        },
        Predicate {
            field: Field::Size,
            operator: Operator::GreaterThanOrEqual,
            value: Value::Number(1024),
        },
        Predicate {
            field: Field::HasAttachments,
            operator: Operator::Equal,
            value: Value::Boolean(true),
        },
        Predicate {
            field: Field::Header("x-app".to_owned()),
            operator: Operator::Matches,
            value: Value::String("b*".to_owned()),
        },
        Predicate {
            field: Field::Text,
            operator: Operator::EndsWith,
            value: Value::String("🙂".to_owned()),
        },
        Predicate {
            field: Field::MessageIdentifier,
            operator: Operator::NotEqual,
            value: Value::String("<Mail-ID@example.test>".to_owned()),
        },
        Predicate {
            field: Field::ToAddress,
            operator: Operator::Contains,
            value: Value::String("abc".to_owned()),
        },
    ];
    if depth == 0 || choice < predicates.len() / 2 {
        return Expression::Predicate(predicates[choice].clone());
    }
    match choice % 3 {
        0 => Expression::Not(Box::new(deterministic_mixed_expression(seed, depth - 1))),
        1 => Expression::And(vec![
            deterministic_mixed_expression(seed, depth - 1),
            deterministic_mixed_expression(seed, depth - 1),
        ]),
        _ => Expression::Or(vec![
            deterministic_mixed_expression(seed, depth - 1),
            deterministic_mixed_expression(seed, depth - 1),
            deterministic_mixed_expression(seed, depth - 1),
        ]),
    }
}

/// Compare reproducible nested mixed-field expressions against canonical evaluation.
#[test]
fn deterministic_mixed_expressions_match_canonical_evaluator() {
    let (index, documents) = fixture_index();
    let mut seed = 0x5eed_cafe_f00d_u64;
    for _ in 0..96 {
        assert_matches_evaluator(
            &index,
            &documents,
            deterministic_mixed_expression(&mut seed, 4),
        );
    }
}

/// Check long exact and literal-glob terms around Tantivy's term byte boundary.
#[test]
fn oversized_subject_equality_and_literal_matches_are_exact() {
    let (index, mut documents) = fixture_index();
    let lengths = [65_522, 65_523, 65_530];
    for (document, length) in documents.iter_mut().zip(lengths) {
        document.facts.subject = "a".repeat(length);
        index.upsert(document).expect("index oversized subject");
    }
    index.commit(3).expect("commit oversized subjects");

    for (document, length) in documents.iter().zip(lengths) {
        let value = "a".repeat(length);
        for operator in [Operator::Equal, Operator::Matches] {
            assert_matches_evaluator(
                &index,
                &documents,
                Expression::Predicate(Predicate {
                    field: Field::Subject,
                    operator,
                    value: Value::String(value.clone()),
                }),
            );
            let results = index
                .search(&request(
                    Expression::Predicate(Predicate {
                        field: Field::Subject,
                        operator,
                        value: Value::String(value.clone()),
                    }),
                    256,
                ))
                .expect("search oversized subject");
            assert_eq!(results, vec![document.identifier]);
        }
    }
}

/// Ensure exact subject terms use UTF-8 byte lengths for long Unicode values.
#[test]
fn oversized_unicode_subject_terms_match_exactly() {
    let (index, mut documents) = fixture_index();
    let low = "é".repeat(32_763);
    let high = "é".repeat(32_764);
    documents[0].facts.subject = low.clone();
    documents[1].facts.subject = high.clone();
    index
        .upsert_batch(&documents[..2])
        .expect("index Unicode subjects");
    index.commit(3).expect("commit Unicode subjects");

    for (value, expected) in [
        (low, documents[0].identifier),
        (high, documents[1].identifier),
    ] {
        for operator in [Operator::Equal, Operator::Matches] {
            assert_eq!(
                index
                    .search(&request(
                        Expression::Predicate(Predicate {
                            field: Field::Subject,
                            operator,
                            value: Value::String(value.clone()),
                        }),
                        256,
                    ))
                    .expect("search Unicode subject"),
                vec![expected]
            );
        }
    }
}

/// Keep short substring searches exact when the header name itself is oversized.
#[test]
fn short_header_contains_survives_oversized_header_name() {
    let (index, mut documents) = fixture_index();
    let header_name = "x".repeat(65_530);
    documents[0].facts.headers.insert(
        header_name.clone().to_ascii_lowercase(),
        vec!["needle value".to_owned()],
    );
    index.upsert(&documents[0]).expect("index long header name");
    index.commit(3).expect("commit long header name");
    assert_eq!(
        index
            .search(&request(
                Expression::Predicate(Predicate {
                    field: Field::Header(header_name),
                    operator: Operator::Contains,
                    value: Value::String("eed".to_owned()),
                }),
                256,
            ))
            .expect("search short header substring"),
        vec![documents[0].identifier]
    );
}

/// Enforce deletion batch bounds and empty/cursor-zero search boundaries.
#[test]
fn delete_batch_limit_and_zero_cursor_boundaries() {
    let index = SearchIndex::memory().expect("memory index");
    let documents: Vec<_> = (1..=257)
        .map(|sequence| IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(sequence),
            revision: 0,
            facts: fixture_facts(1),
        })
        .collect();
    index.upsert_batch(&documents[..256]).expect("first batch");
    index.upsert(&documents[256]).expect("last document");
    index
        .delete_batch(
            &documents[..256]
                .iter()
                .map(|document| document.identifier)
                .collect::<Vec<_>>(),
        )
        .expect("delete maximum batch");
    assert!(matches!(
        index.delete_batch(
            &documents
                .iter()
                .map(|document| document.identifier)
                .collect::<Vec<_>>()
        ),
        Err(SearchError::BatchTooLarge(257))
    ));
    index.commit(257).expect("commit deletions");

    assert!(
        index
            .search(&request(Expression::True, 0))
            .expect("zero result limit")
            .is_empty()
    );
    let mut cursor_at_zero = request(Expression::True, 256);
    cursor_at_zero.before = Some(MessageSequence(0));
    assert!(
        index
            .search(&cursor_at_zero)
            .expect("zero cursor")
            .is_empty()
    );
    assert_eq!(
        index
            .search(&request(Expression::True, 256))
            .expect("remaining document"),
        vec![documents[256].identifier]
    );
}

/// Exercise cloned writer and reader handles across synchronized write and commit phases.
#[test]
fn cloned_handles_concurrently_write_commit_and_read() {
    const WRITER_COUNT: usize = 4;
    const DOCUMENTS_PER_WRITER: usize = 24;
    let index = SearchIndex::memory().expect("memory index");
    let phase = Arc::new(Barrier::new(WRITER_COUNT + 2));
    let mut writers = Vec::new();
    let mut expected_identifiers = Vec::new();

    for worker in 0..WRITER_COUNT {
        let handle = index.clone();
        let barrier = Arc::clone(&phase);
        let documents: Vec<_> = (0..DOCUMENTS_PER_WRITER)
            .map(|offset| IndexedDocument {
                identifier: MessageIdentifier::new(),
                sequence: MessageSequence((worker * DOCUMENTS_PER_WRITER + offset + 1) as u64),
                revision: worker as u64,
                facts: fixture_facts(worker % 3),
            })
            .collect();
        expected_identifiers.extend(documents.iter().map(|document| document.identifier));
        writers.push(thread::spawn(move || {
            barrier.wait();
            handle.upsert_batch(&documents).expect("concurrent upsert");
            barrier.wait();
        }));
    }

    let commit_handle = index.clone();
    let commit_barrier = Arc::clone(&phase);
    let committer = thread::spawn(move || {
        commit_barrier.wait();
        commit_barrier.wait();
        commit_handle.commit((WRITER_COUNT * DOCUMENTS_PER_WRITER) as u64)
    });

    let reader_handle = index.clone();
    let reader_barrier = Arc::clone(&phase);
    let (read_sender, read_receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        reader_barrier.wait();
        reader_barrier.wait();
        read_sender
            .send(reader_handle.search(&request(Expression::True, 256)))
            .unwrap();
    });

    for writer in writers {
        writer.join().expect("writer thread");
    }
    committer.join().expect("committer thread").expect("commit");
    reader.join().expect("reader thread");
    let concurrent_read = read_receiver
        .recv()
        .expect("concurrent read result")
        .expect("search");
    assert!(
        concurrent_read
            .iter()
            .all(|identifier| expected_identifiers.contains(identifier))
    );
    assert!(
        concurrent_read.is_empty() || concurrent_read.len() == DOCUMENTS_PER_WRITER * WRITER_COUNT
    );
    let mut expected_sorted = expected_identifiers;
    expected_sorted.sort();
    let mut actual_sorted = index
        .search(&request(Expression::True, 256))
        .expect("post-commit read");
    actual_sorted.sort();
    assert_eq!(actual_sorted, expected_sorted);
}
