//! SMTP Outbound, routing action and outbound delivery storage types.
use crate::FilterExpression;
use sandpost_core::{
    MessageIdentifier, OutboundAttemptOutcome, OutboundDeliveryIdentifier, OutboundDeliveryStatus,
    OutboundFailure, OutboundRecipientStatus, RoutingActionIdentifier, RoutingActionKind,
    ScopeIdentifier, SmtpOutbound, SmtpOutboundAuthentication, SmtpOutboundIdentifier,
    SmtpOutboundSecurity, UserIdentifier,
};

/// The validated, non-secret settings of an SMTP Outbound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpOutboundSettings {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub security: SmtpOutboundSecurity,
    pub authentication: SmtpOutboundAuthentication,
    pub username: Option<String>,
    pub enabled: bool,
    pub override_from: Option<String>,
    pub envelope_from: Option<String>,
    pub connect_timeout_seconds: Option<u32>,
    pub smtp_timeout_seconds: Option<u32>,
    pub verify_certificates: bool,
    pub helo_name: Option<String>,
    pub maximum_connections: Option<u32>,
    pub allowed_recipients: Vec<String>,
}

/// A new SMTP Outbound. The password crosses the storage boundary only encrypted.
///
/// Intentionally lacks Debug so credential material is never logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct NewSmtpOutbound {
    pub identifier: SmtpOutboundIdentifier,
    pub settings: SmtpOutboundSettings,
    /// The encrypted password, when the outbound authenticates.
    pub encrypted_password: Option<Vec<u8>>,
    /// Unix seconds, UTC.
    pub created_at: i64,
}

/// How an update treats the stored encrypted password.
///
/// Intentionally lacks Debug so credential material is never logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub enum SmtpOutboundPassword {
    /// Keep the stored password unchanged.
    Keep,
    /// Replace it with a newly encrypted password.
    Replace(Vec<u8>),
    /// Remove the stored password.
    Clear,
}

/// An SMTP Outbound with its encrypted password, read only to connect to the server.
///
/// Intentionally lacks Debug so credential material is never logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpOutboundCredential {
    pub outbound: SmtpOutbound,
    pub encrypted_password: Option<Vec<u8>>,
}

/// A new routing action attached to a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRoutingAction {
    pub identifier: RoutingActionIdentifier,
    pub scope_identifier: ScopeIdentifier,
    pub smtp_outbound_identifier: SmtpOutboundIdentifier,
    pub kind: RoutingActionKind,
    pub forward_recipients: Vec<String>,
    pub enabled: bool,
    /// Unix seconds, UTC.
    pub created_at: i64,
}

/// A complete replacement of a routing action's mutable fields; its scope never changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingActionUpdate {
    pub smtp_outbound_identifier: SmtpOutboundIdentifier,
    pub kind: RoutingActionKind,
    pub forward_recipients: Vec<String>,
    pub enabled: bool,
}

/// One envelope recipient of a new delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutboundRecipient {
    pub address: String,
    /// `Pending` to deliver, or `Failed` when a policy refused the recipient up front.
    pub status: OutboundRecipientStatus,
    pub error: Option<OutboundFailure>,
}

/// A delivery job created when routing a stored message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutboundDelivery {
    pub identifier: OutboundDeliveryIdentifier,
    pub scope_identifier: ScopeIdentifier,
    pub routing_action_identifier: RoutingActionIdentifier,
    pub smtp_outbound_identifier: SmtpOutboundIdentifier,
    /// The outbound name when the job was created, kept after the outbound is deleted.
    pub smtp_outbound_name: String,
    pub kind: RoutingActionKind,
    /// `Pending`, or `Failed` when nothing can be delivered (every recipient refused).
    pub status: OutboundDeliveryStatus,
    pub last_error: Option<OutboundFailure>,
    pub recipients: Vec<NewOutboundRecipient>,
    /// Unix seconds, UTC.
    pub created_at: i64,
}

/// The new state of one recipient after an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundRecipientUpdate {
    pub address: String,
    pub status: OutboundRecipientStatus,
    pub error: Option<OutboundFailure>,
    pub response: Option<String>,
    /// Unix seconds, UTC.
    pub delivered_at: Option<i64>,
}

/// The result of one attempt and the job state that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundAttemptResult {
    /// Unix seconds, UTC.
    pub finished_at: i64,
    /// The reverse path used by the attempt; absent for the null reverse path.
    pub envelope_from: Option<String>,
    pub outcome: OutboundAttemptOutcome,
    /// The session-level failure, when the attempt failed as a whole.
    pub error: Option<OutboundFailure>,
    /// Recipients whose state changed; others keep their stored state.
    pub recipients: Vec<OutboundRecipientUpdate>,
    /// `Delivered`, `Failed` or `Retrying`.
    pub status: OutboundDeliveryStatus,
    /// When `status` is `Retrying`, the earliest time of the next attempt.
    pub next_attempt_at: Option<i64>,
}

/// A bounded, newest-first page of delivery jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryQuery {
    /// Only deliveries of this message.
    pub message: Option<MessageIdentifier>,
    /// Only deliveries in this status.
    pub status: Option<OutboundDeliveryStatus>,
    /// Exclusive sequence cursor.
    pub before: Option<u64>,
    pub limit: usize,
    /// Keep only deliveries whose message this reader can read; `None` keeps every delivery.
    pub reader: Option<OutboundDeliveryReader>,
}

/// A reader whose visibility bounds a delivery listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryReader {
    pub user: UserIdentifier,
    /// The reader's composed authorization; the reader's own deletions are excluded too.
    pub filter: FilterExpression,
}
