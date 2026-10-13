//! Bounds that keep one client from exhausting the server.
use std::time::Duration;

/// Configurable bounds for an [`ImapServer`](crate::ImapServer); the defaults suit a local
/// development server. Override only what you need with struct update syntax.
///
/// ```
/// use sandpost_imap::Limits;
/// use std::time::Duration;
///
/// let limits = Limits {
///     maximum_connection_count: 16,
///     ..Limits::default()
/// };
/// assert_eq!(limits.authenticated_timeout, Duration::from_secs(30 * 60));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most sessions served at once; further connections receive `BYE`. Defaults to 64.
    pub maximum_connection_count: usize,
    /// Largest command including its literals; larger commands are refused. Defaults to 64 KiB.
    pub maximum_command_size: usize,
    /// Longest single command line excluding literals. Defaults to 8 KiB.
    pub maximum_line_size: usize,
    /// Failed authentications after which the session is closed. Defaults to 3.
    pub maximum_authentication_failures: usize,
    /// Pause before answering a failed authentication, slowing password guessing. Defaults to
    /// two seconds.
    pub authentication_failure_delay: Duration,
    /// Longest wait for a command before authentication. RFC 9051 section 5.4 places no lower
    /// bound on this timer. Defaults to 60 seconds.
    pub unauthenticated_timeout: Duration,
    /// Inactivity autologout after authentication; RFC 3501 section 5.4 requires at least 30
    /// minutes. Defaults to 30 minutes.
    pub authenticated_timeout: Duration,
    /// Longest IDLE before the server ends the session (RFC 2177 asks clients to re-issue IDLE
    /// within 29 minutes). Defaults to 31 minutes.
    pub idle_timeout: Duration,
    /// Interval of `* OK` keepalives during IDLE, so idle connections survive NAT and proxies.
    /// Defaults to two minutes.
    pub idle_keepalive_interval: Duration,
    /// Pause after a change notification before synchronizing, coalescing bursts of changes.
    /// Defaults to 50 milliseconds.
    pub change_coalescing_delay: Duration,
    /// Longest wait for one network write or literal read. Defaults to 60 seconds.
    pub input_output_timeout: Duration,
    /// Longest wait for one backend call. Defaults to 60 seconds.
    pub backend_timeout: Duration,
    /// Longest wait for a TLS handshake. Defaults to 30 seconds.
    pub tls_handshake_timeout: Duration,
    /// How long sessions may finish after shutdown before they are aborted. Defaults to 5
    /// seconds.
    pub shutdown_drain_timeout: Duration,
}

impl Default for Limits {
    /// The bounds of a local development server.
    fn default() -> Self {
        Self {
            maximum_connection_count: 64,
            maximum_command_size: 64 * 1024,
            maximum_line_size: 8 * 1024,
            maximum_authentication_failures: 3,
            authentication_failure_delay: Duration::from_secs(2),
            unauthenticated_timeout: Duration::from_secs(60),
            authenticated_timeout: Duration::from_secs(30 * 60),
            idle_timeout: Duration::from_secs(31 * 60),
            idle_keepalive_interval: Duration::from_secs(2 * 60),
            change_coalescing_delay: Duration::from_millis(50),
            input_output_timeout: Duration::from_secs(60),
            backend_timeout: Duration::from_secs(60),
            tls_handshake_timeout: Duration::from_secs(30),
            shutdown_drain_timeout: Duration::from_secs(5),
        }
    }
}

/// Largest non-synchronizing literal under LITERAL- (RFC 7888 section 5).
pub(crate) const MAXIMUM_NON_SYNCHRONIZING_LITERAL: usize = 4096;
/// Longest SASL response line accepted during AUTHENTICATE.
pub(crate) const MAXIMUM_AUTHENTICATION_LINE_SIZE: usize = 12 * 1024;
/// Consecutive commands answered with BAD after which the connection is closed.
pub(crate) const MAXIMUM_CONSECUTIVE_BAD_COMMANDS: usize = 20;
/// Messages fetched per backend metadata call.
pub(crate) const METADATA_BATCH_SIZE: usize = 256;
/// Response bytes buffered before they are written to the client.
pub(crate) const OUTPUT_FLUSH_THRESHOLD: usize = 64 * 1024;
/// Pause after an accept error that is not about a single connection.
pub(crate) const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_secs(1);
/// Longest wait while telling a connection over the limit that the server is busy.
pub(crate) const REFUSAL_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
