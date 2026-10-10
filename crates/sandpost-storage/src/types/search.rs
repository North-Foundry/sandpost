//! Search storage types.
use sandpost_core::{MessageFacts, MessageIdentifier, MessageSequence};
use serde::{Deserialize, Serialize};

/// Normalized search-index input without raw MIME bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedMessage {
    /// Durable, globally increasing search revision for this exact mail snapshot.
    pub revision: u64,
    pub identifier: MessageIdentifier,
    pub sequence: MessageSequence,
    pub facts: MessageFacts,
}

/// One durable mail change awaiting search-index synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOperation {
    pub sequence: u64,
    pub message_identifier: MessageIdentifier,
}

/// Durable search-index watermarks and the count of queued changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSynchronization {
    pub latest_sequence: u64,
    pub indexed_sequence: u64,
    pub pending_operations: u64,
}
