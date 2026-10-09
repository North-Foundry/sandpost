//! The configurable SMTP server: connection acceptance, concurrency bounds, and shutdown.
use crate::{
    AuthenticationPolicy, Limits, SessionHandler, TransportSecurity,
    framing::write_reply,
    limits::{ACCEPT_ERROR_BACKOFF, REFUSAL_WRITE_TIMEOUT},
    session::{SessionConfiguration, run_session},
};
use std::{future::Future, io::ErrorKind, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::JoinSet,
    time::{sleep, timeout},
};

/// An SMTP receiver that authenticates sessions and hands accepted mail to a [`SessionHandler`].
///
/// Configure it fluently, then serve a bound listener until a shutdown signal:
///
/// ```no_run
/// # use sandpost_smtp::{AuthenticationError, Credentials, DeliveryError, SessionHandler};
/// # struct Capture;
/// # impl SessionHandler for Capture {
/// #     type Principal = String;
/// #     async fn authenticate(&self, credentials: Credentials) -> Result<sandpost_smtp::AuthenticationOutcome<String>, AuthenticationError> { Ok(sandpost_smtp::AuthenticationOutcome::Authenticated(credentials.username)) }
/// #     async fn deliver(&self, _message: sandpost_core::Message, _principal: Option<String>) -> Result<(), DeliveryError> { Ok(()) }
/// # }
/// use sandpost_smtp::{AuthenticationPolicy, Limits, SmtpServer, TlsConfiguration, TransportSecurity};
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let listener = tokio::net::TcpListener::bind("0.0.0.0:587").await?;
/// let certificate = TlsConfiguration::from_pem_files("smtp.crt", "smtp.key")?;
/// SmtpServer::new(Capture)
///     .authentication(AuthenticationPolicy::Required)
///     .transport_security(TransportSecurity::StartTls { configuration: certificate, required: true })
///     .limits(Limits { maximum_connection_count: 8, ..Limits::default() })
///     .serve(listener, async { let _ = tokio::signal::ctrl_c().await; })
///     .await;
/// # Ok(())
/// # }
/// ```
#[must_use = "a server does nothing until it is served"]
pub struct SmtpServer<Handler> {
    configuration: SessionConfiguration<Handler>,
}

impl<Handler: SessionHandler> SmtpServer<Handler> {
    /// Create a plaintext server that requires authentication and uses the default [`Limits`].
    pub fn new(handler: Handler) -> Self {
        Self {
            configuration: SessionConfiguration {
                handler,
                authentication: AuthenticationPolicy::Required,
                transport_security: TransportSecurity::Plaintext,
                limits: Limits::default(),
                server_name: "sandpost.localhost".to_owned(),
            },
        }
    }

    /// Choose whether sessions must authenticate before submitting mail.
    pub fn authentication(mut self, policy: AuthenticationPolicy) -> Self {
        self.configuration.authentication = policy;
        self
    }

    /// Choose whether TLS is offered with STARTTLS, required, or used from the first byte.
    pub fn transport_security(mut self, transport_security: TransportSecurity) -> Self {
        self.configuration.transport_security = transport_security;
        self
    }

