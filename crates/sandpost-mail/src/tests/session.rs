use crate::{
    limits::{INPUT_OUTPUT_TIMEOUT, MAXIMUM_LINE_SIZE, MAXIMUM_MESSAGE_SIZE},
    session::{mail_protocol_session, parse_mail_protocol_path},
};
use sandpost_core::Message;
use std::{future::Future, sync::Arc};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// Start a mail protocol session over an in-memory duplex stream for protocol tests.
async fn mail_protocol_test_client<Handler, HandlerFuture>(
    handler: Handler,
) -> tokio::io::BufReader<tokio::io::DuplexStream>
where
    Handler: Fn(Message) -> HandlerFuture + Send + Sync + 'static,
    HandlerFuture: Future<Output = Result<(), std::io::Error>> + Send + 'static,
{
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let _ = mail_protocol_session(server_stream, Arc::new(handler)).await;
    });
    tokio::io::BufReader::new(client_stream)
}

/// Read one mail protocol response line from the test client.
async fn read_mail_protocol_response_line(
    reader: &mut tokio::io::BufReader<tokio::io::DuplexStream>,
) -> String {
    let mut response_line = String::new();
    reader.read_line(&mut response_line).await.unwrap();
    response_line
}

/// Send one mail protocol command and return its response line.
async fn send_mail_protocol_command(
    reader: &mut tokio::io::BufReader<tokio::io::DuplexStream>,
    command_text: &str,
) -> String {
    reader
        .get_mut()
        .write_all(format!("{command_text}\r\n").as_bytes())
        .await
        .unwrap();
    read_mail_protocol_response_line(reader).await
}

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
        send_mail_protocol_command(&mut reader, "MAIL FROM:<> SIZE=4")
            .await
            .starts_with("501")
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
    assert_eq!(
        message.raw_message,
        b"From: a@example.com\r\nSubject: Dot\r\n\r\n.dot\r\n\xff\0\r\n"
    );
    assert_eq!(message.facts.envelope_to[0].address, "dest@example.com");
}

#[tokio::test]
/// Verify oversized mail protocol DATA is rejected without persisting a message.
async fn data_limit_is_enforced() {
    let mut reader = mail_protocol_test_client(|_| async { Ok::<_, std::io::Error>(()) }).await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let _ = send_mail_protocol_command(&mut reader, "EHLO local").await;
    let _ = send_mail_protocol_command(&mut reader, "MAIL FROM:<sender@example.com>").await;
    let _ = send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>").await;
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    let mut input_bytes = Vec::with_capacity(MAXIMUM_MESSAGE_SIZE + 64);
    for _ in 0..10_500 {
        input_bytes.extend_from_slice(&[b'x'; 998]);
        input_bytes.extend_from_slice(b"\r\n");
    }
    input_bytes.extend_from_slice(b".\r\n");
    let _ = reader.get_mut().write_all(&input_bytes).await;
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("552")
    );
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
/// Verify an oversized command closes the mail protocol session without reading later commands.
async fn oversized_command_closes_without_reading_trailing_commands() {
    let mut reader = mail_protocol_test_client(|_| async { Ok::<_, std::io::Error>(()) }).await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let mut protocol_bytes = vec![b'X'; MAXIMUM_LINE_SIZE + 1];
    protocol_bytes.extend_from_slice(b"\r\nNOOP\r\n");
    reader.get_mut().write_all(&protocol_bytes).await.unwrap();
    let mut response_line = String::new();
    assert_eq!(reader.read_line(&mut response_line).await.unwrap(), 0);
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
/// Verify replacing MAIL FROM clears prior recipients and uses the new sender.
async fn replacing_mail_from_resets_recipients() {
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
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("503")
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

#[test]
/// Verify malformed SMTP paths and disallowed null reverse paths are rejected.
fn malformed_mail_protocol_paths_are_rejected() {
    for malformed_path in [
        "sender@example.com",
        "<sender@example.com",
        "<sender@example.com> trailing",
        "<>",
    ] {
        assert_eq!(parse_mail_protocol_path(malformed_path, false), None);
    }
    assert_eq!(parse_mail_protocol_path("<>", true), Some(None));
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

#[tokio::test(start_paused = true)]
/// Verify a pending persistence callback delays 250, times out with 451, and resets the transaction.
async fn persistence_callback_gates_acknowledgment_and_timeout_resets_transaction() {
    let (callback_sender, mut callback_receiver) =
        tokio::sync::mpsc::channel::<tokio::sync::oneshot::Sender<()>>(2);
    let mut reader = mail_protocol_test_client(move |_| {
        let callback_sender = callback_sender.clone();
        async move {
            let (release_sender, release_receiver) = tokio::sync::oneshot::channel();
            callback_sender.send(release_sender).await.unwrap();
            let _ = release_receiver.await;
            Ok::<_, std::io::Error>(())
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
        .write_all(b"Subject: delayed\r\n\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    let first_callback_release = callback_receiver.recv().await.unwrap();
    let acknowledgment_task = tokio::spawn(async move {
        let response_line = read_mail_protocol_response_line(&mut reader).await;
        (reader, response_line)
    });
    tokio::task::yield_now().await;
    assert!(!acknowledgment_task.is_finished());
    first_callback_release.send(()).unwrap();
    let (mut reader, acknowledgment_line) = acknowledgment_task.await.unwrap();
    assert!(acknowledgment_line.starts_with("250"));

    let _ = send_mail_protocol_command(&mut reader, "MAIL FROM:<sender@example.com>").await;
    let _ = send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>").await;
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    reader
        .get_mut()
        .write_all(b"Subject: timeout\r\n\r\nbody\r\n.\r\n")
        .await
        .unwrap();
    let _second_callback_release = callback_receiver.recv().await.unwrap();
    tokio::time::advance(INPUT_OUTPUT_TIMEOUT).await;
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
}
