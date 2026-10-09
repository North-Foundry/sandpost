//! Accept-loop resilience, the connection limit, and shutdown, over a scripted connection source.
use crate::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    Limits, SessionHandler, SmtpServer,
    limits::ACCEPT_ERROR_BACKOFF,
    server::{ConnectionSource, is_connection_error},
};
use sandpost_core::Message;
use std::{
    future::Future,
    io::ErrorKind,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc as std_mpsc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream},
    sync::{mpsc, oneshot},
    time::timeout,
};

/// Handler that refuses every credential and accepts every message.
struct AcceptingHandler;

impl SessionHandler for AcceptingHandler {
    type Principal = ();

    /// Refuse every credential; these tests exercise only connection handling.
    fn authenticate(
        &self,
        _credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<()>, AuthenticationError>> + Send {
        std::future::ready(Ok(AuthenticationOutcome::InvalidCredentials))
    }

    /// Accept every message without storing it.
    fn deliver(
        &self,
        _message: Message,
        _principal: Option<()>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        std::future::ready(Ok(()))
    }
}

/// Count deliveries so cancellation can be checked independently of SMTP replies.
struct CountingHandler(Arc<AtomicUsize>);

impl SessionHandler for CountingHandler {
    type Principal = ();

    /// Refuse every credential; the regression test uses optional authentication.
    fn authenticate(
        &self,
        _credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<()>, AuthenticationError>> + Send {
        std::future::ready(Ok(AuthenticationOutcome::InvalidCredentials))
    }

    /// Record delivery calls so the test can prove expired DATA was not delivered.
    fn deliver(
        &self,
        _message: Message,
        _principal: Option<()>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(()))
    }
}

/// A connection source that yields whatever accept outcomes the test sends it.
struct ScriptedConnections(mpsc::UnboundedReceiver<std::io::Result<DuplexStream>>);

impl ConnectionSource for ScriptedConnections {
    type Connection = DuplexStream;

    /// Return the next scripted outcome, or wait forever once the script is exhausted.
    async fn accept_connection(&mut self) -> std::io::Result<DuplexStream> {
        match self.0.recv().await {
            Some(outcome) => outcome,
            None => std::future::pending().await,
        }
    }
}

/// A running scripted server plus the handles that drive and stop it.
struct ScriptedServer {
    connections: mpsc::UnboundedSender<std::io::Result<DuplexStream>>,
    shutdown: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

/// Release a blocking-pool gate when the test exits, including during unwinding.
struct BlockingReleaseGuard(Option<std_mpsc::Sender<()>>);

impl BlockingReleaseGuard {
    /// Release the blocked worker before checking that parser capacity recovers.
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for BlockingReleaseGuard {
    /// Prevent a blocked `spawn_blocking` task from hanging runtime destruction.
    fn drop(&mut self) {
        self.release();
    }
}

/// Start the accept loop over a scripted connection source.
fn start_scripted_server() -> ScriptedServer {
    let (connections, receiver) = mpsc::unbounded_channel();
    let (shutdown, shutdown_receiver) = oneshot::channel::<()>();
    let task = tokio::spawn(
        SmtpServer::new(AcceptingHandler)
            .authentication(AuthenticationPolicy::Optional)
            .serve_connections(ScriptedConnections(receiver), async {
                let _ = shutdown_receiver.await;
            }),
    );
    ScriptedServer {
        connections,
        shutdown,
        task,
    }
}

/// Hand the server a new in-memory connection and return the client side.
fn connect(server: &ScriptedServer) -> BufReader<DuplexStream> {
    let (client, connection) = tokio::io::duplex(4096);
    server.connections.send(Ok(connection)).unwrap();
    BufReader::new(client)
}

/// Read one reply line, failing the test if none arrives promptly.
async fn read_reply(client: &mut BufReader<DuplexStream>) -> String {
    let mut line = String::new();
    timeout(Duration::from_secs(5), client.read_line(&mut line))
        .await
        .expect("reply arrives")
        .unwrap();
    line
}

/// Accept errors of either kind leave the server running and later connections are served.
#[tokio::test(start_paused = true)]
async fn accept_errors_do_not_stop_the_server() {
    let server = start_scripted_server();
    server
        .connections
        .send(Err(std::io::Error::from(ErrorKind::ConnectionAborted)))
        .unwrap();
    server
        .connections
        .send(Err(std::io::Error::other("too many open files")))
        .unwrap();
    let started = tokio::time::Instant::now();
    let mut client = connect(&server);
    assert!(read_reply(&mut client).await.starts_with("220"));
    assert!(
        started.elapsed() >= ACCEPT_ERROR_BACKOFF,
        "a resource error pauses accepting before the next attempt"
    );
    assert!(!server.task.is_finished());
    server.shutdown.send(()).unwrap();
    drop(client);
    server.task.await.unwrap();
}

/// Shutdown is honoured while the server is pausing after an accept error.
#[tokio::test]
async fn shutdown_interrupts_the_accept_error_pause() {
    let server = start_scripted_server();
    server
        .connections
        .send(Err(std::io::Error::other("too many open files")))
        .unwrap();
    tokio::task::yield_now().await;
    server.shutdown.send(()).unwrap();
    timeout(ACCEPT_ERROR_BACKOFF / 2, server.task)
        .await
        .expect("shutdown does not wait for the pause")
        .unwrap();
}

/// A connection beyond the concurrency limit is told to retry later and closed.
#[tokio::test]
async fn connections_over_the_limit_receive_421() {
    let server = start_scripted_server();
    let mut admitted = Vec::new();
    for _ in 0..Limits::default().maximum_connection_count {
        let mut client = connect(&server);
        assert!(read_reply(&mut client).await.starts_with("220"));
        admitted.push(client);
    }
    let mut refused = connect(&server);
    assert!(read_reply(&mut refused).await.starts_with("421 4.3.2"));
    assert_eq!(
        read_reply(&mut refused).await,
        "",
        "the refused connection closes"
    );
    server.shutdown.send(()).unwrap();
    drop(admitted);
    server.task.await.unwrap();
}

/// Keep an expired DATA parser's connection lease until its queued blocking work can run.
#[test]
fn cancelled_session_retains_connection_capacity_until_queued_parser_finishes() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (release_sender, release_receiver) = std_mpsc::channel();
    let (started_sender, started_receiver) = std_mpsc::channel();
    let blocking_task = runtime.spawn_blocking(move || {
        started_sender.send(()).unwrap();
        release_receiver.recv().unwrap();
    });
    let mut blocking_release = BlockingReleaseGuard(Some(release_sender));
    started_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("the blocker occupies the only blocking worker");

