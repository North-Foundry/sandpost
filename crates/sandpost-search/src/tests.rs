//! Differential and persistence tests for the Tantivy candidate compiler.

use crate::{EndpointQuery, IndexedDocument, MessageQuery, SearchError, SearchIndex};
use sandpost_core::{
    Attachment, EndpointIdentifier, Mailbox, MessageFacts, MessageIdentifier, MessageSequence,
};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
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
    let endpoint = EndpointIdentifier::new();
    let documents: Vec<_> = (0..3)
        .map(|position| IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(position as u64 + 1),
            revision: 0,
            endpoint_identifier: endpoint,
            facts: fixture_facts(position),
            attachments: Vec::<Attachment>::new(),
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
        endpoint_identifier: None,
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

/// Enforce endpoint authorization union semantics and all direct request restrictions.
#[test]
fn authorization_endpoint_identifier_and_cursor_compose() {
    let index = SearchIndex::memory().expect("memory index");
    let first_endpoint = EndpointIdentifier::new();
    let second_endpoint = EndpointIdentifier::new();
    let mut documents = Vec::new();
    for position in 1..=3 {
        documents.push(IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(position),
            revision: 0,
            endpoint_identifier: if position == 2 {
                second_endpoint
            } else {
                first_endpoint
            },
            facts: fixture_facts(1),
            attachments: Vec::new(),
        });
    }
    index.upsert_batch(&documents).expect("batch index");
    index.commit(3).expect("commit");
    let request = MessageQuery {
        authorization: Some(vec![
            EndpointQuery {
                endpoint_identifier: first_endpoint,
                expression: Expression::True,
            },
            EndpointQuery {
                endpoint_identifier: second_endpoint,
                expression: Expression::Predicate(Predicate {
                    field: Field::Subject,
                    operator: Operator::Contains,
                    value: Value::String("Test".to_owned()),
                }),
            },
        ]),
        filter: Expression::True,
        endpoint_identifier: None,
        message_identifier: None,
        before: None,
        limit: 256,
    };
    assert_eq!(index.search(&request).expect("authorized search").len(), 3);
    let mut denied = request.clone();
    denied.authorization = Some(Vec::new());
    assert!(index.search(&denied).expect("deny all").is_empty());
    let mut endpoint_filtered = request.clone();
    endpoint_filtered.endpoint_identifier = Some(second_endpoint);
    assert_eq!(
        index.search(&endpoint_filtered).expect("endpoint filter"),
        vec![documents[1].identifier]
    );
    let mut direct = request.clone();
    direct.message_identifier = Some(documents[0].identifier);
    assert_eq!(
        index.search(&direct).expect("direct id"),
        vec![documents[0].identifier]
    );
    let mut cursor = request;
    cursor.before = Some(MessageSequence(3));
    assert_eq!(
        index.search(&cursor).expect("keyset page"),
        vec![documents[1].identifier, documents[0].identifier]
    );
}

/// Continue keyset candidate retrieval past a full page of false-positive trigrams.
#[test]
fn substring_false_positives_do_not_hide_later_exact_matches() {
    let index = SearchIndex::memory().expect("memory index");
    let endpoint = EndpointIdentifier::new();
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
            endpoint_identifier: endpoint,
            facts,
            attachments: Vec::new(),
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
        endpoint_identifier: endpoint,
        facts: exact_facts,
        attachments: Vec::new(),
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
        endpoint_identifier: EndpointIdentifier::new(),
        facts: fixture_facts(1),
        attachments: Vec::new(),
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
    let endpoint = EndpointIdentifier::new();
    let documents: Vec<_> = (1..=257)
        .map(|sequence| IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(sequence),
            revision: 0,
            endpoint_identifier: endpoint,
            facts: fixture_facts(1),
            attachments: Vec::new(),
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
