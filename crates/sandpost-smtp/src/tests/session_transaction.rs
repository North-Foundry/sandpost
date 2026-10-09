//! Mail transactions: envelopes, DATA framing, delivery outcomes, and command syntax tolerance.
use super::support::*;
use crate::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    Limits, SessionHandler,
};
use sandpost_core::Message;
use std::{future::Future, sync::Arc};
use tokio::io::AsyncWriteExt;

#[tokio::test]
/// Verify null senders and non-UTF-8 message bytes survive SMTP ingestion.
async fn null_sender_and_binary_data_are_preserved() {
    let (message_sender, mut message_receiver) = tokio::sync::mpsc::channel(1);
    let mut reader = mail_protocol_test_client(move |message| {
        let message_sender = message_sender.clone();
        async move {
            message_sender
                .send(message)
                .await
                .map_err(std::io::Error::other)
        }
    })
    .await;
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("220")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "EHLO local")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>")
            .await
            .starts_with("503")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<> X-UNKNOWN=4")
            .await
            .starts_with("555 5.5.4")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RSET")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("503")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    let mut protocol_bytes = b"From: a@example.com\r\nSubject: Dot\r\n\r\n..dot\r\n".to_vec();
    protocol_bytes.extend_from_slice(b"\xff\0\r\n.\r\n");
    reader.get_mut().write_all(&protocol_bytes).await.unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("250")
    );
    let message = message_receiver.recv().await.unwrap();
    assert!(message.facts.envelope_from.is_none());
    assert!(
        message
            .raw_message
            .ends_with(b"From: a@example.com\r\nSubject: Dot\r\n\r\n.dot\r\n\xff\0\r\n")
    );
    assert!(
        message
            .raw_message
            .starts_with(b"Return-Path: <>\r\nReceived: from local\r\n")
    );
    assert_eq!(message.facts.envelope_to[0].address, "dest@example.com");
}

