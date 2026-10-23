//! External SMTP delivery: reusable SMTP Outbound connections, scope routing actions, and the
//! durable delivery jobs they create for captured mail.
//!
//! An SMTP Outbound only describes how to reach an external SMTP server; it never decides which
//! mail is delivered. Routing actions belong to scopes and reference an SMTP Outbound: a relay
//! action delivers a message to its original SMTP envelope recipients, a forward action delivers
//! a copy to explicitly configured recipients. Delivery never changes the stored message.
use crate::{
    MessageIdentifier, OutboundDeliveryIdentifier, RoutingActionIdentifier, ScopeIdentifier,
    SmtpOutboundIdentifier,
};
use serde::{Deserialize, Serialize};

/// Implement stable storage and transport names for a fieldless enum.
macro_rules! stable_names {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            /// Every value, used to parse stored names.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /**
             * Return the stable lowercase name used in storage and transport representations.
             */
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            /**
             * Parse a stable name, rejecting unknown values.
             */
            pub fn parse(value: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|candidate| candidate.as_str() == value)
            }
        }
    };
}

/// How the connection to an SMTP Outbound is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmtpOutboundSecurity {
    /// Plain SMTP without TLS; only used when explicitly configured.
    #[serde(rename = "none")]
    None,
    /// Plain connection upgraded with STARTTLS; delivery fails rather than continuing without TLS.
    #[serde(rename = "starttls")]
    StartTls,
    /// TLS from the first byte (implicit TLS, usually port 465).
    #[serde(rename = "tls")]
    Tls,
}

stable_names!(SmtpOutboundSecurity { None => "none", StartTls => "starttls", Tls => "tls" });

/// The SMTP AUTH mechanism used with an SMTP Outbound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpOutboundAuthentication {
    /// The server accepts mail without authentication.
    None,
    /// `AUTH PLAIN` (RFC 4616).
    Plain,
    /// `AUTH LOGIN`.
    Login,
}

stable_names!(SmtpOutboundAuthentication { None => "none", Plain => "plain", Login => "login" });

/// A reusable, instance-level connection to an external SMTP server.
///
/// The password is stored encrypted and is deliberately absent from this representation;
/// `password_configured` only reports whether one is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpOutbound {
    #[serde(rename = "id")]
    pub identifier: SmtpOutboundIdentifier,
    /// Unique human-readable name, such as "Pippo".
    pub name: String,
    /// Host name or IP address of the SMTP server.
    pub host: String,
    pub port: u16,
    pub security: SmtpOutboundSecurity,
    #[serde(rename = "auth_type")]
    pub authentication: SmtpOutboundAuthentication,
    pub username: Option<String>,
    pub password_configured: bool,
    /// Disabled outbounds create no deliveries and fail queued ones at their next attempt.
    pub enabled: bool,
    /// Replaces the `From` header of delivered copies when set.
    pub override_from: Option<String>,
    /// Replaces the SMTP `MAIL FROM` reverse path when set.
    pub envelope_from: Option<String>,
    /// Seconds allowed to connect, negotiate TLS and greet; the default applies when absent.
    #[serde(rename = "connect_timeout")]
    pub connect_timeout_seconds: Option<u32>,
    /// Seconds allowed for each SMTP command after connecting; the default applies when absent.
    #[serde(rename = "smtp_timeout")]
    pub smtp_timeout_seconds: Option<u32>,
    /// Verify the server certificate and host name; on unless explicitly disabled.
    #[serde(rename = "tls_verify")]
    pub verify_certificates: bool,
    /// The name sent in `EHLO`; a default applies when absent.
    pub helo_name: Option<String>,
    /// Simultaneous connections to this server; the default applies when absent.
    #[serde(rename = "max_connections")]
    pub maximum_connections: Option<u32>,
    /// Glob patterns a recipient address must match to be delivered. Empty permits nobody.
    pub allowed_recipients: Vec<String>,
    /// Unix seconds, UTC.
    pub created_at: i64,
    /// Unix seconds, UTC.
    pub updated_at: i64,
}

/// How a routing action chooses the recipients of a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingActionKind {
    /// Deliver to the message's original SMTP envelope recipients.
    Relay,
    /// Deliver a copy to the action's configured recipients only.
    Forward,
}

stable_names!(RoutingActionKind { Relay => "relay", Forward => "forward" });

/// A delivery rule attached to one scope, using one SMTP Outbound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingAction {
    #[serde(rename = "id")]
    pub identifier: RoutingActionIdentifier,
    #[serde(rename = "scope_id")]
    pub scope_identifier: ScopeIdentifier,
    #[serde(rename = "smtp_outbound_id")]
    pub smtp_outbound_identifier: SmtpOutboundIdentifier,
    #[serde(rename = "action_type")]
    pub kind: RoutingActionKind,
    /// Normalized recipients of a forward action; empty for relay actions.
    pub forward_recipients: Vec<String>,
    pub enabled: bool,
    /// Unix seconds, UTC.
    pub created_at: i64,
    /// Unix seconds, UTC.
    pub updated_at: i64,
}

/// The lifecycle state of one delivery job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutboundDeliveryStatus {
    /// Created and waiting for its first attempt.
    Pending,
    /// An attempt is in progress.
    Processing,
    /// A transient failure left recipients undelivered; another attempt is scheduled.
    Retrying,
    /// Every recipient was accepted by the SMTP server.
    Delivered,
    /// At least one recipient failed permanently or the attempts were exhausted.
    Failed,
}

