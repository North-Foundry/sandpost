//! Delivery and connection tests against real local SMTP servers.
//!
//! Sandpost's own capture server (`sandpost-smtp`) plays a real receiving server with plain,
//! STARTTLS and implicit TLS sessions and AUTH; a small scripted server injects per-recipient
//! refusals, failed data, dropped connections and silence. Nothing leaves the loopback interface.
use sandpost_core::{
    Message, OutboundFailureKind, SmtpOutboundAuthentication, SmtpOutboundSecurity,
};
use sandpost_smtp::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    SessionHandler, SmtpServer, TlsConfiguration, TransportSecurity,
};
use sandpost_smtp_outbound::{
    DestinationPolicy, OutboundConnection, OutboundCredentials, OutboundEnvelope, RecipientOutcome,
    deliver, prepare_message, test_connection,
};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Handler that accepts one credential pair and records every delivered message.
struct RecordingHandler(Arc<Mutex<Vec<Message>>>);

impl SessionHandler for RecordingHandler {
    type Principal = String;

    /**
     * Accept only `application` / `secret`.
     */
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

    /**
     * Record the delivered message.
     */
    fn deliver(
        &self,
        message: Message,
        _principal: Option<String>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        self.0.lock().unwrap().push(message);
        std::future::ready(Ok(()))
    }
}