    runtime.block_on(async {
        let delivery_count = Arc::new(AtomicUsize::new(0));
        let (connections, receiver) = mpsc::unbounded_channel();
        let (shutdown, shutdown_receiver) = oneshot::channel::<()>();
        let limits = Limits {
            maximum_connection_count: 1,
            maximum_session_duration: Duration::from_millis(250),
            shutdown_drain_timeout: Duration::from_secs(2),
            ..Limits::default()
        };
        let server_task = tokio::spawn(
            SmtpServer::new(CountingHandler(Arc::clone(&delivery_count)))
                .authentication(AuthenticationPolicy::Optional)
                .limits(limits)
                .serve_connections(ScriptedConnections(receiver), async {
                    let _ = shutdown_receiver.await;
                }),
        );
        let server = ScriptedServer {
            connections,
            shutdown,
            task: server_task,
        };

        let mut expired_client = connect(&server);
        assert!(read_reply(&mut expired_client).await.starts_with("220"));
        assert!(send_command(&mut expired_client, "EHLO local").await.starts_with("250"));
        assert!(
            send_command(&mut expired_client, "MAIL FROM:<sender@example.com>")
                .await
                .starts_with("250")
        );
        assert!(
            send_command(&mut expired_client, "RCPT TO:<recipient@example.com>")
                .await
                .starts_with("250")
        );
        assert!(send_command(&mut expired_client, "DATA").await.starts_with("354"));
        expired_client
            .get_mut()
            .write_all(b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: queued parser\r\n\r\nbody\r\n.\r\n")
            .await
            .unwrap();

        let expiry_reply = read_reply(&mut expired_client).await;
        assert!(expiry_reply.starts_with("421"), "got {expiry_reply:?}");
        let mut refused_client = connect(&server);
        let refusal_reply = read_reply(&mut refused_client).await;
        assert!(refusal_reply.starts_with("421 4.3.2"), "got {refusal_reply:?}");
        assert_eq!(delivery_count.load(Ordering::SeqCst), 0);

        blocking_release.release();
        timeout(Duration::from_secs(2), blocking_task)
            .await
            .expect("blocking-pool gate releases")
            .unwrap();

        let recovered_client = timeout(Duration::from_secs(2), async {
            loop {
                let mut client = connect(&server);
                let reply = read_reply(&mut client).await;
                if reply.starts_with("220") {
                    break client;
                }
                assert!(reply.starts_with("421 4.3.2"), "got {reply:?}");
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("parser completion returns connection capacity");
        assert_eq!(delivery_count.load(Ordering::SeqCst), 0);

        server.shutdown.send(()).unwrap();
        drop(expired_client);
        drop(refused_client);
        drop(recovered_client);
        server.task.await.unwrap();
    });
}

/// Send one SMTP command and consume its complete, potentially multiline reply.
async fn send_command(client: &mut BufReader<DuplexStream>, command: &str) -> String {
    client
        .get_mut()
        .write_all(format!("{command}\r\n").as_bytes())
        .await
        .unwrap();
    loop {
        let reply = read_reply(client).await;
        if reply.as_bytes().get(3) != Some(&b'-') {
            return reply;
        }
    }
}

/// Only per-connection failures are skipped without a pause.
#[test]
fn connection_errors_are_distinguished_from_listener_errors() {
    for kind in [
        ErrorKind::ConnectionRefused,
        ErrorKind::ConnectionAborted,
        ErrorKind::ConnectionReset,
        ErrorKind::Interrupted,
    ] {
        assert!(is_connection_error(&std::io::Error::from(kind)), "{kind:?}");
    }
    assert!(!is_connection_error(&std::io::Error::other(
        "too many open files"
    )));
    assert!(!is_connection_error(&std::io::Error::from(
        ErrorKind::PermissionDenied
    )));
}