#[tokio::test]
/// Verify both successful and failed persistence reset the SMTP transaction.
async fn persistence_failure_and_success_both_reset_the_transaction() {
    let persistence_call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler_call_count = Arc::clone(&persistence_call_count);
    let mut reader = mail_protocol_test_client(move |_| {
        let call_index = handler_call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async move {
            if call_index == 0 {
                Err(std::io::Error::other("storage unavailable"))
            } else {
                Ok(())
            }
        }
    })
    .await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let _ = send_mail_protocol_command(&mut reader, "EHLO local").await;

    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<sender@example.com>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    reader
        .get_mut()
        .write_all(b"Subject: fail\r\n\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("451")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("503")
    );

    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<sender@example.com>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    reader
        .get_mut()
        .write_all(b"Subject: success\r\n\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("503")
    );
    assert_eq!(
        persistence_call_count.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

#[tokio::test]
/// Verify QUIT and disconnect before DATA do not invoke the persistence handler.
async fn quit_and_disconnect_before_data_do_not_persist() {
    let persistence_call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler_call_count = Arc::clone(&persistence_call_count);
    let handler = move |_| {
        handler_call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Ok::<_, std::io::Error>(()) }
    };

    let mut quit_reader = mail_protocol_test_client(handler).await;
    let _ = read_mail_protocol_response_line(&mut quit_reader).await;
    let _ = send_mail_protocol_command(&mut quit_reader, "EHLO local").await;
    let _ = send_mail_protocol_command(&mut quit_reader, "MAIL FROM:<sender@example.com>").await;
    let _ = send_mail_protocol_command(&mut quit_reader, "RCPT TO:<dest@example.com>").await;
    assert!(
        send_mail_protocol_command(&mut quit_reader, "QUIT")
            .await
            .starts_with("221")
    );

    let handler_call_count = Arc::clone(&persistence_call_count);
    let mut disconnect_reader = mail_protocol_test_client(move |_| {
        handler_call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Ok::<_, std::io::Error>(()) }
    })
    .await;
    let _ = read_mail_protocol_response_line(&mut disconnect_reader).await;
    let _ = send_mail_protocol_command(&mut disconnect_reader, "EHLO local").await;
    let _ =
        send_mail_protocol_command(&mut disconnect_reader, "MAIL FROM:<sender@example.com>").await;
    let _ = send_mail_protocol_command(&mut disconnect_reader, "RCPT TO:<dest@example.com>").await;
    disconnect_reader.get_mut().shutdown().await.unwrap();
    assert_eq!(
        read_mail_protocol_response_line(&mut disconnect_reader).await,
        ""
    );
    assert_eq!(
        persistence_call_count.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
/// Verify incomplete DATA terminated by EOF never invokes persistence.
async fn incomplete_data_closure_does_not_persist() {
    let (message_sender, mut message_receiver) = tokio::sync::mpsc::channel(1);
    let mut reader = mail_protocol_test_client(move |message| {
        let message_sender = message_sender.clone();
        async move {
            message_sender
                .send(message)
                .await
                .map_err(std::io::Error::other)
        }
    })
    .await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let _ = send_mail_protocol_command(&mut reader, "EHLO local").await;
    let _ = send_mail_protocol_command(&mut reader, "MAIL FROM:<sender@example.com>").await;
    let _ = send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>").await;
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    reader
        .get_mut()
        .write_all(b"Subject: incomplete\r\n\r\nbody\r\n")
        .await
        .unwrap();
    reader.get_mut().shutdown().await.unwrap();
    assert_eq!(read_mail_protocol_response_line(&mut reader).await, "");
    assert!(message_receiver.try_recv().is_err());
}

#[tokio::test]
/// Verify a nested MAIL FROM is refused with 503 and a reset transaction accepts a new sender.
async fn nested_mail_from_is_refused_until_the_transaction_is_reset() {
    let (message_sender, mut message_receiver) = tokio::sync::mpsc::channel(1);
    let mut reader = mail_protocol_test_client(move |message| {
        let message_sender = message_sender.clone();
        async move {
            message_sender
                .send(message)
                .await
                .map_err(std::io::Error::other)
        }
    })
    .await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let _ = send_mail_protocol_command(&mut reader, "EHLO local").await;
    let _ = send_mail_protocol_command(&mut reader, "MAIL FROM:<old@example.com>").await;
    let _ = send_mail_protocol_command(&mut reader, "RCPT TO:<old-dest@example.com>").await;
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<new@example.com>")
            .await
            .starts_with("503 5.5.1 nested MAIL")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "RSET")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<new@example.com>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("503"),
        "the reset dropped the earlier recipient"
    );
    let _ = send_mail_protocol_command(&mut reader, "RCPT TO:<new-dest@example.com>").await;
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    reader
        .get_mut()
        .write_all(b"Subject: replacement\r\n\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("250")
    );
    let message = message_receiver.recv().await.unwrap();
    assert_eq!(
        message
            .facts
            .envelope_from
            .as_ref()
            .map(|mailbox| mailbox.address.as_str()),
        Some("new@example.com")
    );
    assert_eq!(message.facts.envelope_to.len(), 1);
    assert_eq!(message.facts.envelope_to[0].address, "new-dest@example.com");
}

