//! Centrally administered SMTP access credentials for the global SMTP server.
use crate::SmtpAccessIdentifier;
use serde::{Deserialize, Serialize};

/// One independent SMTP credential that external applications use to authenticate.
///
/// An access is a global resource: it is not owned by a user or scope. Mail submitted with it
/// joins the instance's single mail pool. The password exists only as a
/// one-way hash in storage and is deliberately absent from this representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpAccess {
    pub identifier: SmtpAccessIdentifier,
    /// Descriptive administrator label, such as "Laravel Development".
    pub name: String,
    /// Unique SMTP AUTH username.
    pub username: String,
    /// Disabled accesses are refused at authentication and at delivery.
    pub enabled: bool,
    /// Accept this access only on sessions protected by TLS (STARTTLS or implicit).
    pub requires_encryption: bool,
    /// The SMTP AUTH mechanisms this access may use, sorted and without duplicates; never empty.
    pub allowed_mechanisms: Vec<SmtpAuthenticationMechanism>,
    /// Unix seconds, UTC, when the access was created.
    pub created_at: i64,
    /// Unix seconds, UTC, of the most recent successful authentication, if any.
    pub last_used_at: Option<i64>,
}

/// An SMTP AUTH mechanism that transmits the password itself (RFC 4954).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpAuthenticationMechanism {
    /// `AUTH PLAIN` (RFC 4616).
    Plain,
    /// `AUTH LOGIN`, the widely deployed non-standard username and password prompts.
    Login,
}

impl SmtpAuthenticationMechanism {
    /// Every supported mechanism, in their canonical order.
    pub const ALL: [Self; 2] = [Self::Plain, Self::Login];
}
