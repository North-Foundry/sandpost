//! Search-outbox conformance checks.
use sandpost_storage::Storage;

use crate::ConformanceFailure;
use crate::support::{simple_message, unwrap_storage, verify, verify_equal};

/// Verify the durable search-outbox protocol and its monotonic watermarks.
pub async fn search_outbox(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "search_outbox";
    // Start from a fully acknowledged watermark so this check observes only its own operations.
    let initial = unwrap_storage(CHECK, "search_status", storage.search_status().await)?;
    unwrap_storage(
        CHECK,
        "initial acknowledge",
        storage
            .acknowledge_search_operations(initial.latest_sequence)
            .await,
    )?;
    let baseline = unwrap_storage(CHECK, "baseline status", storage.search_status().await)?;
    verify_equal(
        CHECK,
        "baseline indexed watermark",
        baseline.indexed_sequence,
        baseline.latest_sequence,
    )?;
    verify_equal(
        CHECK,
        "baseline pending operations",
        baseline.pending_operations,
        0,
    )?;
    let message = simple_message("search target");
    unwrap_storage(
        CHECK,
        "insert search message",
        storage.insert_message(&message).await,
    )?;
    let after = unwrap_storage(CHECK, "status after insert", storage.search_status().await)?;
    verify(
        CHECK,
        after.latest_sequence > baseline.latest_sequence,
        "an inserted message must advance the latest search sequence",
    )?;
    verify_equal(
        CHECK,
        "indexed watermark after insert",
        after.indexed_sequence,
        baseline.indexed_sequence,
    )?;
    verify_equal(
        CHECK,
        "pending operations match watermarks",
        after.pending_operations,
        after.latest_sequence - after.indexed_sequence,
    )?;

    let operations = unwrap_storage(
        CHECK,
        "pending_search_operations",
        storage
            .pending_search_operations(baseline.indexed_sequence, 64)
            .await,
    )?;
    verify(
        CHECK,
        !operations.is_empty(),
        "an inserted message must enqueue at least one operation",
    )?;
    verify(
        CHECK,
        operations
            .windows(2)
            .all(|window| window[0].sequence < window[1].sequence),
        "pending operations must be ordered by sequence",
    )?;
    verify(
        CHECK,
        operations.iter().all(|operation| {
            operation.sequence > baseline.indexed_sequence
                && operation.sequence <= after.latest_sequence
        }),
        "pending operations must fall within the unacknowledged range",
    )?;
    verify(
        CHECK,
        operations
            .iter()
            .any(|operation| operation.message_identifier == message.identifier),
        "a pending operation must reference the inserted message",
    )?;

    let (batch_status, batch) = unwrap_storage(
        CHECK,
        "search_batch",
        storage.search_batch(baseline.indexed_sequence, 64).await,
    )?;
    verify_equal(CHECK, "search_batch status", batch_status, after)?;
    verify_equal(CHECK, "search_batch operations", batch, operations.clone())?;

    let first = operations[0].sequence;
    unwrap_storage(
        CHECK,
        "partial acknowledge",
        storage.acknowledge_search_operations(first).await,
    )?;
    let partial = unwrap_storage(
        CHECK,
        "status after partial ack",
        storage.search_status().await,
    )?;
    verify(
        CHECK,
        partial.indexed_sequence >= after.indexed_sequence,
        "acknowledgement must not move the indexed watermark backward",
    )?;
    verify_equal(
        CHECK,
        "partial indexed watermark",
        partial.indexed_sequence,
        first,
    )?;
    verify_equal(
        CHECK,
        "partial pending operations",
        partial.pending_operations,
        partial.latest_sequence - partial.indexed_sequence,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "remaining operations",
            storage
                .pending_search_operations(baseline.indexed_sequence, 64)
                .await,
        )?
        .iter()
        .all(|operation| operation.sequence > first),
        "acknowledged operations must be pruned",
    )?;

    unwrap_storage(
        CHECK,
        "acknowledge an older watermark",
        storage.acknowledge_search_operations(0).await,
    )?;
    verify_equal(
        CHECK,
        "idempotent indexed watermark",
        unwrap_storage(
            CHECK,
            "status after older ack",
            storage.search_status().await,
        )?
        .indexed_sequence,
        partial.indexed_sequence,
    )?;

    unwrap_storage(
        CHECK,
        "acknowledge beyond the latest sequence",
        storage
            .acknowledge_search_operations(partial.latest_sequence + 1)
            .await,
    )?;
    let full = unwrap_storage(
        CHECK,
        "status after full ack",
        storage.search_status().await,
    )?;
    verify_equal(
        CHECK,
        "saturated indexed watermark",
        full.indexed_sequence,
        full.latest_sequence,
    )?;
    verify_equal(
        CHECK,
        "fully acknowledged pending operations",
        full.pending_operations,
        0,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "operations after full ack",
            storage
                .pending_search_operations(baseline.indexed_sequence, 64)
                .await,
        )?
        .is_empty(),
        "a fully acknowledged outbox must have no pending operations",
    )?;
    Ok(())
}
