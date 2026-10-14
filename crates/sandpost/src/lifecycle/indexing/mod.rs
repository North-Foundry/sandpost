//! Schedule durable search synchronization after quiet periods and during shutdown.

use super::supervision::wait_for_shutdown;
use sandpost_application::SearchCoordinator;
use std::time::Duration;
use tokio::sync::watch;

/// Wait for a quiet indexing window, then drain bounded outbox batches under supervision.
pub(super) async fn indexing_worker(
    search: SearchCoordinator,
    shutdown: watch::Receiver<bool>,
) -> Result<(), sandpost_application::ApplicationError> {
    tracing::info!("search indexing worker started");
    let mut interval = tokio::time::interval(Duration::from_millis(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let started = tokio::time::Instant::now();
    let mut debounce = IndexingDebounce::default();
    loop {
        tokio::select! {
            _ = wait_for_shutdown(shutdown.clone()) => {
                search.catch_up().await?;
                return Ok(());
            }
            _ = interval.tick() => {
                let status = search.status().await?;
                let now = started.elapsed();
                debounce.observe(now, status.latest_sequence, status.indexed_sequence);
                if debounce.is_due(now) {
                    search.catch_up().await?;
                    let status = search.status().await?;
                    debounce.observe(
                        started.elapsed(),
                        status.latest_sequence,
                        status.indexed_sequence,
                    );
                }
            }
        }
    }
}

/// Track the quiet period and maximum wait for the latest durable mutation sequence.
#[derive(Default)]
struct IndexingDebounce {
    latest_sequence: Option<u64>,
    pending_since: Option<Duration>,
    quiet_since: Option<Duration>,
}

const INDEXING_QUIET_PERIOD: Duration = Duration::from_millis(50);
const INDEXING_MAXIMUM_WAIT: Duration = Duration::from_secs(5);

impl IndexingDebounce {
    /// Observe backend-independent mutation and index watermarks at a monotonic elapsed time.
    fn observe(&mut self, now: Duration, latest_sequence: u64, indexed_sequence: u64) {
        if latest_sequence == indexed_sequence {
            self.latest_sequence = Some(latest_sequence);
            self.pending_since = None;
            self.quiet_since = None;
            return;
        }

        if self.latest_sequence != Some(latest_sequence) {
            self.latest_sequence = Some(latest_sequence);
            self.quiet_since = Some(now);
        }
        if self.pending_since.is_none() {
            self.pending_since = Some(now);
            self.quiet_since = Some(now);
        }
    }

    /// Report whether 50 milliseconds of quiet or five seconds of pending time has elapsed.
    fn is_due(&self, now: Duration) -> bool {
        self.pending_since
            .is_some_and(|since| now.saturating_sub(since) >= INDEXING_MAXIMUM_WAIT)
            || self
                .quiet_since
                .is_some_and(|since| now.saturating_sub(since) >= INDEXING_QUIET_PERIOD)
    }
}

#[cfg(test)]
mod tests;
