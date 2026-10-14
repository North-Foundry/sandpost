//! Quiet-window and maximum-wait regressions for search synchronization.

use super::*;

/// Keep an empty outbox idle and schedule new pending work after its first observation.
#[test]
fn indexing_debounce_ignores_empty_state() {
    let mut debounce = IndexingDebounce::default();
    debounce.observe(Duration::ZERO, 0, 0);
    assert!(!debounce.is_due(Duration::from_secs(10)));

    debounce.observe(Duration::from_secs(10), 1, 0);
    assert!(!debounce.is_due(Duration::from_millis(10_049)));
    assert!(debounce.is_due(Duration::from_millis(10_050)));
}

/// Reset the quiet deadline after each mutation while preserving the first pending time.
#[test]
fn indexing_debounce_resets_quiet_window_and_enforces_maximum_wait() {
    let mut debounce = IndexingDebounce::default();
    debounce.observe(Duration::ZERO, 1, 0);
    debounce.observe(Duration::from_millis(40), 2, 0);
    assert!(!debounce.is_due(Duration::from_millis(89)));
    assert!(debounce.is_due(Duration::from_millis(90)));

    let mut maximum_wait = IndexingDebounce::default();
    maximum_wait.observe(Duration::ZERO, 1, 0);
    for step in 1..=199 {
        let millisecond = step * 25;
        maximum_wait.observe(Duration::from_millis(millisecond), step + 1, 0);
        assert!(!maximum_wait.is_due(Duration::from_millis(millisecond)));
    }
    assert!(!maximum_wait.is_due(Duration::from_millis(4_999)));
    assert!(maximum_wait.is_due(Duration::from_secs(5)));
}

/// Clear pending debounce state after batches bring the indexed watermark to the latest.
#[test]
fn indexing_debounce_clears_after_drain() {
    let mut debounce = IndexingDebounce::default();
    debounce.observe(Duration::ZERO, 1, 0);
    assert!(debounce.is_due(INDEXING_QUIET_PERIOD));

    debounce.observe(Duration::from_secs(1), 1, 1);
    assert!(!debounce.is_due(Duration::from_secs(10)));
}
