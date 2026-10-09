//! STARTTLS over a real TCP socket through the public API.
use sandpost_core::Message;
use sandpost_smtp::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    SessionHandler, SmtpServer, TlsConfiguration, TransportSecurity,
};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, crypto::ring, pki_types::ServerName},
};

/// Handler that accepts one credential and records delivered messages.
struct RecordingHandler(Arc<Mutex<Vec<Message>>>);

impl SessionHandler for RecordingHandler {
    type Principal = String;

    /// Accept only `application` / `secret`.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<String>, AuthenticationError>> + Send
    {
        let valid = credentials.username == "application" && credentials.password == "secret";
        std::future::ready(Ok(if valid {
            AuthenticationOutcome::Authenticated(credentials.username)
        } else {
            AuthenticationOutcome::InvalidCredentials
        }))
    }

    /// Record every delivered message including its transport trace.
    fn deliver(
        &self,
        message: Message,
        _principal: Option<String>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        self.0.lock().unwrap().push(message);
        std::future::ready(Ok(()))
    }
}

/// Read one complete reply and return its final line.
async fn read_reply<Stream: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut BufReader<Stream>,
) -> String {
    loop {
        let mut line = String::new();
        timeout(Duration::from_secs(5), reader.read_line(&mut line))
            .await
            .expect("reply arrives")
            .unwrap();
        if line.as_bytes().get(3) != Some(&b'-') {
            return line;
        }
    }
}

/// Send one command and return the final line of its reply.
async fn command<Stream: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut BufReader<Stream>,
    text: &str,
) -> String {
    reader
        .get_mut()
        .write_all(format!("{text}\r\n").as_bytes())
        .await
        .unwrap();
    read_reply(reader).await
}

/// A client upgrades with STARTTLS, authenticates, and delivers mail over a real socket.
#[tokio::test]
async fn required_start_tls_over_tcp_delivers_authenticated_mail() {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let configuration = TlsConfiguration::from_pem(
        certified.cert.pem().as_bytes(),
        certified.signing_key.serialize_pem().as_bytes(),
    )
    .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(certified.cert.der().clone()).unwrap();
    let connector = TlsConnector::from(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let (shutdown, shutdown_receiver) = oneshot::channel::<()>();
    let server = tokio::spawn(
        SmtpServer::new(RecordingHandler(Arc::clone(&messages)))
            .server_name("Capture.Example.TEST")
            .unwrap()
            .authentication(AuthenticationPolicy::Required)
            .transport_security(TransportSecurity::StartTls {
                configuration,
                required: true,
            })
            .serve(listener, async {
                let _ = shutdown_receiver.await;
            }),
    );

    let mut plaintext = BufReader::new(TcpStream::connect(address).await.unwrap());
    assert_eq!(
        read_reply(&mut plaintext).await,
        "220 capture.example.test ESMTP ready\r\n"
    );
    assert_eq!(
        command(&mut plaintext, "EHLO client").await,
        "250 STARTTLS\r\n"
    );
    assert!(
        command(&mut plaintext, "AUTH PLAIN")
            .await
            .starts_with("538")
    );
    assert!(
        command(&mut plaintext, "STARTTLS")
            .await
            .starts_with("220 2.0.0")
    );
    let server_name = ServerName::try_from("localhost").unwrap();
    let mut encrypted = BufReader::new(
        connector
            .connect(server_name, plaintext.into_inner())
            .await
            .unwrap(),
    );
    assert_eq!(
        command(&mut encrypted, "EHLO client").await,
        "250 AUTH PLAIN LOGIN\r\n"
    );
    // "\0application\0secret" in base64.
    assert!(
        command(&mut encrypted, "AUTH PLAIN AGFwcGxpY2F0aW9uAHNlY3JldA==")
            .await
            .starts_with("235")
    );
    assert!(
        command(&mut encrypted, "MAIL FROM:<app@example.test> SIZE=64")
            .await
            .starts_with("250")
    );
    assert!(
        command(&mut encrypted, "RCPT TO:<Postmaster>")
            .await
            .starts_with("250")
    );
    assert!(command(&mut encrypted, "DATA").await.starts_with("354"));
    assert!(
        command(&mut encrypted, "Subject: encrypted\r\n\r\nbody\r\n.")
            .await
            .starts_with("250 2.0.0")
    );
    assert!(command(&mut encrypted, "QUIT").await.starts_with("221"));
    {
        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].facts.subject, "encrypted");
        assert_eq!(
            messages[0].facts.envelope_to[0].address,
            "Postmaster@capture.example.test"
        );
        let received = &messages[0].facts.headers["received"][0];
        assert!(
            received.starts_with("from client ([127.0.0.1]) "),
            "{received}"
        );
        assert!(
            received.contains("by capture.example.test with ESMTPSA;"),
            "{received}"
        );
    }

    shutdown.send(()).unwrap();
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}
