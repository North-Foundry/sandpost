use sandpost_core::MessageSequence;
use sandpost_match::MatchDelta;
use std::collections::BTreeSet;

/// Verify message-set differences preserve only additions and removals.
#[test]
fn set_deltas() {
    let old = BTreeSet::from([MessageSequence(1), MessageSequence(2)]);
    let new = BTreeSet::from([MessageSequence(2), MessageSequence(3)]);
    assert_eq!(
        MatchDelta::between(&old, &new),
        MatchDelta {
            added: BTreeSet::from([MessageSequence(3)]),
            removed: BTreeSet::from([MessageSequence(1)])
        }
    );
}
