//! Read-only status of the single configuration-owned SMTP listener.
//!
//! The hosting runtime implements [`MailListenerControl`] to expose its configured and bound
//! addresses. Applications without a runtime report no listener state.
use async_trait::async_trait;
use serde::Serialize;
use std::net::SocketAddr;

/// Where the address the listener uses came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MailListenerSource {
    /// The resolved process configuration, including environment, TOML, and defaults.
    Configuration,
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
    /// The address the runtime tries to bind, after resolving environment, TOML, and defaults.
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
    /// Describe a runtime whose SMTP listener is stopped.
    pub fn disabled() -> Self {
        Self {
            configured_address: None,
            bound_address: None,
            source: MailListenerSource::Configuration,
            state: MailListenerState::Disabled,
            error: None,
            authentication_required: false,
            encryption: MailListenerEncryption::None,
        }
    }
}

/// Report the configuration-owned SMTP listener without allowing management edits.
#[async_trait]
pub trait MailListenerControl: Send + Sync {
    /// Report the current status of the listener.
    async fn status(&self) -> MailListenerStatus;
}