    /// Set the domain or address literal used in greetings, trace headers and unqualified Postmaster.
    ///
    /// Defaults to `sandpost.localhost`. Invalid SMTP domains are rejected before serving.
    pub fn server_name(mut self, server_name: impl Into<String>) -> std::io::Result<Self> {
        let server_name = server_name.into();
        if !crate::session::mailbox::is_valid_domain(&server_name) {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "invalid SMTP server domain",
            ));
        }
        self.configuration.server_name = server_name.to_ascii_lowercase();
        Ok(self)
    }

    /// Replace the default bounds on messages, connections, and time.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.configuration.limits = limits;
        self
    }

    /// Serve sessions from `listener` until `shutdown` completes, then drain them.
    ///
    /// The port is released as soon as accepting stops, so the address can be rebound while
    /// admitted sessions finish within [`Limits::shutdown_drain_timeout`]. Accept errors never stop
    /// the server: a failure of the single connection being accepted is skipped, and any other
    /// error, such as running out of file descriptors, is logged and retried after a pause.
    pub async fn serve(self, listener: TcpListener, shutdown: impl Future<Output = ()>) {
        self.serve_connections(listener, shutdown).await;
    }

    /// Run the accept loop over any connection source, then drain admitted sessions.
    pub(crate) async fn serve_connections<Source: ConnectionSource>(
        self,
        mut source: Source,
        shutdown: impl Future<Output = ()>,
    ) {
        let configuration = Arc::new(self.configuration);
        let limits = configuration.limits;
        // Tokio's acquire_many uses u32; both bounds far exceed any practical connection count.
        let capacity_count = limits
            .maximum_connection_count
            .min(u32::MAX as usize)
            .min(Semaphore::MAX_PERMITS);
        let capacity = Arc::new(Semaphore::new(capacity_count));
        let mut sessions = JoinSet::new();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                Some(_) = sessions.join_next(), if !sessions.is_empty() => {},
                accepted = source.accept_connection() => match accepted {
                    Ok(connection) => {
                        let Ok(permit) = Arc::clone(&capacity).try_acquire_owned() else {
                            refuse_connection(connection).await;
                            continue;
                        };
                        let peer_address = Source::peer_address(&connection);
                        let configuration = Arc::clone(&configuration);
                        sessions.spawn(async move {
                            if let Err(error) = run_session(connection, configuration, peer_address, Arc::new(permit)).await {
                                tracing::debug!(%error, "SMTP session ended");
                            }
                        });
                    }
                    Err(error) if is_connection_error(&error) => {
                        tracing::debug!(%error, "SMTP connection failed before its session started");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "SMTP accept failed; retrying");
                        tokio::select! {
                            _ = &mut shutdown => break,
                            _ = sleep(ACCEPT_ERROR_BACKOFF) => {}
                        }
                    }
                }
            }
        }
        drop(source);
        if timeout(limits.shutdown_drain_timeout, async {
            while sessions.join_next().await.is_some() {}
            // Parser closures retain their session's admission permit even after async cancellation.
            let _all_connections = Arc::clone(&capacity)
                .acquire_many_owned(capacity_count as u32)
                .await;
        })
        .await
        .is_err()
        {
            sessions.abort_all();
            while sessions.join_next().await.is_some() {}
        }
    }
}

/// A source of client connections: a TCP listener in production, a scripted source in tests.
pub(crate) trait ConnectionSource: Send {
    /// The bidirectional stream of one accepted connection.
    type Connection: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    /// Return the actual peer IP for trace information when the source supplies one.
    fn peer_address(_connection: &Self::Connection) -> Option<std::net::IpAddr> {
        None
    }

    /// Wait for the next connection or accept error.
    fn accept_connection(
        &mut self,
    ) -> impl Future<Output = std::io::Result<Self::Connection>> + Send;
}

impl ConnectionSource for TcpListener {
    type Connection = TcpStream;

    /// Read the peer address from the accepted socket rather than trusting the client greeting.
    fn peer_address(connection: &TcpStream) -> Option<std::net::IpAddr> {
        connection.peer_addr().ok().map(|address| address.ip())
    }

    /// Accept one TCP connection, discarding the peer address.
    async fn accept_connection(&mut self) -> std::io::Result<TcpStream> {
        self.accept().await.map(|(stream, _)| stream)
    }
}

/// Tell a connection over the concurrency limit to retry later, then close it.
///
/// The write is bounded tightly because it runs in the accept loop; a fresh connection's send
/// buffer is empty, so the reply normally completes at once.
async fn refuse_connection<Connection: AsyncWrite + Unpin>(mut connection: Connection) {
    let _ = write_reply(
        &mut connection,
        "421 4.3.2 too many connections, try later",
        REFUSAL_WRITE_TIMEOUT,
    )
    .await;
}

/// Report whether an accept error concerns only the connection being accepted.
pub(crate) fn is_connection_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::Interrupted
    )
}
