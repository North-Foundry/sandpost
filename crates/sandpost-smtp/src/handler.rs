//! The contract between the SMTP protocol and the application that authenticates and stores mail.
//!
//! The protocol layer only frames credentials and messages. Deciding whether credentials are
//! valid, which principal they identify, and where accepted mail is stored belongs to the
//! application, which implements [`SessionHandler`].
use sandpost_core::{Message, SmtpAuthenticationMechanism};
use std::future::Future;
use thiserror::Error;

/// Credentials presented through SMTP AUTH, with how they were presented.
///
/// UTF-8 strings are passed without Unicode preparation; the handler chooses preparation and
/// comparison rules compatible with its authentication database.
/// Intentionally lacks Debug and Display so passwords are never formatted into logs.
pub struct Credentials {
    pub username: String,
    pub password: String,
    /// The mechanism the client used.
    pub mechanism: SmtpAuthenticationMechanism,
    /// Whether the session was protected by TLS when the credentials arrived.
    pub encrypted: bool,
}

/// The handler's verdict on presented credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthenticationOutcome<Principal> {
    /// The credentials are valid and allowed on this session; it is bound to the principal.
    Authenticated(Principal),
    /// Unknown, wrong, or disabled credentials; `535`, counted toward the failure limit.
    InvalidCredentials,
    /// Valid credentials that may only be used on a TLS-protected session; `538`.
    EncryptionRequired,
    /// Valid credentials that may not use the presented mechanism; `534`.
    MechanismNotAllowed,
}

/// Whether a session must authenticate before it may submit mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationPolicy {
    /// `MAIL FROM` is refused with `530` until the session authenticates.
    Required,
    /// AUTH is offered, but unauthenticated sessions may also submit mail.
    Optional,
}

/// Why presented credentials could not be checked; invalid credentials are not an error.
#[derive(Debug, Error)]
pub enum AuthenticationError {
    /// Verification failed for a transient reason such as unavailable storage; reported as `454`
    /// so the client may retry, and not counted as a failed attempt.
    #[error("credentials could not be verified: {0}")]
    Unavailable(String),
}

/// Why a parsed message was not accepted for delivery.
#[derive(Debug, Error)]
pub enum DeliveryError {
    /// The submission is refused permanently, for example because its credentials were revoked
    /// after authentication; reported as `554` with the reason.
    #[error("message rejected: {0}")]
    Rejected(String),
    /// A transient failure such as unavailable storage; reported as `451` so the client retries.
    #[error("message could not be stored: {0}")]
    Temporary(String),
}

/// Authenticates SMTP sessions and persists the messages they submit.
pub trait SessionHandler: Send + Sync + 'static {
    /// The application identity bound to an authenticated session.
    type Principal: Clone + Send + Sync + 'static;

    /// Verify presented credentials and decide whether this session may use them.
    ///
    /// Returns an error only when a temporary failure prevented verification.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<Self::Principal>, AuthenticationError>> + Send;

    /// Persist one accepted message submitted by the session's principal, if any.
    ///
    /// The message includes the server's Received and final Return-Path fields. The server
    /// acknowledges it only after this returns `Ok(())`. Cancellation cannot undo side effects
    /// already performed; a client retry may therefore repeat a completed write.
    fn deliver(
        &self,
        message: Message,
        principal: Option<Self::Principal>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send;
}