/// A running capture server and the messages it received.
struct Capture {
    address: SocketAddr,
    messages: Arc<Mutex<Vec<Message>>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for Capture {
    /**
     * Stop the capture server when the test ends.
     */
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

/**
 * Generate a self-signed certificate for `localhost` as a capture TLS configuration.
 */
fn certificate() -> TlsConfiguration {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    TlsConfiguration::from_pem(
        certified.cert.pem().as_bytes(),
        certified.signing_key.serialize_pem().as_bytes(),
    )
    .unwrap()
}

/**
 * Start a capture server on a loopback port with the given TLS and AUTH policies.
 */
async fn start_capture(
    security: TransportSecurity,
    authentication: AuthenticationPolicy,
) -> Capture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let (shutdown, shutdown_receiver) = oneshot::channel::<()>();
    tokio::spawn(
        SmtpServer::new(RecordingHandler(Arc::clone(&messages)))
            .authentication(authentication)
            .transport_security(security)
            .serve(listener, async {
                let _ = shutdown_receiver.await;
            }),
    );
    Capture {
        address,
        messages,
        shutdown: Some(shutdown),
    }
}

/**
 * Describe a connection to a loopback port with short timeouts.
 */
fn connection(
    host: &str,
    port: u16,
    security: SmtpOutboundSecurity,
    credentials: Option<(&str, &str, SmtpOutboundAuthentication)>,
    verify_certificates: bool,
) -> OutboundConnection {
    OutboundConnection {
        host: host.into(),
        port,
        security,
        credentials: credentials.map(|(username, password, mechanism)| OutboundCredentials {
            mechanism,
            username: username.into(),
            password: password.into(),
        }),
        verify_certificates,
        helo_name: "relay.sandpost.test".into(),
        connect_timeout: Duration::from_secs(5),
        command_timeout: Duration::from_secs(5),
    }
}

/**
 * Permit loopback destinations, as tests use local servers.
 */
fn local() -> DestinationPolicy {
    DestinationPolicy {
        allow_internal_addresses: true,
    }
}

/**
 * Build an envelope from a sender and recipients.
 */
fn envelope(sender: Option<&str>, recipients: &[&str]) -> OutboundEnvelope {
    OutboundEnvelope {
        sender: sender.map(str::to_owned),
        recipients: recipients
            .iter()
            .map(|recipient| (*recipient).to_owned())
            .collect(),
    }
}

/// A message as Sandpost captured it, with the capture's trace fields and no Bcc header.
const CAPTURED: &[u8] = b"Return-Path: <app@example.test>\r\nReceived: from app by sandpost.localhost with ESMTP\r\nFrom: App <app@example.test>\r\nTo: First <first@example.test>\r\nSubject: Relayed\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHello\r\n.dotted line\r\n--b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"a.bin\"\r\nContent-Transfer-Encoding: base64\r\n\r\nAAECAw==\r\n--b--\r\n";

/**
 * Return the received raw message without the trace fields the receiving capture prepended.
 */
fn without_new_trace(raw: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.split_inclusive("\r\n").peekable();
    let mut kept = String::new();
    // The receiving capture prepends one Return-Path and one (possibly folded) Received field.
    let mut skipped = 0;
    while let Some(line) = lines.peek() {
        let starts_field = !line.starts_with(' ') && !line.starts_with('\t');
        if starts_field {
            if skipped == 2 {
                break;
            }
            skipped += 1;
        }
        lines.next();
    }
    for line in lines {
        kept.push_str(line);
    }
    kept.into_bytes()
}

/**
 * Plain SMTP relays the original MIME, attachments and Bcc semantics to every envelope recipient.
 */
#[tokio::test]
async fn plain_delivery_preserves_content_and_every_envelope_recipient() {
    let capture = start_capture(TransportSecurity::Plaintext, AuthenticationPolicy::Optional).await;
    let message = prepare_message(CAPTURED, None);
    let report = deliver(
        &connection(
            "127.0.0.1",
            capture.address.port(),
            SmtpOutboundSecurity::None,
            None,
            true,
        ),
        &local(),
        &envelope(
            Some("app@example.test"),
            &["first@example.test", "hidden-bcc@example.test"],
        ),
        &message,
    )
    .await;
    assert_eq!(report.error, None);
    assert_eq!(report.recipients.len(), 2);
    for (_, outcome) in &report.recipients {
        assert!(
            matches!(outcome, RecipientOutcome::Delivered { response } if response.starts_with("250")),
            "{outcome:?}"
        );
    }
    let messages = capture.messages.lock().unwrap();
    assert_eq!(messages.len(), 1);
    let received = &messages[0];
    assert_eq!(
        received
            .facts
            .envelope_to
            .iter()
            .map(|mailbox| mailbox.address.as_str())
            .collect::<Vec<_>>(),
        ["first@example.test", "hidden-bcc@example.test"]
    );
    assert_eq!(
        received
            .facts
            .envelope_from
            .as_ref()
            .map(|mailbox| mailbox.address.as_str()),
        Some("app@example.test")
    );
    assert_eq!(without_new_trace(&received.raw_message), message);
    assert!(!String::from_utf8_lossy(&received.raw_message).contains("hidden-bcc"));
    assert_eq!(received.attachments.len(), 1);
}

/**
 * STARTTLS and implicit TLS sessions authenticate with PLAIN and LOGIN and deliver.
 */
#[tokio::test]
async fn encrypted_sessions_authenticate_and_deliver() {
    for (transport, security, mechanism) in [
        (
            TransportSecurity::StartTls {
                configuration: certificate(),
                required: true,
            },
            SmtpOutboundSecurity::StartTls,
            SmtpOutboundAuthentication::Plain,
        ),
        (
            TransportSecurity::Implicit {
                configuration: certificate(),
            },
            SmtpOutboundSecurity::Tls,
            SmtpOutboundAuthentication::Login,
        ),
    ] {
        let capture = start_capture(transport, AuthenticationPolicy::Required).await;
        let target = connection(
            "localhost",
            capture.address.port(),
            security,
            Some(("application", "secret", mechanism)),
            false,
        );
        let report = deliver(
            &target,
            &local(),
            &envelope(None, &["first@example.test"]),
            &prepare_message(CAPTURED, Some("relay@example.test")),
        )
        .await;
        assert_eq!(report.error, None, "{security:?}");
        assert!(matches!(
            report.recipients[0].1,
            RecipientOutcome::Delivered { .. }
        ));
        let messages = capture.messages.lock().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].facts.from[0].address, "relay@example.test");
        assert!(
            messages[0].facts.envelope_from.is_none(),
            "null reverse path"
        );
        let headers = &messages[0].facts.headers;
        assert_eq!(
            headers["x-original-from"][0].trim(),
            "App <app@example.test>"
        );
        assert_eq!(
            headers["received"].len(),
            2,
            "the earlier trace field travels with the message"
        );
    }
}

/**
 * A self-signed certificate is refused while verification is on, which is the default.
 */
#[tokio::test]
async fn certificate_verification_refuses_untrusted_servers() {
    let capture = start_capture(
        TransportSecurity::Implicit {
            configuration: certificate(),
        },
        AuthenticationPolicy::Optional,
    )
    .await;
    let report = deliver(
        &connection(
            "localhost",
            capture.address.port(),
            SmtpOutboundSecurity::Tls,
            None,
            true,
        ),
        &local(),
        &envelope(Some("app@example.test"), &["first@example.test"]),
        &prepare_message(CAPTURED, None),
    )
    .await;
    let error = report.error.expect("the handshake fails");
    assert_eq!(error.kind, OutboundFailureKind::Tls, "{}", error.message);
    assert!(matches!(
        report.recipients[0].1,
        RecipientOutcome::Deferred(_)
    ));
    assert!(capture.messages.lock().unwrap().is_empty());
}

