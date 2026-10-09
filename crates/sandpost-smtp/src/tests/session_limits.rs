//! Bounds on message size, line length, handler time, and session lifetime.
use super::support::*;
use crate::{
    Limits,
    limits::{MAXIMUM_EXTENDED_MAIL_LINE_SIZE, describe_size},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// Oversized DATA is refused with 552 naming the configured limit, nothing is delivered, and the
/// session closes.
#[tokio::test]
async fn data_over_the_configured_limit_is_refused() {
    let delivered = Arc::new(AtomicUsize::new(0));
    let delivery_count = Arc::clone(&delivered);
    let limits = Limits {
        maximum_message_size: 4096,
        ..Limits::default()
    };
    let mut reader = limited_test_client(limits, move |_| {
        delivery_count.fetch_add(1, Ordering::SeqCst);
        async { Ok::<_, std::io::Error>(()) }
    });
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let _ = send_mail_protocol_command(&mut reader, "EHLO local").await;
    let _ = send_mail_protocol_command(&mut reader, "MAIL FROM:<sender@example.com>").await;
    let _ = send_mail_protocol_command(&mut reader, "RCPT TO:<dest@example.com>").await;
    assert!(
        send_mail_protocol_command(&mut reader, "DATA")
            .await
            .starts_with("354")
    );
    let mut input_bytes = Vec::new();
    for _ in 0..5 {
        input_bytes.extend_from_slice(&[b'x'; 998]);
        input_bytes.extend_from_slice(b"\r\n");
    }
    input_bytes.extend_from_slice(b".\r\n");
    let _ = reader.get_mut().write_all(&input_bytes).await;
    assert_eq!(
        read_mail_protocol_response_line(&mut reader).await,
        "552 5.3.4 message exceeds 4096 bytes\r\n"
    );
    assert_eq!(read_mail_protocol_response_line(&mut reader).await, "");
    assert_eq!(delivered.load(Ordering::SeqCst), 0);
}

/// Size limits are described in whole mebibytes when exact and in bytes otherwise.
#[test]
fn size_limits_are_described_readably() {
    assert_eq!(
        describe_size(Limits::default().maximum_message_size),
        "10 MiB"
    );
    assert_eq!(describe_size(4096), "4096 bytes");
    assert_eq!(describe_size(3 * 1024 * 1024 + 1), "3145729 bytes");
}

#[tokio::test]
/// Verify an oversized command closes the mail protocol session without reading later commands.
async fn oversized_command_closes_without_reading_trailing_commands() {
    let mut reader = mail_protocol_test_client(|_| async { Ok::<_, std::io::Error>(()) }).await;
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let mut protocol_bytes = vec![b'X'; MAXIMUM_EXTENDED_MAIL_LINE_SIZE + 1];
    protocol_bytes.extend_from_slice(b"\r\nNOOP\r\n");
    reader.get_mut().write_all(&protocol_bytes).await.unwrap();
    let mut response_line = String::new();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("500 5.5.2")
    );
    assert_eq!(reader.read_line(&mut response_line).await.unwrap(), 0);
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
    tokio::time::advance(Limits::default().handler_timeout).await;
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

/// An active conversation is closed with 421 once the configured session lifetime passes.
#[tokio::test(start_paused = true)]
async fn session_time_limit_closes_an_active_conversation() {
    let limits = Limits {
        maximum_session_duration: Duration::from_secs(120),
        ..Limits::default()
    };
    let mut reader = limited_test_client(limits, |_| async { Ok::<_, std::io::Error>(()) });
    let _ = read_mail_protocol_response_line(&mut reader).await;
    let step = Duration::from_secs(30);
    let mut elapsed = Duration::ZERO;
    while elapsed <= limits.maximum_session_duration + step {
        tokio::time::advance(step).await;
        elapsed += step;
        // The server may already have closed; the reply below tells which happened.
        let _ = reader.get_mut().write_all(b"NOOP\r\n").await;
        let response = read_mail_protocol_response_line(&mut reader).await;
        if response.starts_with("421") {
            assert!(response.contains("session time limit"), "{response}");
            assert!(
                elapsed >= limits.maximum_session_duration,
                "closed after {elapsed:?}"
            );
            assert_eq!(read_mail_protocol_response_line(&mut reader).await, "");
            return;
        }
        assert!(response.starts_with("250"), "{response}");
    }
    panic!("the session outlived its time limit");
}
