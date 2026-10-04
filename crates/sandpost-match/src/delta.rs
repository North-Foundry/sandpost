//! Added and removed message sequences between two materialized match sets.
use sandpost_core::MessageSequence;
use std::collections::BTreeSet;

/// Set representation can be replaced by compressed bitmaps without changing
/// the semantic delta contract. No mailbox or inbox owns copies of messages.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MatchDelta {
    pub added: BTreeSet<MessageSequence>,
    pub removed: BTreeSet<MessageSequence>,
}
impl MatchDelta {
    /// Compute added and removed message sequences relative to the previous match set.
    pub fn between(old: &BTreeSet<MessageSequence>, new: &BTreeSet<MessageSequence>) -> Self {
        Self {
            added: new.difference(old).copied().collect(),
            removed: old.difference(new).copied().collect(),
        }
    }
}