/**
 * Configured STARTTLS never falls back to clear text when the server does not offer it.
 */
#[tokio::test]
async fn starttls_is_never_skipped() {
    let capture = start_capture(TransportSecurity::Plaintext, AuthenticationPolicy::Optional).await;
    let target = connection(
        "127.0.0.1",
        capture.address.port(),
        SmtpOutboundSecurity::StartTls,
        Some(("application", "secret", SmtpOutboundAuthentication::Plain)),
        true,
    );
    let report = deliver(
        &target,
        &local(),
        &envelope(Some("app@example.test"), &["first@example.test"]),
        &prepare_message(CAPTURED, None),
    )
    .await;
    let error = report.error.expect("STARTTLS is missing");
    assert_eq!(error.kind, OutboundFailureKind::Tls);
    assert!(error.message.contains("STARTTLS"), "{}", error.message);
    assert!(capture.messages.lock().unwrap().is_empty());
    let tested = test_connection(&target, &local()).await;
    assert!(!tested.success);
    assert!(!tested.encrypted);
}

/**
 * Rejected credentials are a permanent authentication failure that names the reply code.
 */
#[tokio::test]
async fn rejected_credentials_fail_authentication() {
    let capture = start_capture(TransportSecurity::Plaintext, AuthenticationPolicy::Required).await;
    let report = deliver(
        &connection(
            "127.0.0.1",
            capture.address.port(),
            SmtpOutboundSecurity::None,
            Some(("application", "wrong", SmtpOutboundAuthentication::Plain)),
            true,
        ),
        &local(),
        &envelope(Some("app@example.test"), &["first@example.test"]),
        &prepare_message(CAPTURED, None),
    )
    .await;
    let error = report.error.expect("authentication fails");
    assert_eq!(error.kind, OutboundFailureKind::Authentication);
    assert_eq!(error.code, Some(535));
    assert!(matches!(
        report.recipients[0].1,
        RecipientOutcome::Failed(_)
    ));
}

/**
 * Connection tests negotiate TLS and authenticate without starting a mail transaction.
 */
#[tokio::test]
async fn connection_tests_deliver_nothing() {
    let capture = start_capture(
        TransportSecurity::StartTls {
            configuration: certificate(),
            required: true,
        },
        AuthenticationPolicy::Required,
    )
    .await;
    let report = test_connection(
        &connection(
            "localhost",
            capture.address.port(),
            SmtpOutboundSecurity::StartTls,
            Some(("application", "secret", SmtpOutboundAuthentication::Login)),
            false,
        ),
        &local(),
    )
    .await;
    assert!(report.success, "{:?}", report.error);
    assert!(report.encrypted);
    assert!(report.authenticated);
    assert!(report.extensions.iter().any(|name| name == "AUTH LOGIN"));
    assert!(capture.messages.lock().unwrap().is_empty());
}

/**
 * Internal destinations are refused before connecting unless the policy allows them.
 */
#[tokio::test]
async fn internal_destinations_are_refused_by_default() {
    let capture = start_capture(TransportSecurity::Plaintext, AuthenticationPolicy::Optional).await;
    let target = connection(
        "127.0.0.1",
        capture.address.port(),
        SmtpOutboundSecurity::None,
        None,
        true,
    );
    let report = test_connection(&target, &DestinationPolicy::default()).await;
    let error = report.error.expect("the destination is refused");
    assert_eq!(error.kind, OutboundFailureKind::Policy);
    assert!(report.address.is_none());
    let refused = deliver(
        &target,
        &DestinationPolicy::default(),
        &envelope(Some("app@example.test"), &["first@example.test"]),
        &prepare_message(CAPTURED, None),
    )
    .await;
    assert!(matches!(
        refused.recipients[0].1,
        RecipientOutcome::Failed(_)
    ));
    assert!(capture.messages.lock().unwrap().is_empty());
}

/**
 * A closed port is a transient connection failure.
 */
#[tokio::test]
async fn refused_connections_are_transient() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let report = deliver(
        &connection("127.0.0.1", port, SmtpOutboundSecurity::None, None, true),
        &local(),
        &envelope(Some("app@example.test"), &["first@example.test"]),
        b"Subject: x\r\n\r\nbody\r\n",
    )
    .await;
    assert_eq!(
        report.error.map(|error| error.kind),
        Some(OutboundFailureKind::Connection)
    );
    assert!(matches!(
        report.recipients[0].1,
        RecipientOutcome::Deferred(_)
    ));
}

