//! Smtp storage types.
use sandpost_core::{SmtpAccess, SmtpAuthenticationMechanism};

/// A new SMTP access supplied to the SMTP server storage creation operation.
///
/// Only the one-way password hash crosses the storage boundary; the plaintext password never
/// does. Intentionally lacks Debug so credential material is never logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct NewSmtpAccess {
    pub name: String,
    pub username: String,
    pub password_hash: String,
    /// Unix seconds, UTC, recorded as the creation time.
    pub created_at: i64,
    /// Accept the access only on TLS-protected sessions.
    pub requires_encryption: bool,
    /// The AUTH mechanisms the access may use; never empty.
    pub allowed_mechanisms: Vec<SmtpAuthenticationMechanism>,
}

/// An SMTP access together with its stored password hash, read only for authentication.
///
/// Intentionally lacks Debug so the hash is never logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpAccessCredential {
    pub access: SmtpAccess,
    pub password_hash: String,
}
