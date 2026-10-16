//! Bind, move, and stop the single global SMTP listener while the runtime keeps running.
//!
//! The global server's listener address is persisted in storage. At startup the runtime binds it,
//! or an explicit `[smtp]`/`SANDPOST_SMTP_BIND_ADDRESS`/`SANDPOST_SMTP_PORT` override
//! instead. Afterwards management edits
//! reconcile the listener through [`MailListenerControl`]. A persisted address that cannot be
//! bound (for example, because the port is already in use) is reported as a failed listener
//! instead of stopping the runtime; an operator override keeps its fail-fast startup behavior
//! because an operator asked for it explicitly.
//!
//! Every bind evaluates the authentication and TLS policies for its address, so moving the
//! listener from loopback to a network interface starts requiring SMTP AUTH (and, with a
//! certificate, STARTTLS) immediately.
use crate::configuration::{
    Configuration, MailAuthenticationSetting, MailEncryptionMode, MailEncryptionSetting,
};
use crate::ingestion::MailDelivery;
use crate::lifecycle::RuntimeError;
use async_trait::async_trait;
use sandpost_application::{
    MailListenerControl, MailListenerEncryption, MailListenerSource, MailListenerState,
    MailListenerStatus,
};
use sandpost_smtp::{AuthenticationPolicy, SmtpServer, TlsConfiguration, TransportSecurity};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Mutex, watch},
    task::JoinHandle,
};

/// How long a rebind waits for a just-stopped listener of this runtime to release its port.
const REBIND_GRACE_PERIOD: Duration = Duration::from_secs(1);
/// Pause between rebind attempts during the grace period.
const REBIND_RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// The listener address the runtime should bind at startup and where it came from.
pub(crate) struct DesiredListener {
    pub address: SocketAddr,
    pub source: MailListenerSource,
}

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
    state: Mutex<ServerState>,
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

/// The current listener, plus stopped listeners that are still draining sessions.
#[derive(Default)]
struct ServerState {
    closed: bool,
    entry: Option<ListenerEntry>,
    draining: Vec<JoinHandle<()>>,
}

