//! Receiver conformance scenarios through the public TCP API, without using server parsers.
use sandpost_core::Message;
use sandpost_smtp::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    Limits, SessionHandler, SmtpServer,
};
use std::{future::Future, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};

/// Accept one fixed credential and record only messages acknowledged by the receiver.
struct RecordingHandler(mpsc::UnboundedSender<Message>);

impl SessionHandler for RecordingHandler {
    type Principal = String;

    /// Authenticate the test account so reply and transaction state can be exercised together.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<String>, AuthenticationError>> + Send
    {
        std::future::ready(Ok(
            if credentials.username == "tester" && credentials.password == "secret" {
                AuthenticationOutcome::Authenticated(credentials.username)
            } else {
                AuthenticationOutcome::InvalidCredentials
            },
        ))
    }

    /// Record the complete message before returning SMTP delivery success.
    fn deliver(
        &self,
        message: Message,
        _principal: Option<String>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        std::future::ready(
            self.0
                .send(message)
                .map_err(|_| DeliveryError::Temporary("test receiver closed".into())),
        )
    }
}

/// An isolated server and a client which checks reply framing independently of the implementation.
struct ProtocolClient {
    reader: BufReader<TcpStream>,
    messages: mpsc::UnboundedReceiver<Message>,
    shutdown: oneshot::Sender<()>,
    server: JoinHandle<()>,
}

impl ProtocolClient {
    /// Bind an ephemeral loopback listener with an explicit client DATA allowance.
    async fn start(maximum_message_size: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (message_sender, messages) = mpsc::unbounded_channel();
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let server = tokio::spawn(
            SmtpServer::new(RecordingHandler(message_sender))
                .authentication(AuthenticationPolicy::Optional)
                .limits(Limits {
                    maximum_message_size,
                    ..Limits::default()
                })
                .serve(listener, async {
                    let _ = shutdown_receiver.await;
                }),
        );
        let stream = TcpStream::connect(address).await.unwrap();
        stream.set_nodelay(true).unwrap();
        let mut client = Self {
            reader: BufReader::new(stream),
            messages,
            shutdown,
            server,
        };
        assert_eq!(
            client.reply().await[0],
            "220 sandpost.localhost ESMTP ready\r\n"
        );
        client
    }

    /// Read a complete reply and independently validate RFC 5321 line bounds and multiline syntax.
    async fn reply(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            timeout(Duration::from_secs(3), self.reader.read_line(&mut line))
                .await
                .expect("reply must arrive without waiting for another command")
                .unwrap();
            assert!((6..=512).contains(&line.len()), "{line:?}");
            assert!(line.is_ascii() && line.ends_with("\r\n"), "{line:?}");
            assert!(
                line.as_bytes()[..3].iter().all(u8::is_ascii_digit),
                "{line:?}"
            );
            assert!(matches!(line.as_bytes()[3], b' ' | b'-'), "{line:?}");
            if let Some(first) = lines.first() {
                let first: &String = first;
                assert_eq!(&line[..3], &first[..3]);
            }
            let continued = line.as_bytes()[3] == b'-';
            lines.push(line);
            if !continued {
                return lines;
            }
        }
    }

    /// Send a single command and return its complete reply.
    async fn command(&mut self, command: &str) -> Vec<String> {
        self.reader
            .get_mut()
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .unwrap();
        self.reply().await
    }

    /// Negotiate and check the exact extension names without interpreting them through server code.
    async fn greet(&mut self) -> Vec<String> {
        let lines = self.command("EHLO client.example.test").await;
        assert_eq!(lines[0], "250-sandpost.localhost\r\n");
        assert!(lines.iter().any(|line| line == "250-PIPELINING\r\n"));
        assert!(lines.iter().any(|line| line == "250-8BITMIME\r\n"));
        assert!(
            lines
                .iter()
                .any(|line| line == "250-ENHANCEDSTATUSCODES\r\n")
        );
        assert_eq!(lines.last().unwrap(), "250 AUTH PLAIN LOGIN\r\n");
        lines
    }

    /// Stop accepting and await the server after its client has closed.
    async fn finish(self) {
        self.shutdown.send(()).unwrap();
        drop(self.reader);
        timeout(Duration::from_secs(3), self.server)
            .await
            .unwrap()
            .unwrap();
    }
}