#[tokio::test]
/// Verify the recipient limit rejects the next recipient and accepts DATA for the allowed set.
async fn recipient_limit_accepts_one_hundred_recipients() {
    let (message_sender, mut message_receiver) = tokio::sync::mpsc::channel(1);
    let mut reader = mail_protocol_test_client(move |message| {
        let message_sender = message_sender.clone();
        async move {
            message_sender
                .send(message)
                .await
                .map_err(std::io::Error::other)
        }
    })
    .await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let _ = send_mail_protocol_command(&mut reader, "EHLO local").await;
    let _ = send_mail_protocol_command(&mut reader, "MAIL FROM:<sender@example.com>").await;
    for recipient_index in 0..100 {
        assert!(
            send_mail_protocol_command(
                &mut reader,
                &format!("RCPT TO:<recipient-{recipient_index}@example.com>")
            )
            .await
            .starts_with("250")
        );
    }
    assert!(
        send_mail_protocol_command(&mut reader, "RCPT TO:<overflow@example.com>")
            .await
            .starts_with("452")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    reader
        .get_mut()
        .write_all(b"Subject: recipients\r\n\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("250")
    );
    assert_eq!(
        message_receiver
            .recv()
            .await
            .unwrap()
            .facts
            .envelope_to
            .len(),
        100
    );
}

/// A command line that is not UTF-8 is answered with 500 and the session keeps working.
#[tokio::test]
async fn non_utf8_command_is_refused_without_ending_the_session() {
    let mut reader = mail_protocol_test_client(|_| async { Ok::<_, std::io::Error>(()) }).await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    reader.get_mut().write_all(b"NO\xffOP\r\n").await.unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("500 5.5.2")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "NOOP")
            .await
            .starts_with("250")
    );
}

/// Leading whitespace and lowercase command names still yield the right argument.
#[tokio::test]
async fn commands_tolerate_leading_whitespace_and_any_case() {
    let (message_sender, mut message_receiver) = tokio::sync::mpsc::channel(1);
    let mut reader = mail_protocol_test_client(move |message| {
        let message_sender = message_sender.clone();
        async move {
            message_sender
                .send(message)
                .await
                .map_err(std::io::Error::other)
        }
    })
    .await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    for (command, expected) in [
        ("  ehlo local", "250"),
        (" mail from:<padded@example.com>", "250"),
        ("\tRcpt To:<dest@example.com>", "250"),
        ("  data", "354"),
    ] {
        let response = send_mail_protocol_command(&mut reader, command).await;
        assert!(response.starts_with(expected), "{command:?}: {response}");
    }
    reader
        .get_mut()
        .write_all(b"Subject: padded\r\n\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("250")
    );
    let message = message_receiver.recv().await.unwrap();
    assert_eq!(
        message.facts.envelope_from.unwrap().address,
        "padded@example.com"
    );
}

/// A permanently rejected delivery, such as revoked credentials, is reported with 554.
#[tokio::test]
async fn rejected_delivery_is_reported_permanently() {
    struct RejectingHandler(String);
    impl SessionHandler for RejectingHandler {
        type Principal = ();
        /// Accept every credential.
        fn authenticate(
            &self,
            _credentials: Credentials,
        ) -> impl Future<Output = Result<AuthenticationOutcome<()>, AuthenticationError>> + Send
        {
            std::future::ready(Ok(AuthenticationOutcome::Authenticated(())))
        }
        /// Refuse every message as if its credentials had been revoked.
        fn deliver(
            &self,
            _message: Message,
            _principal: Option<()>,
        ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
            std::future::ready(Err(DeliveryError::Rejected(self.0.clone())))
        }
    }
    let mut reader = start_session(
        RejectingHandler("credentials revoked\r\ninjected".into()),
        AuthenticationPolicy::Optional,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert_eq!(
        submit_test_message(&mut reader, "refused").await,
        "554 5.7.1 credentials revoked  injected\r\n"
    );
    let mut reader = start_session(
        RejectingHandler(format!("{}\r\n250 injected\0", "é".repeat(300))),
        AuthenticationPolicy::Optional,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let response = submit_test_message(&mut reader, "unicode refusal").await;
    assert!(response.starts_with("554 5.7.1 "));
    assert!(response.is_ascii());
    assert!(response.len() <= crate::limits::MAXIMUM_REPLY_LINE_SIZE);
    assert_eq!(
        send_mail_protocol_command(&mut reader, "NOOP").await,
        "250 2.0.0 OK\r\n"
    );
}
