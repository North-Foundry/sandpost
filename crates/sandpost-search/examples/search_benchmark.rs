//! Repeatable local timings; run with `cargo run -p sandpost-search --example search_benchmark`.

use sandpost_core::{MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_search::{IndexedDocument, MessageQuery, SearchIndex};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

/// Measure the median of seven complete requests after warming the index reader.
fn median_search(index: &SearchIndex, filter: Expression) -> Duration {
    let request = MessageQuery {
        authorization: None,
        filter,
        message_identifier: None,
        before: None,
        limit: 50,
    };
    black_box(index.search_hits(&request).unwrap());
    let mut times = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        black_box(index.search_hits(&request).unwrap());
        times.push(started.elapsed());
    }
    times.sort();
    times[times.len() / 2]
}

/// Index 400 messages with 64 KiB bodies and compare broad and selective request costs.
fn main() {
    let documents: Vec<_> = (1..=400)
        .map(|sequence| IndexedDocument {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(sequence),
            revision: sequence,
            facts: MessageFacts {
                subject: format!("benchmark message {sequence}"),
                text: format!(
                    "{}{}",
                    "A realistic mail paragraph with Café and words.\n".repeat(1400),
                    if sequence == 1 { "zz" } else { "" }
                ),
                size: 65_536,
                received_at: sequence as i64,
                ..Default::default()
            },
        })
        .collect();
    let index = SearchIndex::memory().unwrap();
    let started = Instant::now();
    for batch in documents.chunks(256) {
        index.upsert_batch(batch).unwrap();
    }
    index.commit(400).unwrap();
    println!(
        "index_and_commit_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    for (name, expression) in [
        ("list", Expression::True),
        (
            "numeric",
            Expression::Predicate(Predicate {
                field: Field::Size,
                operator: Operator::GreaterThanOrEqual,
                value: Value::Number(65_000),
            }),
        ),
        (
            "short_substring",
            Expression::Predicate(Predicate {
                field: Field::Text,
                operator: Operator::Contains,
                value: Value::String("zz".into()),
            }),
        ),
        (
            "subject",
            Expression::Predicate(Predicate {
                field: Field::Subject,
                operator: Operator::Equal,
                value: Value::String("benchmark message 200".into()),
            }),
        ),
    ] {
        println!(
            "{name}_median_ms={:.3}",
            median_search(&index, expression).as_secs_f64() * 1000.0
        );
    }
}