/// Compare primary replies and validate the RFC 2034/3463 enhanced-code class and numeric grammar.
fn assert_status(lines: &[String], expected: &str) {
    for line in lines {
        assert_eq!(&line[..3], expected, "{lines:?}");
        if matches!(line.as_bytes()[0], b'2' | b'4' | b'5') {
            let enhanced = line[4..]
                .split_once(' ')
                .expect("enhanced code precedes reply text")
                .0;
            let components: Vec<_> = enhanced.split('.').collect();
            assert_eq!(components.len(), 3, "{line:?}");
            assert_eq!(components[0].as_bytes(), &line.as_bytes()[..1]);
            assert!(
                components[1..].iter().all(|component| {
                    (1..=3).contains(&component.len())
                        && component.bytes().all(|byte| byte.is_ascii_digit())
                        && (component.len() == 1 || !component.starts_with('0'))
                }),
                "{line:?}"
            );
        }
    }
}

/// Apply SMTP transparency on the client side, retaining CRLF and adding the terminating dot.
fn framed_data(source: &[u8]) -> Vec<u8> {
    assert!(source.ends_with(b"\r\n"));
    let mut framed = Vec::new();
    for line in source.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b".") {
            framed.push(b'.');
        }
        framed.extend_from_slice(line);
    }
    framed.extend_from_slice(b".\r\n");
    framed
}

/// RFC 2920 §3.2: fragmented grouped commands receive ordered replies, and DATA retains later input.
#[tokio::test]
async fn rfc2920_pipelining_preserves_order_and_fragmented_input() {
    for chunk_size in [1, 7, 512] {
        let mut client = ProtocolClient::start(4096).await;
        client.greet().await;
        let commands = b"MAIL FROM:<Sender@example.test>\r\nRCPT TO:<invalid>\r\nRCPT TO:<First@example.test>\r\nRCPT TO:<Second@example.test>\r\nDATA\r\n";
        for chunk in commands.chunks(chunk_size) {
            client.reader.get_mut().write_all(chunk).await.unwrap();
        }
        for expected in ["250", "501", "250", "250", "354"] {
            assert_status(&client.reply().await, expected);
        }
        let source = b"Subject: pipelined\r\n\r\n.first\r\n.\r\nlast\r\n";
        let mut submitted = framed_data(source);
        submitted.extend_from_slice(b"RSET\r\nNOOP\r\nFROBNICATE\r\nQUIT\r\n");
        client.reader.get_mut().write_all(&submitted).await.unwrap();
        for expected in ["250", "250", "250", "500", "221"] {
            assert_status(&client.reply().await, expected);
        }
        let message = client.messages.recv().await.unwrap();
        assert!(message.raw_message.ends_with(source));
        assert_eq!(message.facts.envelope_to.len(), 2);
        assert_eq!(message.facts.envelope_to[0].address, "First@example.test");
        assert_eq!(message.facts.envelope_to[1].address, "Second@example.test");
        assert!(client.messages.try_recv().is_err());
        client.finish().await;
    }
}

/// RFC 2920 §3.2: no valid recipient means no DATA acceptance or delivery; later commands remain intact.
#[tokio::test]
async fn rfc2920_rejected_recipients_do_not_deliver_and_recovery_keeps_input() {
    let mut client = ProtocolClient::start(4096).await;
    client.greet().await;
    client.reader.get_mut().write_all(b"MAIL FROM:<>\r\nRCPT TO:<invalid>\r\nDATA\r\nNOOP\r\nRSET\r\nMAIL FROM:<>\r\nRCPT TO:<Postmaster>\r\nDATA\r\n").await.unwrap();
    for expected in ["250", "501", "503", "250", "250", "250", "250", "354"] {
        assert_status(&client.reply().await, expected);
    }
    assert!(client.messages.try_recv().is_err());
    client
        .reader
        .get_mut()
        .write_all(&framed_data(b"Subject: recovered\r\n\r\nbody\r\n"))
        .await
        .unwrap();
    assert_status(&client.reply().await, "250");
    assert_eq!(
        client.messages.recv().await.unwrap().facts.subject,
        "recovered"
    );
    assert!(client.messages.try_recv().is_err());
    assert_status(&client.command("QUIT").await, "221");
    client.finish().await;
}

