//! RFC 5321 reply classes, enhanced status codes, and the SIZE extension.
use super::support::*;
use crate::{AuthenticationPolicy, Limits};

/// Start an optional-authentication plaintext session with default limits.
fn compliance_client() -> tokio::io::BufReader<tokio::io::DuplexStream> {
    start_session(
        ClosureHandler(record_into(RecordedDeliveries::default())),
        AuthenticationPolicy::Optional,
        Limits::default(),
    )
}

/// Unknown, unimplemented, malformed, and informational commands get their RFC 5321 replies.
#[tokio::test]
async fn commands_receive_their_rfc_5321_reply_classes() {
    let mut reader = compliance_client();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("220 sandpost.localhost ESMTP")
    );
    for (command, expected) in [
        ("FROBNICATE", "500 5.5.2"),
        ("EXPN staff", "502 5.5.1"),
        ("EHLO", "501 5.5.4"),
        ("DATA now", "501 5.5.4"),
        ("VRFY postmaster", "252 2.1.5"),
        ("HELP", "214 2.0.0"),
        ("NOOP", "250 2.0.0"),
        ("EHLO client", "250 AUTH"),
        ("RSET", "250 2.0.0"),
        ("MAIL FROM:<app@example.test>", "250 2.1.0"),
        ("RCPT TO:<qa@example.test>", "250 2.1.5"),
        ("RCPT TO:<qa@example.test> NOTIFY=NEVER", "555 5.5.4"),
        ("RCPT TO:<broken>", "501 5.1.3"),
        ("RSET", "250 2.0.0"),
        ("MAIL FROM:<broken>", "501 5.1.7"),
        ("MAIL FROM:<> BODY=BINARYMIME", "501 5.5.4"),
        ("QUIT", "221 2.0.0"),
    ] {
        let response = send_mail_protocol_command(&mut reader, command).await;
        assert!(response.starts_with(expected), "{command:?}: {response}");
    }
}

/// A SIZE larger than the limit is refused at MAIL FROM, before any data is sent.
#[tokio::test]
async fn declared_sizes_over_the_limit_are_refused_early() {
    let mut reader = start_session(
        ClosureHandler(record_into(RecordedDeliveries::default())),
        AuthenticationPolicy::Optional,
        Limits {
            maximum_message_size: 4096,
            ..Limits::default()
        },
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert_eq!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test> SIZE=4097").await,
        "552 5.3.4 message size exceeds the 4096 bytes limit\r\n"
    );
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test> SIZE=4096")
            .await
            .starts_with("250 2.1.0")
    );
}

/// Capture full messages so trace fields, envelope spelling and submitted SIZE can be checked together.
fn capturing_client(
    limits: Limits,
) -> (
    tokio::io::BufReader<tokio::io::DuplexStream>,
    tokio::sync::mpsc::Receiver<sandpost_core::Message>,
) {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let client = start_session(
        ClosureHandler(move |message, _| {
            let sender = sender.clone();
            async move { sender.send(message).await.map_err(std::io::Error::other) }
        }),
        AuthenticationPolicy::Optional,
        limits,
    );
    (client, receiver)
}

/// Ordinary commands keep the 512-byte bound while EHLO permits the AUTH extension's longer MAIL command.
#[tokio::test]
async fn command_limits_follow_the_negotiated_extensions() {
    let mut reader = compliance_client();
    read_mail_protocol_response_line(&mut reader).await;
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<a@example.test>")
            .await
            .starts_with("503")
    );
    assert!(
        send_mail_protocol_command(&mut reader, &format!("NOOP {}", "x".repeat(505)))
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, &format!("NOOP {}", "x".repeat(506)))
            .await
            .starts_with("500")
    );
    send_mail_protocol_command(&mut reader, "HELO client").await;
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<> SIZE=1")
            .await
            .starts_with("555")
    );
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let domain = ["a".repeat(63), "b".repeat(63), "c".repeat(61)].join(".");
    let identity = format!("{}@{domain}", "u".repeat(64));
    let encoded: String = identity
        .bytes()
        .map(|byte| format!("+{byte:02X}"))
        .collect();
    let command = format!("MAIL FROM:<> AUTH={encoded}");
    assert!(command.len() + 2 > 512);
    assert!(
        send_mail_protocol_command(&mut reader, &command)
            .await
            .starts_with("250")
    );
}

