//! Stored verification results of the HTTP and HTTPS links found in captured messages.
use crate::MessageIdentifier;
use serde::{Deserialize, Serialize};

/// The verdict of verifying one link, shared by every user who can read its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkOutcome {
    /// The destination answered with a successful (2xx) response, possibly after redirects.
    Healthy,
    /// The destination answered with a client error (4xx) other than an access refusal.
    Broken,
    /// The destination answered with a server error (5xx).
    ServerError,
    /// No HTTP response was obtained: the name, connection, TLS handshake, or redirects failed.
    Failed,
    /// A response or policy prevented a verdict, such as an access refusal or a private address.
    Inconclusive,
}

impl LinkOutcome {
    /// Every outcome, in the order summaries present them.
    pub const ALL: [Self; 5] = [
        Self::Healthy,
        Self::Broken,
        Self::ServerError,
        Self::Failed,
        Self::Inconclusive,
    ];

    /**
     * Return the stable lowercase name used in storage and transport representations.
     */
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Broken => "broken",
            Self::ServerError => "server_error",
            Self::Failed => "failed",
            Self::Inconclusive => "inconclusive",
        }
    }

    /**
     * Parse a stable outcome name, rejecting unknown values.
     */
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.as_str() == value)
    }
}

/// Why a link check produced no usable HTTP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkCheckErrorKind {
    /// The host name did not resolve to any address.
    NameResolution,
    /// No connection could be established to any resolved address.
    Connection,
    /// The TLS handshake or certificate verification failed.
    Tls,
    /// The destination did not respond within the time limit.
    Timeout,
    /// The response was not valid HTTP.
    Protocol,
    /// The destination redirected more times than the limit allows.
    TooManyRedirects,
    /// The destination redirected back to a URL already visited.
    RedirectLoop,
    /// A redirect had no usable HTTP or HTTPS location.
    InvalidRedirect,
    /// The destination resolved to a private, local, or otherwise non-public address.
    BlockedAddress,
}

impl LinkCheckErrorKind {
    /// Every error kind, used to parse stored names.
    pub const ALL: [Self; 9] = [
        Self::NameResolution,
        Self::Connection,
        Self::Tls,
        Self::Timeout,
        Self::Protocol,
        Self::TooManyRedirects,
        Self::RedirectLoop,
        Self::InvalidRedirect,
        Self::BlockedAddress,
    ];

    /**
     * Return the stable lowercase name used in storage and transport representations.
     */
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NameResolution => "name_resolution",
            Self::Connection => "connection",
            Self::Tls => "tls",
            Self::Timeout => "timeout",
            Self::Protocol => "protocol",
            Self::TooManyRedirects => "too_many_redirects",
            Self::RedirectLoop => "redirect_loop",
            Self::InvalidRedirect => "invalid_redirect",
            Self::BlockedAddress => "blocked_address",
        }
    }

    /**
     * Parse a stable error kind name, rejecting unknown values.
     */
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// The stored result of verifying one distinct link of a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkCheckResult {
    /// The absolute HTTP or HTTPS URL as found in the message, without its fragment.
    pub url: String,
    pub outcome: LinkOutcome,
    /// Status code of the final response, absent when no response was received.
    pub status_code: Option<u16>,
    /// The URL that produced the final response, present only when redirects were followed.
    pub final_url: Option<String>,
    /// Number of redirects followed before the final response or failure.
    pub redirect_count: u32,
    pub error_kind: Option<LinkCheckErrorKind>,
    /// Short diagnostic detail for failures, never containing response bodies.
    pub error_message: Option<String>,
    /// Unix seconds, UTC, when this link's verification finished.
    pub checked_at: i64,
}

/// The most recent completed link check of one message, replaced as a whole by a recheck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageLinkCheck {
    pub message_identifier: MessageIdentifier,
    /// Unix seconds, UTC, when the whole check finished.
    pub checked_at: i64,
    /// Distinct links found beyond the per-message limit, which were not verified.
    pub omitted_link_count: u32,
    /// Results in the order their links first appear in the message.
    pub links: Vec<LinkCheckResult>,
}
