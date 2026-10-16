//! Bind and stop the single configuration-owned SMTP listener.
//!
//! Authentication and TLS policies are evaluated for the configured address at startup and
//! apply again after a process restart with new settings.
use crate::configuration::{
    Configuration, MailAuthenticationSetting, MailEncryptionMode, MailEncryptionSetting,
};
use crate::lifecycle::RuntimeError;
use crate::smtp_delivery::MailDelivery;
use async_trait::async_trait;
use sandpost_application::{
    MailListenerControl, MailListenerEncryption, MailListenerSource, MailListenerState,
    MailListenerStatus,
};
use sandpost_smtp::{AuthenticationPolicy, SmtpServer, TlsConfiguration, TransportSecurity};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{Mutex, watch},
    task::JoinHandle,
};

/// Shared handle to the runtime's global SMTP listener.
#[derive(Clone)]
pub(crate) struct MailServer {
    shared: Arc<SharedServer>,
}

/// State shared by the handle and by the running listener task.
struct SharedServer {
    delivery: MailDelivery,
    authentication: MailAuthenticationSetting,
    encryption: MailEncryption,
    state: Mutex<Option<ListenerEntry>>,
}

/// The TLS settings every bind evaluates for its address.
#[derive(Clone)]
pub(crate) struct MailEncryption {
    setting: MailEncryptionSetting,
    mode: MailEncryptionMode,
    /// The loaded certificate; `None` when none is configured or TLS is disabled.
    configuration: Option<TlsConfiguration>,
}

impl MailEncryption {
    /// No TLS on any address.
    #[cfg(test)]
    pub(crate) fn plaintext() -> Self {
        Self {
            setting: MailEncryptionSetting::Disabled,
            mode: MailEncryptionMode::StartTls,
            configuration: None,
        }
    }

    /// Load the configured certificate, failing startup when it cannot be used.
    pub(crate) fn load(configuration: &Configuration) -> Result<Self, RuntimeError> {
        let certificate = match (
            &configuration.mail_certificate,
            configuration.mail_encryption,
        ) {
            (_, MailEncryptionSetting::Disabled) | (None, _) => None,
            (Some(paths), _) => Some(
                TlsConfiguration::from_pem_files(&paths.certificate_chain, &paths.private_key)
                    .map_err(|error| format!("SMTP TLS certificate: {error}"))?,
            ),
        };
        Ok(Self {
            setting: configuration.mail_encryption,
            mode: configuration.mail_encryption_mode,
            configuration: certificate,
        })
    }

    /// Decide how sessions on an address are protected.
    ///
    /// Without a certificate there is no TLS. Implicit mode encrypts every connection; STARTTLS is
    /// required when the setting says so, or automatically on any non-loopback address.
    pub(crate) fn transport_security(&self, address: SocketAddr) -> TransportSecurity {
        let Some(configuration) = self.configuration.clone() else {
            return TransportSecurity::Plaintext;
        };
        match self.mode {
            MailEncryptionMode::Implicit => TransportSecurity::Implicit { configuration },
            MailEncryptionMode::StartTls => TransportSecurity::StartTls {
                configuration,
                required: match self.setting {
                    MailEncryptionSetting::Required => true,
                    MailEncryptionSetting::Automatic => !address.ip().to_canonical().is_loopback(),
                    MailEncryptionSetting::Disabled => false,
                },
            },
        }
    }

    /// Describe the protection of an address for the listener status.
    pub(crate) fn describe(&self, address: SocketAddr) -> MailListenerEncryption {
        match self.transport_security(address) {
            TransportSecurity::Plaintext => MailListenerEncryption::None,
            TransportSecurity::StartTls {
                required: false, ..
            } => MailListenerEncryption::StartTlsOptional,
            TransportSecurity::StartTls { required: true, .. } => {
                MailListenerEncryption::StartTlsRequired
            }
            TransportSecurity::Implicit { .. } => MailListenerEncryption::Implicit,
        }
    }
}

/// The configured listener and its bound socket.
struct ListenerEntry {
    configured_address: SocketAddr,
    running: RunningListener,
}

