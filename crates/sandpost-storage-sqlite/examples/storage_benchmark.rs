//! Bounded benchmark for the public SQLite storage API.
//!
//! Run with `cargo run -p sandpost-storage-sqlite --example storage_benchmark --release`.

use sandpost_core::{
    GlobalRole, MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier, MessageSequence,
};
use sandpost_storage::{MessageListQuery, MessageStorage, NewUser, UserStorage};
use sandpost_storage_sqlite::SqliteStorage;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DATASET: usize = 1_000;
const ITERATIONS: usize = 100;
const CONCURRENCY: usize = 8;
const INDEX_LIMIT: usize = 64;

/// Open a temporary file database, run each workload, and close it before cleanup.
#[tokio::main]
async fn main() {
    let directory = std::env::temp_dir().join(format!("sandpost-benchmark-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create benchmark directory");
    let storage = Arc::new(
        SqliteStorage::open(directory.join("bench.sqlite3"))
            .await
            .expect("open benchmark database"),
    );
    storage
        .create_first_owner(&owner())
        .await
        .expect("create benchmark owner");

    measure("insert sequential", DATASET, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            async move {
                storage.insert_message(&message(1)).await.expect("insert");
            }
        }
    })
    .await;
    measure_concurrent("insert concurrent", ITERATIONS, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            async move {
                storage.insert_message(&message(2)).await.expect("insert");
            }
        }
    })
    .await;

    let indexed = storage
        .index_messages(MessageSequence(0), None, INDEX_LIMIT)
        .await
        .expect("read bounded index");
    assert!(
        indexed.len() <= INDEX_LIMIT,
        "index read respects its hard limit"
    );
    let identifiers = indexed
        .iter()
        .take(100)
        .map(|item| item.identifier)
        .collect::<Vec<_>>();
    let current_revisions = indexed
        .iter()
        .take(100)
        .map(|item| (item.identifier, item.revision))
        .collect::<Vec<_>>();

    measure("list single", ITERATIONS, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            async move {
                storage.list_messages(query(None, 100)).await.expect("list");
            }
        }
    })
    .await;
    measure_concurrent("list concurrent", ITERATIONS, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            async move {
                storage.list_messages(query(None, 100)).await.expect("list");
            }
        }
    })
    .await;
    let filter = sandpost_query::Expression::Predicate(sandpost_query::Predicate {
        field: sandpost_query::Field::Subject,
        operator: sandpost_query::Operator::Contains,
        value: sandpost_query::Value::String("benchmark".into()),
    });
    measure("list filtered", ITERATIONS, {
        let storage = Arc::clone(&storage);
        let filter = filter.clone();
        move || {
            let storage = Arc::clone(&storage);
            let filter = filter.clone();
            async move {
                storage
                    .list_messages(query(Some(filter), 100))
                    .await
                    .expect("filtered list");
            }
        }
    })
    .await;
    let summary_identifiers = identifiers.clone();
    measure("summary hydration", ITERATIONS, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            let identifiers = summary_identifiers.clone();
            async move {
                storage
                    .hydrate_messages(&identifiers)
                    .await
                    .expect("hydrate summaries");
            }
        }
    })
    .await;
    measure("indexed batch hydration", ITERATIONS, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            let identifiers = identifiers.clone();
            async move {
                storage
                    .indexed_messages(&identifiers)
                    .await
                    .expect("hydrate indexed facts");
            }
        }
    })
    .await;
    measure("current hydration", ITERATIONS, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            let current_revisions = current_revisions.clone();
            async move {
                storage
                    .hydrate_current_messages(&current_revisions)
                    .await
                    .expect("hydrate current revisions");
            }
        }
    })
    .await;
    measure("mixed insert and list", ITERATIONS, {
        let storage = Arc::clone(&storage);
        move || {
            let storage = Arc::clone(&storage);
            async move {
                let message = message(3);
                let (insert, list) = tokio::join!(
                    storage.insert_message(&message),
                    storage.list_messages(query(None, 50))
                );
                insert.expect("mixed insert");
                list.expect("mixed list");
            }
        }
    })
    .await;

    storage.close().await;
    let _ = std::fs::remove_dir_all(directory);
}

/// Return a benchmark owner record.
fn owner() -> NewUser {
    NewUser {
        name: "Benchmark Owner".into(),
        email: "owner@example.test".into(),
        password_hash: "benchmark-only".into(),
        global_role: GlobalRole::Owner,
        mail_access: MailAccess::All,
        personal_filter: None,
    }
}

/// Build a small normalized message with deterministic searchable facts.
fn message(index: usize) -> Message {
    let from = Mailbox {
        address: format!("sender{index}@example.test"),
        domain: "example.test".into(),
    };
    let to = Mailbox {
        address: "owner@example.test".into(),
        domain: "example.test".into(),
    };
    let raw_message = format!("From: {}\r\nTo: owner@example.test\r\nSubject: benchmark message {index}\r\n\r\nbody {index}", from.address).into_bytes();
    let size = raw_message.len() as u64;
    Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            envelope_from: Some(from.clone()),
            envelope_to: vec![to.clone()],
            from: vec![from],
            to: vec![to],
            subject: format!("benchmark message {index}"),
            text: format!("body searchable token{}", index % 10),
            message_identifier: Some(format!("<benchmark-{index}@example.test>")),
            received_at: 1_800_000_000 + index as i64,
            size,
            ..MessageFacts::default()
        },
        raw_message,
        attachments: Vec::new(),
    }
}

/// Build a bounded newest-first message query.
fn query(filter: Option<sandpost_storage::FilterExpression>, limit: usize) -> MessageListQuery {
    MessageListQuery {
        filter,
        before: None,
        limit,
    }
}

/// Measure sequential calls and report operation-level latency and wall-clock throughput.
async fn measure<F, Fut>(label: &str, iterations: usize, mut operation: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut samples = Vec::with_capacity(iterations);
    let started = Instant::now();
    for _ in 0..iterations {
        let sample = Instant::now();
        operation().await;
        samples.push(sample.elapsed());
    }
    report(label, samples, started.elapsed());
}

/// Launch each batch fully before awaiting and record each task's own latency.
async fn measure_concurrent<F, Fut>(label: &str, batches: usize, operation: F)
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let operation = Arc::new(operation);
    let mut samples = Vec::with_capacity(batches * CONCURRENCY);
    let started = Instant::now();
    for _ in 0..batches {
        let jobs = (0..CONCURRENCY)
            .map(|_| {
                let operation = Arc::clone(&operation);
                tokio::spawn(async move {
                    let started = Instant::now();
                    operation().await;
                    started.elapsed()
                })
            })
            .collect::<Vec<_>>();
        for job in jobs {
            samples.push(job.await.expect("benchmark task"));
        }
    }
    report(label, samples, started.elapsed());
}

/// Print nearest-rank p50/p95/p99 latency and completed operations per elapsed second.
fn report(label: &str, mut samples: Vec<Duration>, elapsed: Duration) {
    samples.sort_unstable();
    let percentile =
        |value: usize| samples[((samples.len() - 1) * value).div_ceil(100)].as_secs_f64() * 1_000.0;
    println!(
        "{label:24} p50={:.3}ms p95={:.3}ms p99={:.3}ms throughput={:.1}/s",
        percentile(50),
        percentile(95),
        percentile(99),
        samples.len() as f64 / elapsed.as_secs_f64()
    );
}
