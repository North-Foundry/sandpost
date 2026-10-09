//! Line framing, DATA framing with dot-unstuffing, and bounded reply writes.
use crate::{
    framing::{
        MessageDataError, is_line_too_long, read_bounded_line, read_line_with_limit,
        read_message_data, write_reply,
    },
    limits::{MAXIMUM_COMMAND_LINE_SIZE, MAXIMUM_LINE_SIZE, MAXIMUM_REPLY_LINE_SIZE},
};
use std::time::Duration;
use tokio::io::BufReader;

/// A small message size limit, so exact-boundary tests stay fast.
const MESSAGE_SIZE_LIMIT: usize = 64 * 1024;
/// The read and write timeout the framing tests run with.
const INPUT_OUTPUT_TIMEOUT: Duration = Duration::from_secs(60);

/// Count CRLF bytes in line limits and retain following lines across fragmented reads.
#[tokio::test]
async fn protocol_lines_enforce_exact_byte_limits_and_preserve_following_lines() {
    let mut protocol_bytes = vec![b'x'; MAXIMUM_LINE_SIZE - 2];
    protocol_bytes.extend_from_slice(b"\r\nNOOP\r\n");
    let mut reader = BufReader::with_capacity(1, protocol_bytes.as_slice());
    let line = read_bounded_line(&mut reader, MAXIMUM_LINE_SIZE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line, vec![b'x'; MAXIMUM_LINE_SIZE - 2]);
    assert_eq!(
        read_bounded_line(&mut reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap(),
        Some(b"NOOP".to_vec())
    );
    assert!(
        read_bounded_line(&mut reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap()
            .is_none()
    );

    let mut oversized_line = vec![b'x'; MAXIMUM_LINE_SIZE - 1];
    oversized_line.extend_from_slice(b"\r\n");
    let error = read_bounded_line(
        &mut BufReader::with_capacity(1, oversized_line.as_slice()),
        MAXIMUM_LINE_SIZE,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

/// Treat an empty stream as clean closure, but reject unfinished or non-CRLF lines.
#[tokio::test]
async fn protocol_lines_distinguish_clean_closure_from_invalid_terminators() {
    let mut empty_reader = BufReader::new(&b""[..]);
    assert!(
        read_bounded_line(&mut empty_reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap()
            .is_none()
    );
    for input in [&b"NOOP"[..], &b"NOOP\n"[..], &b"NOOP\r"[..]] {
        let error = read_bounded_line(&mut BufReader::new(input), MAXIMUM_LINE_SIZE)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}

/// Distinguish an overlong line from malformed CRLF while retaining `InvalidData` for both.
#[tokio::test]
async fn bounded_line_marks_only_overlong_input() {
    let mut oversized = vec![b'x'; MAXIMUM_COMMAND_LINE_SIZE];
    oversized.extend_from_slice(b"\r\n");
    let error = read_line_with_limit(
        &mut BufReader::new(oversized.as_slice()),
        INPUT_OUTPUT_TIMEOUT,
        MAXIMUM_COMMAND_LINE_SIZE,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(is_line_too_long(&error));

    let error = read_line_with_limit(
        &mut BufReader::new(&b"NOOP\n"[..]),
        INPUT_OUTPUT_TIMEOUT,
        MAXIMUM_COMMAND_LINE_SIZE,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(!is_line_too_long(&error));
}

/// Accept a stuffed 1000-octet DATA line despite its extra physical transparency dot.
#[tokio::test]
async fn message_data_accepts_maximum_unstuffed_dot_line() {
    let mut protocol_bytes = vec![b'.'];
    protocol_bytes.extend(std::iter::repeat_n(b'x', MAXIMUM_LINE_SIZE - 2));
    protocol_bytes.extend_from_slice(b"\r\n.\r\nNOOP\r\n");
    let mut reader = BufReader::with_capacity(3, protocol_bytes.as_slice());
    let data = read_message_data(&mut reader, 2048, INPUT_OUTPUT_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(data.len(), MAXIMUM_LINE_SIZE);
    assert_eq!(data[0], b'x');
    assert_eq!(&data[MAXIMUM_LINE_SIZE - 2..], b"\r\n");
    assert_eq!(
        read_bounded_line(&mut reader, MAXIMUM_COMMAND_LINE_SIZE)
            .await
            .unwrap(),
        Some(b"NOOP".to_vec())
    );
}

/// Reject an ordinary DATA line that exceeds 1000 octets after CRLF is restored.
#[tokio::test]
async fn message_data_rejects_oversized_unstuffed_line() {
    let mut protocol_bytes = vec![b'x'; MAXIMUM_LINE_SIZE - 1];
    protocol_bytes.extend_from_slice(b"\r\n.\r\n");
    assert!(matches!(
        read_message_data(
            &mut BufReader::new(protocol_bytes.as_slice()),
            2048,
            INPUT_OUTPUT_TIMEOUT
        )
        .await,
        Err(MessageDataError::InputOutput(error))
            if error.kind() == std::io::ErrorKind::InvalidData
    ));
}

/// Remove one transparency dot from any content line and preserve the terminator boundary.
#[tokio::test]
async fn message_data_unstuffs_single_dot_and_preserves_following_command() {
    let mut reader = BufReader::with_capacity(1, &b".foo\r\n.\r\nNOOP\r\n"[..]);
    assert_eq!(
        read_message_data(&mut reader, 2048, INPUT_OUTPUT_TIMEOUT)
            .await
            .unwrap(),
        b"foo\r\n"
    );
    assert_eq!(
        read_bounded_line(&mut reader, MAXIMUM_COMMAND_LINE_SIZE)
            .await
            .unwrap(),
        Some(b"NOOP".to_vec())
    );
}

/// Undo one leading escape dot, preserve binary bytes, and leave subsequent commands unread.
#[tokio::test]
async fn message_data_preserves_binary_bytes_and_following_commands() {
    let protocol_bytes = b"..first\r\n...second\r\ninside.dot\r\n\xff\0\r\n.\r\nNOOP\r\n";
    let mut reader = BufReader::with_capacity(2, protocol_bytes.as_slice());
    assert_eq!(
        read_message_data(&mut reader, MESSAGE_SIZE_LIMIT, INPUT_OUTPUT_TIMEOUT)
            .await
            .unwrap(),
        b".first\r\n..second\r\ninside.dot\r\n\xff\0\r\n"
    );
    assert_eq!(
        read_bounded_line(&mut reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap(),
        Some(b"NOOP".to_vec())
    );
}

/// Reject DATA closed before its standalone terminator rather than accepting partial content.
#[tokio::test]
async fn message_data_rejects_end_of_stream_before_terminator() {
    for input in [&b""[..], &b"Subject: incomplete\r\n\r\nbody\r\n"[..]] {
        let error = read_message_data(
            &mut BufReader::new(input),
            MESSAGE_SIZE_LIMIT,
            INPUT_OUTPUT_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            MessageDataError::InputOutput(error)
                if error.kind() == std::io::ErrorKind::UnexpectedEof
        ));
    }
}

/// Build terminated DATA of a specified decoded size with every line inside the protocol bound.
fn framed_message_data(message_size: usize) -> Vec<u8> {
    let mut protocol_bytes = Vec::with_capacity(message_size + 3);
    let mut remaining_bytes = message_size;
    while remaining_bytes > 0 {
        let line_size = remaining_bytes.min(MAXIMUM_LINE_SIZE);
        assert!(line_size >= 2, "test data must leave room for CRLF");
        protocol_bytes.extend(std::iter::repeat_n(b'x', line_size - 2));
        protocol_bytes.extend_from_slice(b"\r\n");
        remaining_bytes -= line_size;
    }
    protocol_bytes.extend_from_slice(b".\r\n");
    protocol_bytes
}

/// Accept the complete-message byte ceiling exactly, and reject one additional decoded byte.
#[tokio::test]
async fn message_data_accepts_exact_size_limit_and_rejects_next_byte() {
    let protocol_bytes = framed_message_data(MESSAGE_SIZE_LIMIT);
    let data = read_message_data(
        &mut BufReader::new(protocol_bytes.as_slice()),
        MESSAGE_SIZE_LIMIT,
        INPUT_OUTPUT_TIMEOUT,
    )
    .await
    .unwrap();
    assert_eq!(data.len(), MESSAGE_SIZE_LIMIT);
    assert_eq!(data.as_slice(), &protocol_bytes[..MESSAGE_SIZE_LIMIT]);

    let oversized_data = framed_message_data(MESSAGE_SIZE_LIMIT + 1);
    assert!(matches!(
        read_message_data(
            &mut BufReader::new(oversized_data.as_slice()),
            MESSAGE_SIZE_LIMIT,
            INPUT_OUTPUT_TIMEOUT
        )
        .await,
        Err(MessageDataError::MessageTooLarge)
    ));
}

/// Bound an idle DATA read without waiting sixty seconds of wall-clock time.
#[tokio::test(start_paused = true)]
async fn stalled_message_data_reads_time_out() {
    let (peer_stream, server_stream) = tokio::io::duplex(1);
    let mut reader = BufReader::new(server_stream);
    let started_at = tokio::time::Instant::now();
    assert!(matches!(
        read_message_data(&mut reader, MESSAGE_SIZE_LIMIT, INPUT_OUTPUT_TIMEOUT).await,
        Err(MessageDataError::ReadTimeout)
    ));
    assert_eq!(started_at.elapsed(), INPUT_OUTPUT_TIMEOUT);
    drop(peer_stream);
}

/// Bound a response write when the peer never consumes its limited receive buffer.
#[tokio::test(start_paused = true)]
async fn stalled_protocol_response_writes_time_out() {
    let (peer_stream, mut server_stream) = tokio::io::duplex(1);
    let started_at = tokio::time::Instant::now();
    let error = write_reply(&mut server_stream, "250 ok", INPUT_OUTPUT_TIMEOUT)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(started_at.elapsed(), INPUT_OUTPUT_TIMEOUT);
    drop(peer_stream);
}

/// Refuse replies that exceed the SMTP line bound or could inject another protocol line.
#[tokio::test]
async fn protocol_replies_reject_overlong_or_unsafe_text() {
    for reply in [
        "x".repeat(MAXIMUM_REPLY_LINE_SIZE),
        "250 caf\u{e9}".to_owned(),
        "250 first\r\n250 injected".to_owned(),
    ] {
        let mut output = Vec::new();
        let error = write_reply(&mut output, &reply, INPUT_OUTPUT_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(output.is_empty());
    }
    let mut output = Vec::new();
    write_reply(
        &mut output,
        &"x".repeat(MAXIMUM_REPLY_LINE_SIZE - 2),
        INPUT_OUTPUT_TIMEOUT,
    )
    .await
    .unwrap();
    assert_eq!(output.len(), MAXIMUM_REPLY_LINE_SIZE);
}
