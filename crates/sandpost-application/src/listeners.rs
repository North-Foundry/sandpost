//! Runtime control of the single global SMTP listener, implemented by the hosting runtime.
//!
//! The application persists the global SMTP server's listener address; a hosting runtime that owns
//! the real socket implements [`MailListenerControl`] so management edits bind, move, or stop the
//! listener without a restart. Applications without a runtime (tests, offline tools) have no
//! control and report no listener state.
use async_trait::async_trait;
use serde::Serialize;
use std::net::SocketAddr;

/// Where the address the listener uses came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MailListenerSource {
    /// The persisted server address, editable through management.
    Database,
    /// A process environment override, which management edits do not rebind.
    Environment,
}

/// Lifecycle state of the global SMTP listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MailListenerState {
    /// The listener is bound and accepting SMTP sessions.
    Listening,
    /// No address is configured, so the instance receives no SMTP traffic.
    Disabled,
    /// Binding or accepting failed; `error` explains why (for example, a port already in use).
    Failed,
}

/// How the global SMTP listener protects sessions in transit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MailListenerEncryption {
    /// No TLS; everything, including SMTP AUTH credentials, travels in clear text.
    None,
    /// STARTTLS is offered, but clients may also continue in clear text.
    StartTlsOptional,
    /// Clients must upgrade with STARTTLS before AUTH or mail.
    StartTlsRequired,
    /// Every connection starts with a TLS handshake.
    Implicit,
}

/// Observable status of the global SMTP listener.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MailListenerStatus {
    /// The address the runtime tries to bind, after environment overrides.
    pub configured_address: Option<SocketAddr>,
    /// The actual bound address, including a system-allocated port.
    pub bound_address: Option<SocketAddr>,
    pub source: MailListenerSource,
    pub state: MailListenerState,
    pub error: Option<String>,
    /// Whether sessions must authenticate with an SMTP access before submitting mail.
    ///
    /// The runtime decides this for the configured address: by default loopback listeners accept
    /// unauthenticated local mail, while any other address requires authentication.
    pub authentication_required: bool,
    /// How sessions on the configured address are protected in transit.
    pub encryption: MailListenerEncryption,
}

impl MailListenerStatus {
    /// Describe a server whose persisted definition has no listener address.
    pub fn disabled() -> Self {
        Self {
            configured_address: None,
            bound_address: None,
            source: MailListenerSource::Database,
            state: MailListenerState::Disabled,
            error: None,
            authentication_required: false,
            encryption: MailListenerEncryption::None,
        }
    }
}

/// Bind, move, and stop the global SMTP listener for its persisted address.
#[async_trait]
pub trait MailListenerControl: Send + Sync {
    /// Reconcile the listener with the persisted server address and return the new status.
    ///
    /// `None` stops the listener. A failed bind is reported in the returned status rather than as
    /// an error; an environment-managed listener keeps its override and is left unchanged.
    async fn apply(&self, address: Option<SocketAddr>) -> MailListenerStatus;

    /// Report the current status of the listener.
    async fn status(&self) -> MailListenerStatus;
}
