//! Test handlers and an in-memory client shared by the session tests.
use crate::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    Limits, SessionHandler, TransportSecurity,
    session::{SessionConfiguration, run_session},
};
use sandpost_core::Message;
use std::{future::Future, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream},
    sync::Semaphore,
};

/// Run a plaintext session for `handler` over an in-memory stream and return the client side.
pub(super) fn start_session<Handler: SessionHandler>(
    handler: Handler,
    policy: AuthenticationPolicy,
    limits: Limits,
) -> BufReader<DuplexStream> {
    start_secured_session(handler, policy, TransportSecurity::Plaintext, limits)
}

/// Run a session with the given transport security and return the raw client side.
pub(super) fn start_secured_session<Handler: SessionHandler>(
    handler: Handler,
    authentication: AuthenticationPolicy,
    transport_security: TransportSecurity,
    limits: Limits,
) -> BufReader<DuplexStream> {
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    tokio::spawn(run_session(
        server_stream,
        Arc::new(SessionConfiguration {
            handler,
            authentication,
            transport_security,
            limits,
            server_name: "sandpost.localhost".to_owned(),
        }),
        None,
        Arc::new(Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap()),
    ));
    BufReader::new(client_stream)
}

/// Start an optional-authentication session with custom limits whose deliveries go to `handler`.
pub(super) fn limited_test_client<Handler, HandlerFuture>(
    limits: Limits,
    handler: Handler,
) -> BufReader<DuplexStream>
where
    Handler: Fn(Message) -> HandlerFuture + Send + Sync + 'static,
    HandlerFuture: Future<Output = Result<(), std::io::Error>> + Send + 'static,
{
    start_session(
        ClosureHandler(move |message, _| handler(message)),
        AuthenticationPolicy::Optional,
        limits,
    )
}

/// The only username the test handler accepts.
pub(super) const TEST_USERNAME: &str = "application";

/// The only password the test handler accepts.
pub(super) const TEST_PASSWORD: &str = "correct horse battery";

/// Test handler that accepts one fixed credential and forwards deliveries to a closure.
pub(super) struct ClosureHandler<Delivery>(pub(super) Delivery);

impl<Delivery, DeliveryFuture> SessionHandler for ClosureHandler<Delivery>
where
    Delivery: Fn(Message, Option<String>) -> DeliveryFuture + Send + Sync + 'static,
    DeliveryFuture: Future<Output = Result<(), std::io::Error>> + Send + 'static,
{
    type Principal = String;

    /// Accept only the fixed test credential, identifying the session by its username.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<String>, AuthenticationError>> + Send
    {
        let valid = credentials.username == TEST_USERNAME && credentials.password == TEST_PASSWORD;
        async move {
            Ok(if valid {
                AuthenticationOutcome::Authenticated(credentials.username)
            } else {
                AuthenticationOutcome::InvalidCredentials
            })
        }
    }

    /// Forward the message and principal to the closure, mapping failures to temporary errors.
    fn deliver(
        &self,
        message: Message,
        principal: Option<String>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        let delivery = (self.0)(message, principal);
        async move {
            delivery
                .await
                .map_err(|error| DeliveryError::Temporary(error.to_string()))
        }
    }
}

/// Start a mail protocol session over an in-memory duplex stream for protocol tests.
pub(super) async fn mail_protocol_test_client<Handler, HandlerFuture>(
    handler: Handler,
) -> BufReader<DuplexStream>
where
    Handler: Fn(Message) -> HandlerFuture + Send + Sync + 'static,
    HandlerFuture: Future<Output = Result<(), std::io::Error>> + Send + 'static,
{
    authenticating_test_client(AuthenticationPolicy::Optional, move |message, _| {
        handler(message)
    })
    .await
}

/// Start a session with an explicit policy whose deliveries also receive the principal.
pub(super) async fn authenticating_test_client<Delivery, DeliveryFuture>(
    policy: AuthenticationPolicy,
    delivery: Delivery,
) -> BufReader<DuplexStream>
where
    Delivery: Fn(Message, Option<String>) -> DeliveryFuture + Send + Sync + 'static,
    DeliveryFuture: Future<Output = Result<(), std::io::Error>> + Send + 'static,
{
    start_session(ClosureHandler(delivery), policy, Limits::default())
}

/// Read one complete mail protocol reply and return its final line.
///
/// Multi-line replies such as the EHLO capability list mark continuation lines with a hyphen
/// after the status code; an empty string means the server closed the connection.
pub(super) async fn read_mail_protocol_response_line<Stream: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut BufReader<Stream>,
) -> String {
    loop {
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await.unwrap();
        if response_line.as_bytes().get(3) != Some(&b'-') {
            return response_line;
        }
    }
}

/// Send one mail protocol command and return its response line.
pub(super) async fn send_mail_protocol_command<Stream: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut BufReader<Stream>,
    command_text: &str,
) -> String {
    reader
        .get_mut()
        .write_all(format!("{command_text}\r\n").as_bytes())
        .await
        .unwrap();
    read_mail_protocol_response_line(reader).await
}