/// How the scripted server answers.
#[derive(Clone)]
struct Script {
    /// Lines of the EHLO reply after the first one.
    extensions: Vec<&'static str>,
    /// Reply to MAIL FROM; `None` never answers.
    mail: Option<&'static str>,
    /// Replies to RCPT TO by address; others get `250 ok`.
    recipients: HashMap<&'static str, &'static str>,
    /// Reply after the message data; `None` closes the connection instead.
    data: Option<&'static str>,
}

impl Default for Script {
    /**
     * Accept everything and announce 8BITMIME.
     */
    fn default() -> Self {
        Self {
            extensions: vec!["8BITMIME"],
            mail: Some("250 2.1.0 sender ok"),
            recipients: HashMap::new(),
            data: Some("250 2.0.0 queued as 42"),
        }
    }
}

/// What the scripted server observed.
#[derive(Default)]
struct Transcript {
    commands: Vec<String>,
    data: Vec<String>,
}

/**
 * Serve one scripted SMTP session on a loopback port and record what the client sent.
 */
async fn scripted(script: Script) -> (u16, Arc<Mutex<Transcript>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let transcript = Arc::new(Mutex::new(Transcript::default()));
    let recorded = Arc::clone(&transcript);
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        stream
            .get_mut()
            .write_all(b"220 scripted ready\r\n")
            .await
            .unwrap();
        loop {
            let mut line = String::new();
            if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let command = line.trim_end().to_owned();
            recorded.lock().unwrap().commands.push(command.clone());
            let upper = command.to_ascii_uppercase();
            let reply = if upper.starts_with("EHLO") {
                let mut reply = String::from("250");
                for extension in &script.extensions {
                    reply.push_str("-scripted\r\n250 ");
                    reply.push_str(extension);
                }
                if script.extensions.is_empty() {
                    reply.push_str(" scripted");
                }
                reply
            } else if upper.starts_with("MAIL") {
                match script.mail {
                    Some(reply) => reply.to_owned(),
                    None => {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        return;
                    }
                }
            } else if upper.starts_with("RCPT") {
                let address = command
                    .split_once('<')
                    .and_then(|(_, rest)| rest.split_once('>'))
                    .map(|(address, _)| address)
                    .unwrap_or_default();
                script
                    .recipients
                    .get(address)
                    .copied()
                    .unwrap_or("250 2.1.5 ok")
                    .to_owned()
            } else if upper == "DATA" {
                stream
                    .get_mut()
                    .write_all(b"354 go ahead\r\n")
                    .await
                    .unwrap();
                loop {
                    let mut data_line = String::new();
                    stream.read_line(&mut data_line).await.unwrap();
                    if data_line == ".\r\n" {
                        break;
                    }
                    recorded.lock().unwrap().data.push(data_line);
                }
                match script.data {
                    Some(reply) => reply.to_owned(),
                    None => return,
                }
            } else if upper == "QUIT" {
                let _ = stream.get_mut().write_all(b"221 bye\r\n").await;
                return;
            } else {
                "250 ok".to_owned()
            };
            stream
                .get_mut()
                .write_all(format!("{reply}\r\n").as_bytes())
                .await
                .unwrap();
        }
    });
    (port, transcript)
}

/**
 * Each recipient keeps its own outcome and only accepted recipients receive the data.
 */
#[tokio::test]
async fn recipients_are_accepted_refused_or_deferred_individually() {
    let (port, transcript) = scripted(Script {
        recipients: HashMap::from([
            ("gone@example.test", "550 5.1.1 no such user"),
            ("busy@example.test", "450 4.2.1 mailbox busy"),
        ]),
        ..Script::default()
    })
    .await;
    let report = deliver(
        &connection("127.0.0.1", port, SmtpOutboundSecurity::None, None, true),
        &local(),
        &envelope(
            Some("app@example.test"),
            &["ok@example.test", "gone@example.test", "busy@example.test"],
        ),
        b"Subject: x\r\n\r\n.starts with a dot\r\n",
    )
    .await;
    assert_eq!(report.error, None);
    assert!(matches!(
        &report.recipients[0].1,
        RecipientOutcome::Delivered { response } if response == "250 2.0.0 queued as 42"
    ));
    match &report.recipients[1].1 {
        RecipientOutcome::Failed(failure) => {
            assert_eq!(failure.kind, OutboundFailureKind::Permanent);
            assert_eq!(failure.code, Some(550));
        }
        other => panic!("unexpected {other:?}"),
    }
    match &report.recipients[2].1 {
        RecipientOutcome::Deferred(failure) => {
            assert_eq!(failure.kind, OutboundFailureKind::Temporary);
            assert_eq!(failure.code, Some(450));
        }
        other => panic!("unexpected {other:?}"),
    }
    let transcript = transcript.lock().unwrap();
    assert_eq!(
        transcript.data,
        ["Subject: x\r\n", "\r\n", "..starts with a dot\r\n"],
        "the message is dot-stuffed and its final CRLF is not doubled"
    );
    assert!(
        transcript
            .commands
            .iter()
            .any(|command| command == "MAIL FROM:<app@example.test>")
    );
}

