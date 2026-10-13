//! Shared harness for protocol tests: an in-memory backend, a validating client, and servers
//! bound to ephemeral loopback ports.
#![allow(dead_code)]

pub mod backend;
pub mod client;

use backend::MemoryBackend;
use client::TestClient;
use sandpost_imap::{ImapServer, Limits, TransportSecurity};
use std::net::SocketAddr;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

/// A running server and the means to stop it.
pub struct RunningServer {
    pub address: SocketAddr,
    pub backend: MemoryBackend,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl RunningServer {
    /// Serve a fresh in-memory backend in plaintext with the given limits.
    pub async fn start(limits: Limits) -> Self {
        Self::start_with(limits, TransportSecurity::Plaintext).await
    }

    /// Serve a fresh in-memory backend with the given limits and transport security.
    pub async fn start_with(limits: Limits, transport_security: TransportSecurity) -> Self {
        let backend = MemoryBackend::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let server = ImapServer::new(backend.clone())
            .transport_security(transport_security)
            .limits(limits);
        let task = tokio::spawn(server.serve(listener, async {
            let _ = shutdown_receiver.await;
        }));
        Self {
            address,
            backend,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    /// Connect a client and consume the greeting.
    pub async fn connect(&self) -> TestClient<TcpStream> {
        let stream = TcpStream::connect(self.address).await.unwrap();
        stream.set_nodelay(true).unwrap();
        let mut client = TestClient::new(stream);
        client.greeting().await;
        client
    }

    /// Connect, log in, and select a mailbox.
    pub async fn selected(&self, mailbox: &str) -> TestClient<TcpStream> {
        let mut client = self.connect().await;
        client.login().await;
        let completion = client
            .status_of("select", &format!("SELECT {mailbox}"))
            .await;
        assert!(completion.starts_with("select OK"), "{completion}");
        client
    }

    /// Stop accepting, drain sessions, and wait for the server task.
    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await.unwrap();
        }
    }
}

/// Fast limits for tests: no authentication delay and short timeouts.
pub fn test_limits() -> Limits {
    Limits {
        authentication_failure_delay: std::time::Duration::ZERO,
        change_coalescing_delay: std::time::Duration::from_millis(5),
        ..Limits::default()
    }
}

/// A small multipart message used across tests.
pub const MULTIPART_MESSAGE: &[u8] = b"Date: Tue, 9 Jan 2024 10:00:00 +0000\r\n\
From: \"Laravel App\" <app@laravel.test>\r\n\
To: Developer <developer@example.test>\r\n\
Subject: Password reset\r\n\
Message-ID: <reset-1@laravel.test>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Reset your password here.\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Reset your <b>password</b> here.</p>\r\n\
--b1--\r\n";

/// A plain message with a given subject.
pub fn plain_message(subject: &str) -> Vec<u8> {
    format!(
        "From: sender@example.test\r\nTo: developer@example.test\r\nSubject: {subject}\r\n\r\nBody of {subject}.\r\n"
    )
    .into_bytes()
}