/// RFC 6152 §§2–3: BODY negotiation preserves eight-bit octets, stuffed dots and the last CRLF.
#[tokio::test]
async fn rfc6152_eight_bit_mime_preserves_octets_and_transparency() {
    let mut client = ProtocolClient::start(4096).await;
    client.greet().await;
    assert_status(
        &client.command("MAIL FROM:<> BODY=8BITMIME BODY=7BIT").await,
        "501",
    );
    assert_status(&client.command("MAIL FROM:<> BODY=BINARYMIME").await, "501");
    for body_parameter in ["8bitmime", "7BIT"] {
        assert_status(
            &client
                .command(&format!("MAIL FROM:<> BODY={body_parameter}"))
                .await,
            "250",
        );
        assert_status(&client.command("RCPT TO:<Postmaster>").await, "250");
        assert_status(&client.command("DATA").await, "354");
        let mut source = b"Subject: eight bit\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: 8bit\r\n\r\n.".to_vec();
        if body_parameter.eq_ignore_ascii_case("8BITMIME") {
            source.extend(128..=255);
        } else {
            source.extend_from_slice(b"seven-bit content");
        }
        source.extend_from_slice(b"\r\n.\r\n");
        client
            .reader
            .get_mut()
            .write_all(&framed_data(&source))
            .await
            .unwrap();
        assert_status(&client.reply().await, "250");
        assert!(
            client
                .messages
                .recv()
                .await
                .unwrap()
                .raw_message
                .ends_with(&source)
        );
    }
    assert_status(&client.command("QUIT").await, "221");
    client.finish().await;
}

/// RFC 1870 §§5–6: SIZE counts content octets, ignores quoting dots and still enforces actual DATA size.
#[tokio::test]
async fn rfc1870_size_counts_unstuffed_content_and_enforces_actual_limit() {
    let mut source = b"Subject: exact\r\n\r\n.".to_vec();
    source.resize(254, b'x');
    source.extend_from_slice(b"\r\n");
    let mut client = ProtocolClient::start(source.len()).await;
    let capabilities = client.greet().await;
    assert!(capabilities.iter().any(|line| line == "250-SIZE 256\r\n"));
    // SIZE is an estimate: this capture policy permits an underestimate within the actual ceiling.
    for declared_size in [source.len(), 1] {
        assert_status(
            &client
                .command(&format!("MAIL FROM:<> SIZE={declared_size}"))
                .await,
            "250",
        );
        assert_status(&client.command("RCPT TO:<Postmaster>").await, "250");
        assert_status(&client.command("DATA").await, "354");
        client
            .reader
            .get_mut()
            .write_all(&framed_data(&source))
            .await
            .unwrap();
        assert_status(&client.reply().await, "250");
        let message = client.messages.recv().await.unwrap();
        assert!(message.raw_message.ends_with(&source));
        assert!(
            message.raw_message.len() > source.len(),
            "server trace is outside the client budget"
        );
    }
    assert_status(&client.command("MAIL FROM:<> SIZE=1").await, "250");
    assert_status(&client.command("RCPT TO:<Postmaster>").await, "250");
    assert_status(&client.command("DATA").await, "354");
    source.insert(source.len() - 2, b'x');
    client
        .reader
        .get_mut()
        .write_all(&framed_data(&source))
        .await
        .unwrap();
    assert_status(&client.reply().await, "552");
    let mut remaining = String::new();
    assert_eq!(client.reader.read_line(&mut remaining).await.unwrap(), 0);
    assert!(client.messages.try_recv().is_err());
    client.finish().await;
}

/// RFC 1870 §3: zero capacity must not be announced as SIZE 0, which means no fixed upper bound.
#[tokio::test]
async fn rfc1870_zero_capacity_does_not_advertise_unlimited_size() {
    let mut client = ProtocolClient::start(0).await;
    let capabilities = client.greet().await;
    assert!(capabilities.iter().any(|line| line == "250-SIZE\r\n"));
    assert!(!capabilities.iter().any(|line| line == "250-SIZE 0\r\n"));
    assert_status(&client.command("MAIL FROM:<> SIZE=1").await, "552");
    assert_status(&client.command("QUIT").await, "221");
    client.finish().await;
}

