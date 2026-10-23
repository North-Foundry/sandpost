//! Delivery of captured mail to external SMTP servers.
//!
//! This crate is the transport half of Sandpost's outbound delivery: it knows how to reach an
//! SMTP server and hand it a message, but nothing about scopes, routing actions or storage.
//!
//! - [`destination`] resolves the server's host name once and refuses non-public addresses
//!   unless explicitly allowed; the connection then uses only the validated addresses, so a DNS
//!   answer cannot redirect a later connection to an internal service.
//! - [`deliver`] opens one SMTP session (plain, STARTTLS that is never skipped, or implicit TLS),
//!   authenticates with `AUTH PLAIN` or `AUTH LOGIN` when configured, offers every recipient with
//!   its own `RCPT TO` so each one gets its own outcome, and sends the message only to the
//!   recipients the server accepted. Every phase is bounded by a timeout.
//! - [`test_connection`] performs the same connection, TLS negotiation and authentication and
//!   then quits without starting a mail transaction.
//! - [`prepare_message`] adapts the captured bytes for re-submission without changing their
//!   content: the capture's `Return-Path` is removed (a relay must not carry it), bare line feeds
//!   become CRLF, and the `From` field is replaced only when an override is configured.
//!
//! Failures are classified with [`sandpost_core::OutboundFailureKind`]. SMTP cannot guarantee
//! exactly-once delivery: when a connection is lost after the message data was sent, the server
//! may already have accepted it, and a retry can deliver a duplicate.
//!
//! The SMTP protocol itself is implemented by `lettre`'s low-level client connection; passwords
//! and authentication exchanges are never logged by this crate.

mod content;
mod delivery;
mod destination;
mod failure;
mod session;

pub use content::prepare_message;
pub use delivery::{
    ConnectionReport, DeliveryReport, OutboundEnvelope, RecipientOutcome, deliver, test_connection,
};
pub use destination::{DestinationPolicy, resolve};
pub use failure::is_transient;
pub use session::{DEFAULT_HELO_NAME, OutboundConnection, OutboundCredentials};
