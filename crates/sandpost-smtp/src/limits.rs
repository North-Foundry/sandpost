//! Bounds that keep one client from exhausting the server.
use std::time::Duration;

/// Configurable bounds for an [`SmtpServer`](crate::SmtpServer); the defaults suit a development
/// catcher.
///
/// Override only what you need with struct update syntax:
///
/// ```
/// use sandpost_smtp::Limits;
/// use std::time::Duration;
///
/// let limits = Limits {
///     maximum_message_size: 25 * 1024 * 1024,
///     maximum_session_duration: Duration::from_secs(60 * 60),
///     ..Limits::default()
/// };
/// assert_eq!(limits.maximum_connection_count, 32);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest message accepted after dot-unstuffing; larger DATA receives `552` and the
    /// session closes. Defaults to 10 MiB.
    pub maximum_message_size: usize,
    /// Most recipients in one transaction; further `RCPT TO` receives `452`. Defaults to 100.
    pub maximum_recipient_count: usize,
    /// Most sessions served at once; further connections receive `421`. Defaults to 32.
    pub maximum_connection_count: usize,
    /// Failed AUTH attempts after which the session is closed with `421`. Defaults to 3.
    pub maximum_authentication_failures: usize,
    /// Longest wait for one network read or write. Defaults to five minutes, including the
    /// minimum idle-command timeout recommended by RFC 5321 section 4.5.3.2.7.
    pub input_output_timeout: Duration,
    /// Longest wait for the handler to verify credentials or persist a message. Defaults to
    /// 60 seconds.
    pub handler_timeout: Duration,
    /// Longest wait for a TLS handshake, implicit or after STARTTLS; a client that does not
    /// finish in time is disconnected. Defaults to 30 seconds.
    pub tls_handshake_timeout: Duration,
    /// Longest lifetime of one session, so a slow client cannot hold a connection slot forever;
    /// the client then receives `421`. Defaults to 15 minutes.
    pub maximum_session_duration: Duration,
    /// How long active sessions may finish after shutdown before they are aborted. Defaults to
    /// 5 seconds.
    pub shutdown_drain_timeout: Duration,
}

impl Default for Limits {
    /// The bounds of a local development catcher.
    fn default() -> Self {
        Self {
            maximum_message_size: sandpost_mime::DEFAULT_SIZE_LIMIT,
            maximum_recipient_count: 100,
            maximum_connection_count: 32,
            maximum_authentication_failures: 3,
            input_output_timeout: Duration::from_secs(5 * 60),
            handler_timeout: Duration::from_secs(60),
            tls_handshake_timeout: Duration::from_secs(30),
            maximum_session_duration: Duration::from_secs(15 * 60),
            shutdown_drain_timeout: Duration::from_secs(5),
        }
    }
}

/// Longest unstuffed DATA line, including CRLF, under RFC 5321 section 4.5.3.1.6.
pub(crate) const MAXIMUM_LINE_SIZE: usize = 1000;
/// Longest ordinary command including CRLF, under RFC 5321 section 4.5.3.1.4.
pub(crate) const MAXIMUM_COMMAND_LINE_SIZE: usize = 512;
/// MAIL adds the SIZE allowance (RFC 1870, 26 bytes) and AUTH allowance (RFC 4954, 500 bytes).
pub(crate) const MAXIMUM_EXTENDED_MAIL_LINE_SIZE: usize = MAXIMUM_COMMAND_LINE_SIZE + 26 + 500;
/// AUTH continuations have an independent buffer under RFC 4954 section 4.
pub(crate) const MAXIMUM_AUTHENTICATION_LINE_SIZE: usize = 12288;
/// Longest reply including CRLF, under RFC 5321 section 4.5.3.1.5.
pub(crate) const MAXIMUM_REPLY_LINE_SIZE: usize = 512;
/// Pause after an accept error that is not about a single connection, such as running out of
/// file descriptors, before accepting again.
pub(crate) const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_secs(1);
/// Longest wait while telling a connection over the limit that the server is busy.
pub(crate) const REFUSAL_WRITE_TIMEOUT: Duration = Duration::from_secs(1);

/// Describe a byte size for a reply: whole mebibytes when exact, otherwise bytes.
pub(crate) fn describe_size(bytes: usize) -> String {
    const MEBIBYTE: usize = 1024 * 1024;
    if bytes >= MEBIBYTE && bytes.is_multiple_of(MEBIBYTE) {
        format!("{} MiB", bytes / MEBIBYTE)
    } else {
        format!("{bytes} bytes")
    }
}
