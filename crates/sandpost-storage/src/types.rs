//! Backend-independent request and result types used by the storage contract.
use sandpost_core::{
    Attachment, EndpointIdentifier, GlobalRole, Mailbox, MessageFacts, MessageIdentifier,
    MessageSequence,
};
use serde::{Deserialize, Serialize};

/// The backend-independent filter representation accepted by message queries.
///
/// This is the canonical SandPost query AST produced by the shared query language parser. A
/// backend compiles it into its own execution strategy but never receives raw SQL.
pub type FilterExpression = sandpost_query::Expression;

/// A new user supplied to the user storage creation operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub global_role: GlobalRole,
    pub personal_filter: Option<String>,
}

/// A complete replacement of a user's mutable fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateUser {
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub global_role: GlobalRole,
    pub personal_filter: Option<String>,
}

/// A bounded, backend-independent message listing request.
#[derive(Debug, Clone)]
pub struct MessageListQuery {
    /// Optional endpoint restriction.
    pub endpoint: Option<EndpointIdentifier>,
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
    pub endpoint_identifier: EndpointIdentifier,
    pub subject: String,
    pub from: Vec<Mailbox>,
    pub to: Vec<Mailbox>,
    pub envelope_to: Vec<Mailbox>,
    pub preview: String,
    pub received_at: i64,
    pub size: u64,
    pub attachment_count: u64,
}

/// Normalized search-index input without raw MIME bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedMessage {
    /// Durable, globally increasing search revision for this exact mail snapshot.
    pub revision: u64,
    pub identifier: MessageIdentifier,
    pub sequence: MessageSequence,
    pub endpoint_identifier: EndpointIdentifier,
    pub facts: MessageFacts,
    pub attachments: Vec<Attachment>,
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
