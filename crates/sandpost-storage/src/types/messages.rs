//! Messages storage types.
use super::FilterExpression;
use sandpost_core::{Mailbox, MessageIdentifier, MessageSequence};
use serde::{Deserialize, Serialize};

/// A bounded, backend-independent message listing request.
#[derive(Debug, Clone)]
pub struct MessageListQuery {
    /// Optional canonical filter applied to normalized message facts.
    pub filter: Option<FilterExpression>,
    /// Exclusive upper sequence bound for newest-first keyset pagination.
    pub before: Option<MessageSequence>,
    /// Requested number of summaries; backends cap this at their documented maximum.
    pub limit: usize,
}

/// Lightweight message projection returned by list and hydration reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    #[serde(rename = "seq")]
    pub sequence: MessageSequence,
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    pub subject: String,
    pub from: Vec<Mailbox>,
    pub to: Vec<Mailbox>,
    pub envelope_to: Vec<Mailbox>,
    pub preview: String,
    pub received_at: i64,
    pub size: u64,
    pub attachment_count: u64,
}