/// Encode test SASL responses as standard padded base64.
pub(super) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                encoded.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

/// Build a PLAIN initial response for an authorization identity, username, and password.
pub(super) fn plain_response(authorization: &str, username: &str, password: &str) -> String {
    base64_encode(format!("{authorization}\0{username}\0{password}").as_bytes())
}

/// Delivered message subjects paired with the principal of the session that sent them.
pub(super) type RecordedDeliveries =
    std::sync::Arc<std::sync::Mutex<Vec<(String, Option<String>)>>>;

/// Build a delivery closure that records each message's subject with the session principal.
pub(super) fn record_into(
    deliveries: RecordedDeliveries,
) -> impl Fn(Message, Option<String>) -> std::future::Ready<Result<(), std::io::Error>>
+ Send
+ Sync
+ 'static {
    move |message: Message, principal: Option<String>| {
        deliveries
            .lock()
            .unwrap()
            .push((message.facts.subject, principal));
        std::future::ready(Ok(()))
    }
}

/// Submit one complete transaction and return the final DATA reply.
pub(super) async fn submit_test_message<Stream: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut BufReader<Stream>,
    subject: &str,
) -> String {
    assert!(
        send_mail_protocol_command(reader, "MAIL FROM:<app@example.test>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(reader, "RCPT TO:<dest@example.test>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(reader, "DATA")
            .await
            .starts_with("354")
    );
    reader
        .get_mut()
        .write_all(format!("Subject: {subject}\r\n\r\nbody\r\n.\r\n").as_bytes())
        .await
        .unwrap();
    read_mail_protocol_response_line(reader).await
}

/// Read a complete multi-line reply, returning every line without its CRLF.
pub(super) async fn read_mail_protocol_reply_lines<Stream: AsyncRead + AsyncWrite + Unpin>(
    reader: &mut BufReader<Stream>,
) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await.unwrap();
        let continues = response_line.as_bytes().get(3) == Some(&b'-');
        lines.push(response_line.trim_end().to_owned());
        if !continues {
            return lines;
        }
    }
}

/// A self-signed server certificate and a client that trusts it, for TLS tests.
pub(super) struct TestCertificate {
    pub(super) configuration: crate::TlsConfiguration,
    pub(super) connector: tokio_rustls::TlsConnector,
    /// The PEM certificate, for configuration tests.
    pub(super) certificate_pem: String,
    /// The PEM private key, for configuration tests.
    pub(super) private_key_pem: String,
}

/// Generate a fresh self-signed certificate for `localhost`.
pub(super) fn test_certificate() -> TestCertificate {
    use tokio_rustls::rustls::{ClientConfig, RootCertStore, crypto::ring};
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate_pem = certified.cert.pem();
    let private_key_pem = certified.signing_key.serialize_pem();
    let configuration =
        crate::TlsConfiguration::from_pem(certificate_pem.as_bytes(), private_key_pem.as_bytes())
            .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(certified.cert.der().clone()).unwrap();
    let client = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TestCertificate {
        configuration,
        connector: tokio_rustls::TlsConnector::from(Arc::new(client)),
        certificate_pem,
        private_key_pem,
    }
}

/// Perform the client side of a TLS handshake for `localhost` over a test stream.
pub(super) async fn connect_with_tls(
    connector: &tokio_rustls::TlsConnector,
    stream: DuplexStream,
) -> BufReader<tokio_rustls::client::TlsStream<DuplexStream>> {
    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost").unwrap();
    BufReader::new(connector.connect(server_name, stream).await.unwrap())
}

/// The mechanism and TLS state of each authentication a [`RulesHandler`] received.
pub(super) type ReceivedAuthentications =
    Arc<std::sync::Mutex<Vec<(sandpost_core::SmtpAuthenticationMechanism, bool)>>>;

/// Handler with per-credential rules: `tls-only` needs TLS, `login-only` refuses PLAIN.
pub(super) struct RulesHandler(pub(super) ReceivedAuthentications);

impl SessionHandler for RulesHandler {
    type Principal = ();

    /// Record how the credentials arrived and apply the account's rule.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<()>, AuthenticationError>> + Send {
        use sandpost_core::SmtpAuthenticationMechanism;
        self.0
            .lock()
            .unwrap()
            .push((credentials.mechanism, credentials.encrypted));
        let outcome = match (credentials.username.as_str(), credentials.mechanism) {
            ("tls-only", _) if !credentials.encrypted => AuthenticationOutcome::EncryptionRequired,
            ("tls-only", _) => AuthenticationOutcome::Authenticated(()),
            ("login-only", SmtpAuthenticationMechanism::Plain) => {
                AuthenticationOutcome::MechanismNotAllowed
            }
            ("login-only", SmtpAuthenticationMechanism::Login) => {
                AuthenticationOutcome::Authenticated(())
            }
            _ => AuthenticationOutcome::InvalidCredentials,
        };
        std::future::ready(Ok(outcome))
    }

    /// Accept every message.
    fn deliver(
        &self,
        _message: Message,
        _principal: Option<()>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        std::future::ready(Ok(()))
    }
}