/// A bound listener task.
struct RunningListener {
    bound_address: SocketAddr,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

/// Decide whether sessions on an address must authenticate.
///
/// The automatic setting keeps loopback listeners usable by local tools without credentials,
/// while any other address, including a wildcard, requires an SMTP access.
pub(crate) fn authentication_policy(
    setting: MailAuthenticationSetting,
    address: SocketAddr,
) -> AuthenticationPolicy {
    match setting {
        MailAuthenticationSetting::Required => AuthenticationPolicy::Required,
        MailAuthenticationSetting::Optional => AuthenticationPolicy::Optional,
        MailAuthenticationSetting::Automatic if address.ip().to_canonical().is_loopback() => {
            AuthenticationPolicy::Optional
        }
        MailAuthenticationSetting::Automatic => AuthenticationPolicy::Required,
    }
}

impl MailServer {
    /// Create a stopped server that authenticates and captures mail through `delivery`.
    pub(crate) fn new(
        delivery: MailDelivery,
        authentication: MailAuthenticationSetting,
        encryption: MailEncryption,
    ) -> Self {
        Self {
            shared: Arc::new(SharedServer {
                delivery,
                authentication,
                encryption,
                state: Mutex::new(None),
            }),
        }
    }

    /// Bind the configured startup listener and return its actual address; any bind failure
    /// fails startup.
    pub(crate) async fn start(&self, address: SocketAddr) -> Result<SocketAddr, RuntimeError> {
        let mut state = self.shared.state.lock().await;
        let running = self.bind(address).await.map_err(|error| {
            std::io::Error::new(error.kind(), format!("SMTP listener {address}: {error}"))
        })?;
        let bound_address = running.bound_address;
        if !address.ip().to_canonical().is_loopback() {
            if authentication_policy(self.shared.authentication, address)
                == AuthenticationPolicy::Optional
            {
                tracing::warn!(address = %address, "SMTP accepts unauthenticated mail on a non-loopback address because SANDPOST_SMTP_AUTH=optional");
            }
            if self.shared.encryption.describe(address) == MailListenerEncryption::None {
                tracing::warn!(address = %address, "SMTP AUTH credentials travel unencrypted on a non-loopback address; set SANDPOST_SMTP_TLS_CERTIFICATE and SANDPOST_SMTP_TLS_PRIVATE_KEY");
            }
        }
        *state = Some(ListenerEntry {
            configured_address: address,
            running,
        });
        Ok(bound_address)
    }

    /// Stop accepting and wait for in-flight sessions to drain.
    pub(crate) async fn shutdown(&self) {
        let entry = self.shared.state.lock().await.take();
        if let Some(entry) = entry {
            let _ = entry.running.stop.send(true);
            let _ = entry.running.task.await;
        }
    }

    /// Bind an address and spawn its SMTP service.
    async fn bind(&self, address: SocketAddr) -> std::io::Result<RunningListener> {
        let listener = TcpListener::bind(address).await?;
        let bound_address = listener.local_addr()?;
        let (stop, stop_receiver) = watch::channel(false);
        let server = SmtpServer::new(self.shared.delivery.clone())
            .authentication(authentication_policy(self.shared.authentication, address))
            .transport_security(self.shared.encryption.transport_security(address));
        Ok(RunningListener {
            bound_address,
            stop,
            task: tokio::spawn(serve_listener(server, listener, stop_receiver)),
        })
    }
}

#[async_trait]
impl MailListenerControl for MailServer {
    async fn status(&self) -> MailListenerStatus {
        let state = self.shared.state.lock().await;
        state
            .as_ref()
            .map(|entry| MailListenerStatus {
                configured_address: Some(entry.configured_address),
                bound_address: Some(entry.running.bound_address),
                source: MailListenerSource::Configuration,
                state: MailListenerState::Listening,
                error: None,
                authentication_required: authentication_policy(
                    self.shared.authentication,
                    entry.configured_address,
                ) == AuthenticationPolicy::Required,
                encryption: self.shared.encryption.describe(entry.configured_address),
            })
            .unwrap_or_else(MailListenerStatus::disabled)
    }
}

/// Serve SMTP sessions until the listener is stopped; accept errors are retried by the server.
async fn serve_listener(
    server: SmtpServer<MailDelivery>,
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
) {
    server
        .serve(listener, async move {
            while !*stop.borrow() {
                if stop.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
}

#[cfg(test)]
mod tests;