/// The desired address, origin, and current outcome of the listener.
struct ListenerEntry {
    configured_address: SocketAddr,
    source: MailListenerSource,
    running: Option<RunningListener>,
    error: Option<String>,
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

impl ListenerEntry {
    /// Report this entry through the application's listener status representation.
    fn status(
        &self,
        authentication: MailAuthenticationSetting,
        encryption: &MailEncryption,
    ) -> MailListenerStatus {
        MailListenerStatus {
            configured_address: Some(self.configured_address),
            bound_address: self.running.as_ref().map(|running| running.bound_address),
            source: self.source,
            state: if self.running.is_some() {
                MailListenerState::Listening
            } else {
                MailListenerState::Failed
            },
            error: self.error.clone(),
            authentication_required: authentication_policy(authentication, self.configured_address)
                == AuthenticationPolicy::Required,
            encryption: encryption.describe(self.configured_address),
        }
    }
}

impl ServerState {
    /// Signal a running listener to stop and keep its task so shutdown can await its drain.
    ///
    /// The task is not awaited here: it releases its port as soon as it observes the signal,
    /// while in-flight SMTP sessions finish in the background.
    fn retire(&mut self, running: RunningListener) {
        let _ = running.stop.send(true);
        self.draining.retain(|task| !task.is_finished());
        self.draining.push(running.task);
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
                state: Mutex::new(ServerState::default()),
            }),
        }
    }

    /// Bind the startup listener, failing only for an operator override that cannot bind.
    pub(crate) async fn start(&self, desired: Option<DesiredListener>) -> Result<(), RuntimeError> {
        let Some(listener) = desired else {
            return Ok(());
        };
        let mut state = self.shared.state.lock().await;
        let entry = match self.bind(listener.address, false).await {
            Ok(running) => ListenerEntry {
                configured_address: listener.address,
                source: listener.source,
                running: Some(running),
                error: None,
            },
            Err(error) if listener.source == MailListenerSource::Environment => {
                return Err(std::io::Error::new(
                    error.kind(),
                    format!("SMTP listener {}: {error}", listener.address),
                )
                .into());
            }
            Err(error) => {
                let message = bind_error_message(listener.address, &error);
                tracing::warn!(error = %message, "SMTP listener not started");
                ListenerEntry {
                    configured_address: listener.address,
                    source: listener.source,
                    running: None,
                    error: Some(message),
                }
            }
        };
        if authentication_policy(self.shared.authentication, listener.address)
            == AuthenticationPolicy::Optional
            && !listener.address.ip().to_canonical().is_loopback()
        {
            tracing::warn!(address = %listener.address, "SMTP accepts unauthenticated mail on a non-loopback address because SANDPOST_SMTP_AUTH=optional");
        }
        if self.shared.encryption.describe(listener.address) == MailListenerEncryption::None
            && !listener.address.ip().to_canonical().is_loopback()
        {
            tracing::warn!(address = %listener.address, "SMTP AUTH credentials travel unencrypted on a non-loopback address; set SANDPOST_SMTP_TLS_CERTIFICATE and SANDPOST_SMTP_TLS_PRIVATE_KEY");
        }
        state.entry = Some(entry);
        Ok(())
    }

    /// Report the listener's bound address, if it is listening.
    pub(crate) async fn bound_address(&self) -> Option<SocketAddr> {
        self.shared
            .state
            .lock()
            .await
            .entry
            .as_ref()
            .and_then(|entry| entry.running.as_ref())
            .map(|running| running.bound_address)
    }

    /// Stop accepting and wait for in-flight sessions to drain.
    ///
    /// Later reconciliation requests are refused, so no listener can start after shutdown.
    pub(crate) async fn shutdown(&self) {
        let tasks = {
            let mut state = self.shared.state.lock().await;
            state.closed = true;
            if let Some(running) = state.entry.as_mut().and_then(|entry| entry.running.take()) {
                state.retire(running);
            }
            std::mem::take(&mut state.draining)
        };
        for task in tasks {
            let _ = task.await;
        }
    }

    /// Bind an address and spawn its SMTP service, retrying briefly after a local stop.
    ///
    /// `after_local_stop` is set when this runtime just stopped its previous listener; that port
    /// is released asynchronously, so an "address in use" error is retried for a short grace
    /// period before it is reported.
    async fn bind(
        &self,
        address: SocketAddr,
        after_local_stop: bool,
    ) -> std::io::Result<RunningListener> {
        let deadline = tokio::time::Instant::now() + REBIND_GRACE_PERIOD;
        let listener = loop {
            match TcpListener::bind(address).await {
                Ok(listener) => break listener,
                Err(error)
                    if after_local_stop
                        && error.kind() == std::io::ErrorKind::AddrInUse
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(REBIND_RETRY_INTERVAL).await;
                }
                Err(error) => return Err(error),
            }
        };
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
    async fn apply(&self, address: Option<SocketAddr>) -> MailListenerStatus {
        let authentication = self.shared.authentication;
        let encryption = &self.shared.encryption;
        let mut state = self.shared.state.lock().await;
        if state.closed {
            return MailListenerStatus {
                error: Some("the SMTP server is shutting down".into()),
                state: MailListenerState::Failed,
                configured_address: address,
                ..MailListenerStatus::disabled()
            };
        }
        if let Some(entry) = &state.entry {
            // A TOML/environment override outranks persistence until the process restarts.
            if entry.source == MailListenerSource::Environment {
                return entry.status(authentication, encryption);
            }
            if Some(entry.configured_address) == address && entry.running.is_some() {
                return entry.status(authentication, encryption);
            }
        }
        let previous = state.entry.take().and_then(|entry| entry.running);
        let after_local_stop = previous.is_some();
        if let Some(previous) = previous {
            state.retire(previous);
        }
        let Some(address) = address else {
            return MailListenerStatus::disabled();
        };
        let entry = match self.bind(address, after_local_stop).await {
            Ok(running) => ListenerEntry {
                configured_address: address,
                source: MailListenerSource::Database,
                running: Some(running),
                error: None,
            },
            Err(error) => ListenerEntry {
                configured_address: address,
                source: MailListenerSource::Database,
                running: None,
                error: Some(bind_error_message(address, &error)),
            },
        };
        let status = entry.status(authentication, encryption);
        state.entry = Some(entry);
        status
    }

    async fn status(&self) -> MailListenerStatus {
        let state = self.shared.state.lock().await;
        state
            .entry
            .as_ref()
            .map(|entry| entry.status(self.shared.authentication, &self.shared.encryption))
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

/// Describe a bind failure in terms an administrator can act on.
fn bind_error_message(address: SocketAddr, error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::AddrInUse => format!("{address} is already in use by another process"),
        std::io::ErrorKind::AddrNotAvailable => {
            format!("{} is not an address of this machine", address.ip())
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("permission denied while binding {address}")
        }
        _ => format!("{address} could not be bound: {error}"),
    }
}

#[cfg(test)]
mod tests;