/**
 * A data reply decides every accepted recipient: 4xx defers, 5xx fails.
 */
#[tokio::test]
async fn data_replies_decide_accepted_recipients() {
    for (reply, kind) in [
        ("451 4.3.0 try later", OutboundFailureKind::Temporary),
        ("554 5.6.0 content rejected", OutboundFailureKind::Permanent),
    ] {
        let (port, _) = scripted(Script {
            data: Some(reply),
            ..Script::default()
        })
        .await;
        let report = deliver(
            &connection("127.0.0.1", port, SmtpOutboundSecurity::None, None, true),
            &local(),
            &envelope(
                Some("app@example.test"),
                &["a@example.test", "b@example.test"],
            ),
            b"Subject: x\r\n\r\nbody\r\n",
        )
        .await;
        assert_eq!(report.error.as_ref().map(|error| error.kind), Some(kind));
        for (_, outcome) in &report.recipients {
            match (outcome, kind) {
                (RecipientOutcome::Deferred(_), OutboundFailureKind::Temporary)
                | (RecipientOutcome::Failed(_), OutboundFailureKind::Permanent) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}

/**
 * A connection lost after the data was sent is deferred, accepting a possible duplicate.
 */
#[tokio::test]
async fn lost_connections_after_data_are_deferred() {
    let (port, transcript) = scripted(Script {
        data: None,
        ..Script::default()
    })
    .await;
    let report = deliver(
        &connection("127.0.0.1", port, SmtpOutboundSecurity::None, None, true),
        &local(),
        &envelope(Some("app@example.test"), &["a@example.test"]),
        b"Subject: x\r\n\r\nbody\r\n",
    )
    .await;
    assert_eq!(
        report.error.map(|error| error.kind),
        Some(OutboundFailureKind::Connection)
    );
    assert!(matches!(
        report.recipients[0].1,
        RecipientOutcome::Deferred(_)
    ));
    assert!(!transcript.lock().unwrap().data.is_empty());
}

/**
 * A server that stops answering is bounded by the command timeout.
 */
#[tokio::test]
async fn silent_servers_time_out() {
    let (port, _) = scripted(Script {
        mail: None,
        ..Script::default()
    })
    .await;
    let mut target = connection("127.0.0.1", port, SmtpOutboundSecurity::None, None, true);
    target.command_timeout = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let report = deliver(
        &target,
        &local(),
        &envelope(Some("app@example.test"), &["a@example.test"]),
        b"Subject: x\r\n\r\nbody\r\n",
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(5));
    let error = report.error.expect("the command times out");
    assert_eq!(error.kind, OutboundFailureKind::Connection);
    assert!(
        error.message.contains("did not answer"),
        "{}",
        error.message
    );
}

/**
 * 8-bit content is refused permanently by a server that does not announce 8BITMIME.
 */
#[tokio::test]
async fn eight_bit_content_requires_eight_bit_mime() {
    let (port, transcript) = scripted(Script {
        extensions: Vec::new(),
        ..Script::default()
    })
    .await;
    let report = deliver(
        &connection("127.0.0.1", port, SmtpOutboundSecurity::None, None, true),
        &local(),
        &envelope(Some("app@example.test"), &["a@example.test"]),
        "Subject: caf\u{e9}\r\n\r\nbody\r\n".as_bytes(),
    )
    .await;
    assert_eq!(
        report.error.map(|error| error.kind),
        Some(OutboundFailureKind::Permanent)
    );
    assert!(
        !transcript
            .lock()
            .unwrap()
            .commands
            .iter()
            .any(|command| command.starts_with("MAIL"))
    );
}