/// Capture supplies one final Return-Path and prepends Received without altering older trace fields or the body.
#[tokio::test]
async fn capture_preserves_transport_addresses_and_adds_authoritative_trace() {
    use tokio::io::AsyncWriteExt;
    let (mut reader, mut messages) = capturing_client(Limits::default());
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client.example").await;
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<\"App@QA\"@Example.TEST>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RCPT TO:<PoStMaStEr>")
            .await
            .starts_with("250")
    );
    send_mail_protocol_command(&mut reader, "DATA").await;
    let original = b"Return-Path: <old@example.test>\r\n\t(ignored)\r\nReceived: original trace\r\nSubject: trace\r\n\r\nReturn-Path: body text\r\n\xff\0\r\n";
    reader.get_mut().write_all(original).await.unwrap();
    reader.get_mut().write_all(b".\r\n").await.unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("250")
    );
    let message = messages.recv().await.unwrap();
    assert_eq!(
        message.facts.envelope_from.as_ref().unwrap().address,
        "\"App@QA\"@example.test"
    );
    assert_eq!(
        message.facts.envelope_to[0].address,
        "PoStMaStEr@sandpost.localhost"
    );
    assert!(message.raw_message.starts_with(
        b"Return-Path: <\"App@QA\"@example.test>\r\nReceived: from client.example\r\n"
    ));
    assert!(
        message.raw_message.ends_with(
            &original[original
                .windows(9)
                .position(|window| window == b"Received:")
                .unwrap()..]
        )
    );
    assert_eq!(
        message.facts.headers["return-path"],
        ["<\"App@QA\"@example.test>"]
    );
    assert_eq!(message.facts.headers["received"].len(), 2);
    assert_eq!(message.facts.headers["received"][1], "original trace");
    assert!(message.facts.headers["received"][0].contains("by sandpost.localhost with ESMTP;"));
    let date = message.facts.headers["received"][0]
        .split_once(';')
        .unwrap()
        .1
        .trim();
    assert!(mail_parser::DateTime::parse_rfc822(date).is_some());
    assert_eq!(message.facts.size, message.raw_message.len() as u64);
}

/// Generated trace bytes are outside the advertised client SIZE budget, even at its exact boundary.
#[tokio::test]
async fn generated_trace_does_not_consume_the_submitted_size_limit() {
    use tokio::io::AsyncWriteExt;
    let input = b"Subject: exact\r\n\r\nbody\r\n";
    let (mut reader, mut messages) = capturing_client(Limits {
        maximum_message_size: input.len(),
        ..Limits::default()
    });
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    send_mail_protocol_command(&mut reader, &format!("MAIL FROM:<> SIZE={}", input.len())).await;
    send_mail_protocol_command(&mut reader, "RCPT TO:<qa@example.test>").await;
    send_mail_protocol_command(&mut reader, "DATA").await;
    reader.get_mut().write_all(input).await.unwrap();
    reader.get_mut().write_all(b".\r\n").await.unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("250")
    );
    let message = messages.recv().await.unwrap();
    assert!(message.raw_message.len() > input.len());
    assert!(message.raw_message.ends_with(input));
    assert_eq!(message.facts.headers["return-path"], ["<>"]);
}

/// Default SMTP command waits meet the five-minute recommendation while handler and handshake bounds stay separate.
#[test]
fn default_command_timeout_is_five_minutes() {
    assert_eq!(
        Limits::default().input_output_timeout,
        std::time::Duration::from_secs(300)
    );
}
