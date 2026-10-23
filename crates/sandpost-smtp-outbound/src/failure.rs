//! Classification of SMTP client errors into structured outbound failures.
use sandpost_core::{OutboundFailure, OutboundFailureKind};
use std::time::Duration;

/// The SMTP session phase in which an error occurred, which decides how it is classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Connecting, the greeting and `EHLO`.
    Connect,
    /// STARTTLS negotiation.
    Encrypt,
    /// `AUTH`.
    Authenticate,
    /// `MAIL FROM` and `RCPT TO`.
    Envelope,
    /// `DATA` and the message content.
    Data,
}

impl Phase {
    /**
     * Describe the phase for human-readable error messages.
     */
    fn description(self) -> &'static str {
        match self {
            Self::Connect => "connecting",
            Self::Encrypt => "negotiating STARTTLS",
            Self::Authenticate => "authenticating",
            Self::Envelope => "sending the envelope",
            Self::Data => "sending the message",
        }
    }
}

/**
 * Build a failure of a kind with a message and no SMTP code.
 */
pub(crate) fn failure(kind: OutboundFailureKind, message: impl Into<String>) -> OutboundFailure {
    OutboundFailure {
        kind,
        message: message.into(),
        code: None,
    }
}

/**
 * Classify an SMTP client error raised during a phase.
 *
 * Negative replies keep their code and text: 4xx is temporary and 5xx permanent, except while
 * authenticating, where both are authentication failures (a 4xx one stays retryable). Transport
 * and parse errors are connection failures, TLS errors are TLS failures, and client-side refusals
 * (such as no usable AUTH mechanism) take the kind of their phase.
 */
pub(crate) fn classify(error: &lettre::transport::smtp::Error, phase: Phase) -> OutboundFailure {
    let code = error.status().map(u16::from);
    let message = error.to_string();
    let kind = if code.is_some() {
        if phase == Phase::Authenticate {
            OutboundFailureKind::Authentication
        } else if error.is_transient() {
            OutboundFailureKind::Temporary
        } else {
            OutboundFailureKind::Permanent
        }
    } else if error.is_tls() || caused_by_tls(error) {
        OutboundFailureKind::Tls
    } else if error.is_client() {
        match phase {
            Phase::Authenticate => OutboundFailureKind::Authentication,
            Phase::Encrypt => OutboundFailureKind::Tls,
            Phase::Connect => OutboundFailureKind::Connection,
            Phase::Envelope | Phase::Data => OutboundFailureKind::Permanent,
        }
    } else {
        OutboundFailureKind::Connection
    };
    OutboundFailure {
        kind,
        message: format!("{} failed while {}", message, phase.description()),
        code,
    }
}

/**
 * Report whether a TLS handshake or certificate error caused `error`, even when the client
 * reported it as a connection error (as it does for implicit TLS).
 */
fn caused_by_tls(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = error.source();
    while let Some(cause) = source {
        if cause.is::<tokio_rustls::rustls::Error>() {
            return true;
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>()
            && io
                .get_ref()
                .is_some_and(|inner| inner.is::<tokio_rustls::rustls::Error>())
        {
            return true;
        }
        source = cause.source();
    }
    false
}

/**
 * Build the failure reported when a phase exceeded its time limit.
 */
pub(crate) fn timeout(phase: Phase, limit: Duration) -> OutboundFailure {
    failure(
        OutboundFailureKind::Connection,
        format!(
            "the server did not answer within {} s while {}",
            limit.as_secs(),
            phase.description()
        ),
    )
}

/**
 * Report whether a failure may succeed when attempted again later.
 *
 * Connection, TLS and temporary SMTP failures, interruptions, and authentication failures with a
 * 4xx reply are transient; permanent replies, policy refusals, configuration problems and other
 * authentication failures are not.
 */
pub fn is_transient(failure: &OutboundFailure) -> bool {
    match failure.kind {
        OutboundFailureKind::Connection
        | OutboundFailureKind::Tls
        | OutboundFailureKind::Temporary
        | OutboundFailureKind::Interrupted => true,
        OutboundFailureKind::Authentication => {
            failure.code.is_some_and(|code| (400..500).contains(&code))
        }
        OutboundFailureKind::Configuration
        | OutboundFailureKind::Policy
        | OutboundFailureKind::Permanent => false,
    }
}