stable_names!(OutboundDeliveryStatus {
    Pending => "pending",
    Processing => "processing",
    Retrying => "retrying",
    Delivered => "delivered",
    Failed => "failed",
});

impl OutboundDeliveryStatus {
    /**
     * Report whether the job has ended and will change only through an explicit retry.
     */
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Delivered | Self::Failed)
    }
}

/// The state of one envelope recipient of a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutboundRecipientStatus {
    /// Not accepted yet; it is included in the next attempt.
    Pending,
    /// The SMTP server accepted the message for this recipient.
    Delivered,
    /// Refused permanently, by policy, or after the last attempt.
    Failed,
}

stable_names!(OutboundRecipientStatus {
    Pending => "pending",
    Delivered => "delivered",
    Failed => "failed",
});

/// Why a delivery, recipient, attempt or connection test failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutboundFailureKind {
    /// The SMTP Outbound or routing action is missing, disabled or incomplete.
    Configuration,
    /// A safety rule refused the destination or a recipient.
    Policy,
    /// Name resolution, connection, timeout or a dropped connection.
    Connection,
    /// TLS negotiation or certificate verification failed, or STARTTLS was not offered.
    Tls,
    /// The server rejected the credentials or offers no usable mechanism.
    Authentication,
    /// A transient SMTP reply (4xx).
    Temporary,
    /// A permanent SMTP reply (5xx).
    Permanent,
    /// The service stopped while an attempt was in progress.
    Interrupted,
}

stable_names!(OutboundFailureKind {
    Configuration => "configuration",
    Policy => "policy",
    Connection => "connection",
    Tls => "tls",
    Authentication => "authentication",
    Temporary => "temporary",
    Permanent => "permanent",
    Interrupted => "interrupted",
});

/// A structured outbound failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundFailure {
    pub kind: OutboundFailureKind,
    /// Human-readable explanation, including the SMTP reply text when there is one.
    pub message: String,
    /// The SMTP reply code when the failure came from a reply.
    pub code: Option<u16>,
}

/// One envelope recipient of a delivery and its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundRecipient {
    pub address: String,
    pub status: OutboundRecipientStatus,
    /// The most recent failure for this recipient.
    pub error: Option<OutboundFailure>,
    /// The server's final reply when the recipient was delivered.
    pub response: Option<String>,
    /// Unix seconds, UTC.
    pub delivered_at: Option<i64>,
}

/// One delivery job: a message delivered through one routing action's SMTP Outbound.
///
/// References to the scope, action and outbound become absent when those are deleted; the
/// action type and outbound name are kept as recorded when the job was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundDelivery {
    #[serde(rename = "id")]
    pub identifier: OutboundDeliveryIdentifier,
    /// Increasing creation order, used for newest-first keyset pagination.
    #[serde(rename = "seq")]
    pub sequence: u64,
    #[serde(rename = "message_id")]
    pub message_identifier: MessageIdentifier,
    /// The subject of the delivered message, read in the same snapshot.
    pub subject: String,
    #[serde(rename = "scope_id")]
    pub scope_identifier: Option<ScopeIdentifier>,
    #[serde(rename = "routing_action_id")]
    pub routing_action_identifier: Option<RoutingActionIdentifier>,
    #[serde(rename = "smtp_outbound_id")]
    pub smtp_outbound_identifier: Option<SmtpOutboundIdentifier>,
    pub smtp_outbound_name: String,
    #[serde(rename = "action_type")]
    pub kind: RoutingActionKind,
    pub status: OutboundDeliveryStatus,
    /// The reverse path used for the latest attempt; absent for the null reverse path.
    pub envelope_from: Option<String>,
    /// Attempts started, including interrupted ones.
    pub attempt_count: u32,
    pub last_error: Option<OutboundFailure>,
    /// Unix seconds, UTC.
    pub created_at: i64,
    /// Unix seconds, UTC.
    pub last_attempt_at: Option<i64>,
    /// Unix seconds, UTC, when the next attempt may start; absent for terminal jobs.
    pub next_attempt_at: Option<i64>,
    /// Unix seconds, UTC, when the last recipient was delivered.
    pub delivered_at: Option<i64>,
    pub recipients: Vec<OutboundRecipient>,
}

/// The result of one delivery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutboundAttemptOutcome {
    /// Every attempted recipient was delivered.
    Delivered,
    /// Some recipients were delivered and others were not.
    Partial,
    /// Nothing was delivered and recipients remain for a later attempt.
    Deferred,
    /// Nothing was delivered and no recipient remains.
    Failed,
    /// The service stopped during the attempt.
    Interrupted,
}

stable_names!(OutboundAttemptOutcome {
    Delivered => "delivered",
    Partial => "partial",
    Deferred => "deferred",
    Failed => "failed",
    Interrupted => "interrupted",
});

/// One recorded delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundDeliveryAttempt {
    /// 1-based attempt number.
    pub number: u32,
    /// Unix seconds, UTC.
    pub started_at: i64,
    /// Unix seconds, UTC; absent while the attempt runs.
    pub finished_at: Option<i64>,
    /// Absent while the attempt runs.
    pub outcome: Option<OutboundAttemptOutcome>,
    /// The session-level failure, when the attempt failed as a whole.
    pub error: Option<OutboundFailure>,
    pub delivered_recipients: u32,
    pub deferred_recipients: u32,
    pub failed_recipients: u32,
}