/// RFC 2034 §§3–5 and RFC 3463 §2: enhanced codes accompany primary reply classes even without EHLO.
#[tokio::test]
async fn rfc2034_enhanced_status_codes_match_reply_classes_without_negotiation() {
    let mut client = ProtocolClient::start(4096).await;
    for (command, expected) in [
        ("MAIL FROM:<a@example.test>", "503"),
        ("HELO", "501"),
        ("RCPT TO:<a@example.test>", "503"),
        ("AUTH PLAIN", "503"),
        ("NOOP", "250"),
        ("HELP", "214"),
        ("VRFY Postmaster", "252"),
        ("EXPN group", "502"),
        ("UNKNOWN", "500"),
    ] {
        assert_status(&client.command(command).await, expected);
    }
    assert_eq!(
        client.command("HELO client.example.test").await,
        ["250 sandpost.localhost\r\n"]
    );
    assert_status(&client.command("MAIL FROM:<> SIZE=1").await, "555");
    client.greet().await;
    for (command, expected) in [
        ("MAIL FROM:<> SIZE=4097", "552"),
        ("MAIL FROM:<> UNKNOWN=value", "555"),
        ("MAIL FROM:<> SIZE=1 SIZE=2", "501"),
        ("AUTH PLAIN =", "501"),
        ("AUTH CRAM-MD5", "504"),
        ("AUTH PLAIN AHRlc3RlcgB3cm9uZw==", "535"),
        ("AUTH PLAIN AHRlc3RlcgBzZWNyZXQ=", "235"),
        ("AUTH PLAIN", "503"),
        ("MAIL FROM:<>", "250"),
        ("RCPT TO:<invalid>", "501"),
        ("RCPT TO:<Postmaster>", "250"),
        ("DATA", "354"),
    ] {
        assert_status(&client.command(command).await, expected);
    }
    client
        .reader
        .get_mut()
        .write_all(&framed_data(b"Subject: status\r\n\r\nbody\r\n"))
        .await
        .unwrap();
    assert_status(&client.reply().await, "250");
    assert_status(&client.command("QUIT").await, "221");
    client.finish().await;
}

/// RFC 5321 §4.1.4: RSET and either greeting abandon old recipients without ending the connection.
#[tokio::test]
async fn rfc5321_greetings_and_resets_discard_transactions() {
    let mut client = ProtocolClient::start(4096).await;
    client.greet().await;
    for reset in [
        "RSET",
        "HELO replacement.example.test",
        "EHLO replacement.example.test",
    ] {
        assert_status(&client.command("MAIL FROM:<old@example.test>").await, "250");
        assert_status(&client.command("RCPT TO:<old@example.test>").await, "250");
        let reply = client.command(reset).await;
        assert_eq!(&reply[0][..3], "250");
        assert_status(&client.command("DATA").await, "503");
        assert_status(&client.command("RCPT TO:<new@example.test>").await, "503");
        assert_status(&client.command("MAIL FROM:<new@example.test>").await, "250");
        assert_status(&client.command("RCPT TO:<new@example.test>").await, "250");
        assert_status(&client.command("DATA").await, "354");
        client
            .reader
            .get_mut()
            .write_all(&framed_data(b"Subject: reset\r\n\r\nbody\r\n"))
            .await
            .unwrap();
        assert_status(&client.reply().await, "250");
        let message = client.messages.recv().await.unwrap();
        assert_eq!(
            message.facts.envelope_from.unwrap().address,
            "new@example.test"
        );
        assert_eq!(message.facts.envelope_to.len(), 1);
        assert_eq!(message.facts.envelope_to[0].address, "new@example.test");
    }
    assert_status(&client.command("QUIT").await, "221");
    client.finish().await;
}

/// RFC 5321 §2.2: unsupported optional extensions are not advertised and cannot silently take effect.
#[tokio::test]
async fn rfc5321_unimplemented_extensions_are_not_advertised_or_accepted() {
    let mut client = ProtocolClient::start(4096).await;
    let capabilities = client.greet().await;
    for extension in ["SMTPUTF8", "CHUNKING", "BINARYMIME", "DSN", "REQUIRETLS"] {
        assert!(
            !capabilities
                .iter()
                .any(|line| line[4..].starts_with(extension)),
            "{capabilities:?}"
        );
    }
    for command in [
        "BDAT 0 LAST",
        "EXPN list",
        "TURN",
        "ETRN example.test",
        "ATRN example.test",
        "BURL imap://example.test/message",
    ] {
        assert_status(&client.command(command).await, "502");
    }
    for command in [
        "MAIL FROM:<> SMTPUTF8",
        "MAIL FROM:<> RET=FULL",
        "MAIL FROM:<> REQUIRETLS",
    ] {
        assert_status(&client.command(command).await, "555");
    }
    assert_status(&client.command("MAIL FROM:<> BODY=BINARYMIME").await, "501");
    assert_status(&client.command("MAIL FROM:<é@example.test>").await, "500");
    assert_status(&client.command("MAIL FROM:<>").await, "250");
    assert_status(
        &client.command("RCPT TO:<Postmaster> NOTIFY=SUCCESS").await,
        "555",
    );
    assert_status(&client.command("DATA").await, "503");
    assert!(client.messages.try_recv().is_err());
    assert_status(&client.command("QUIT").await, "221");
    client.finish().await;
}
