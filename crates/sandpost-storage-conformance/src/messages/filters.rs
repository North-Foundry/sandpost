//! Message filter equivalence checks against the canonical query evaluator.
use std::collections::BTreeSet;

use sandpost_core::{Message, MessageFacts, MessageIdentifier};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_storage::{MessageListQuery, Storage};

use crate::ConformanceFailure;
use crate::support::{attachment, failure, headers, mailbox, unwrap_storage, verify, verify_equal};

/// Verify that a storage filter selects exactly the rows the canonical evaluator selects.
pub async fn message_filter_equivalence(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "message_filter_equivalence";
    let mut inserted = Vec::new();
    for message in filter_fixtures() {
        unwrap_storage(
            CHECK,
            "insert filter message",
            storage.insert_message(&message).await,
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
                    filter: Some(expression.clone()),
                    before: None,
                    limit: usize::MAX,
                })
                .await,
        )?;
        // Other checks share the mail pool; compare only this check's fixtures.
        let actual: BTreeSet<MessageIdentifier> = rows
            .iter()
            .map(|summary| summary.identifier)
            .filter(|identifier| inserted.iter().any(|(fixture, _)| fixture == identifier))
            .collect();
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
                message_identifier: Some("<alpha@example.test>".to_owned()),
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
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: "a".to_owned(),
                ..MessageFacts::default()
            },
            raw_message: b"glob character class trap".to_vec(),
            attachments: Vec::new(),
        },
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: "[ab]".to_owned(),
                ..MessageFacts::default()
            },
            raw_message: b"literal glob brackets".to_vec(),
            attachments: Vec::new(),
        },
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: "Café ☕".to_owned(),
                ..MessageFacts::default()
            },
            raw_message: b"unicode suffix".to_vec(),
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
            Field::Subject,
            Operator::Matches,
            Value::String("[ab]".to_owned()),
        ),
        Expression::Not(Box::new(predicate(
            Field::Subject,
            Operator::Matches,
            Value::String("[ab]".to_owned()),
        ))),
        predicate(
            Field::MessageIdentifier,
            Operator::Contains,
            Value::String(String::new()),
        ),
        predicate(
            Field::MessageIdentifier,
            Operator::StartsWith,
            Value::String(String::new()),
        ),
        predicate(
            Field::MessageIdentifier,
            Operator::EndsWith,
            Value::String(String::new()),
        ),
        Expression::Not(Box::new(predicate(
            Field::MessageIdentifier,
            Operator::Contains,
            Value::String(String::new()),
        ))),
        predicate(
            Field::Subject,
            Operator::EndsWith,
            Value::String("report".to_owned()),
        ),
        predicate(
            Field::Subject,
            Operator::EndsWith,
            Value::String("é ☕".to_owned()),
        ),
        predicate(
            Field::FromAddress,
            Operator::EndsWith,
            Value::String("@example.test".to_owned()),
        ),
        predicate(
            Field::Content,
            Operator::EndsWith,
            Value::String("REPORT".to_owned()),
        ),
        predicate(
            Field::Header("x-team".to_owned()),
            Operator::EndsWith,
            Value::String("ue".to_owned()),
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
