//! The configurable IMAP server: connection acceptance, concurrency bounds, and shutdown.
use crate::{
    Limits, TransportSecurity,
    backend::ImapBackend,
    limits::{ACCEPT_ERROR_BACKOFF, REFUSAL_WRITE_TIMEOUT},
    session::{SessionConfiguration, run_session},
};
use std::{future::Future, io::ErrorKind, net::IpAddr, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, watch},
    task::JoinSet,
    time::{sleep, timeout},
};

/// One logical IMAP service that authenticates sessions through an [`ImapBackend`] and serves
/// every account from a single listener.
///
/// ```no_run
/// # use sandpost_imap::*;
/// # async fn run<B: ImapBackend>(backend: B) -> Result<(), Box<dyn std::error::Error>> {
/// let listener = tokio::net::TcpListener::bind("0.0.0.0:993").await?;
/// let certificate = TlsConfiguration::from_pem_files("imap.crt", "imap.key")?;
/// ImapServer::new(backend)
///     .transport_security(TransportSecurity::Implicit { configuration: certificate })
///     .serve(listener, async { let _ = tokio::signal::ctrl_c().await; })
///     .await;
/// # Ok(())
/// # }
/// ```
#[must_use = "a server does nothing until it is served"]
pub struct ImapServer<Backend> {
    configuration: SessionConfiguration<Backend>,
}

impl<Backend: ImapBackend> ImapServer<Backend> {
    /// Create a plaintext server with the default [`Limits`].
    pub fn new(backend: Backend) -> Self {
        Self {
            configuration: SessionConfiguration {
                backend,
                transport_security: TransportSecurity::Plaintext,
                limits: Limits::default(),
                greeting: "Sandpost IMAP".to_owned(),
            },
        }
    }

    /// Choose whether TLS is offered with STARTTLS, required, or used from the first byte.
    pub fn transport_security(mut self, transport_security: TransportSecurity) -> Self {
        self.configuration.transport_security = transport_security;
        self
    }

    /// Replace the default bounds on connections, commands, and time.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.configuration.limits = limits;
        self
    }

    /// Set the human-readable greeting text; non-printable characters are replaced.
    pub fn greeting(mut self, greeting: impl Into<String>) -> Self {
        self.configuration.greeting = greeting.into();
        self
    }

    /// Serve sessions from `listener` until `shutdown` completes, then drain them.
    ///
    /// On shutdown the port is released at once; sessions waiting for input receive `BYE` and
    /// close, and any session still running after [`Limits::shutdown_drain_timeout`] is
    /// aborted. Accept errors never stop the server.
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
        let capacity = Arc::new(Semaphore::new(
            limits.maximum_connection_count.min(Semaphore::MAX_PERMITS),
        ));
        let (stop_sessions, sessions_stopping) = watch::channel(false);
        let mut sessions = JoinSet::new();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                Some(_) = sessions.join_next(), if !sessions.is_empty() => {},
                accepted = source.accept_connection() => match accepted {
                    Ok((connection, peer_address)) => {
                        let Ok(permit) = Arc::clone(&capacity).try_acquire_owned() else {
                            if matches!(configuration.transport_security, TransportSecurity::Implicit { .. }) {
                                // IMAPS never sends protocol text before a TLS handshake.
                                drop(connection);
                            } else {
                                refuse_connection(connection).await;
                            }
                            continue;
                        };
                        let configuration = Arc::clone(&configuration);
                        let stopping = sessions_stopping.clone();
                        sessions.spawn(async move {
                            if let Err(error) = run_session(connection, configuration, peer_address, stopping, permit).await {
                                tracing::debug!(%error, "IMAP session ended");
                            }
                        });
                    }
                    Err(error) if is_connection_error(&error) => {
                        tracing::debug!(%error, "IMAP connection failed before its session started");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "IMAP accept failed; retrying");
                        tokio::select! {
                            _ = &mut shutdown => break,
                            _ = sleep(ACCEPT_ERROR_BACKOFF) => {}
                        }
                    }
                }
            }
        }
        drop(source);
        let _ = stop_sessions.send(true);
        if timeout(limits.shutdown_drain_timeout, async {
            while sessions.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            sessions.abort_all();
            while sessions.join_next().await.is_some() {}
        }
    }
}

/// A source of client connections: a TCP listener in production, scripted sources in tests.
pub(crate) trait ConnectionSource: Send {
    /// The bidirectional stream of one accepted connection.
    type Connection: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    /// Wait for the next connection and its peer address, or an accept error.
    fn accept_connection(
        &mut self,
    ) -> impl Future<Output = std::io::Result<(Self::Connection, Option<IpAddr>)>> + Send;
}

impl ConnectionSource for TcpListener {
    type Connection = TcpStream;

    /// Accept one TCP connection and read its peer address from the socket.
    async fn accept_connection(&mut self) -> std::io::Result<(TcpStream, Option<IpAddr>)> {
        let (stream, address) = self.accept().await?;
        let _ = stream.set_nodelay(true);
        Ok((stream, Some(address.ip())))
    }
}

/// Tell a connection over the concurrency limit to retry later, then close it.
async fn refuse_connection<Connection: AsyncWrite + Unpin>(mut connection: Connection) {
    let _ = timeout(REFUSAL_WRITE_TIMEOUT, async {
        connection
            .write_all(b"* BYE Too many connections, try again later\r\n")
            .await?;
        connection.shutdown().await
    })
    .await;
}

/// Report whether an accept error concerns only the connection being accepted.
fn is_connection_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::Interrupted
    )
}
