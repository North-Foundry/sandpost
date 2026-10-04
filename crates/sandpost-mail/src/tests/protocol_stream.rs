use crate::{
    limits::{INPUT_OUTPUT_TIMEOUT, MAXIMUM_LINE_SIZE, MAXIMUM_MESSAGE_SIZE},
    protocol_stream::{
        MessageDataError, read_bounded_protocol_line, read_message_data,
        write_mail_protocol_response,
    },
};
use tokio::io::BufReader;

/// Count CRLF bytes in line limits and retain following lines across fragmented reads.
#[tokio::test]
async fn protocol_lines_enforce_exact_byte_limits_and_preserve_following_lines() {
    let mut protocol_bytes = vec![b'x'; MAXIMUM_LINE_SIZE - 2];
    protocol_bytes.extend_from_slice(b"\r\nNOOP\r\n");
    let mut reader = BufReader::with_capacity(1, protocol_bytes.as_slice());
    let line = read_bounded_protocol_line(&mut reader, MAXIMUM_LINE_SIZE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line, vec![b'x'; MAXIMUM_LINE_SIZE - 2]);
    assert_eq!(
        read_bounded_protocol_line(&mut reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap(),
        Some(b"NOOP".to_vec())
    );
    assert!(
        read_bounded_protocol_line(&mut reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap()
            .is_none()
    );

    let mut oversized_line = vec![b'x'; MAXIMUM_LINE_SIZE - 1];
    oversized_line.extend_from_slice(b"\r\n");
    let error = read_bounded_protocol_line(
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
        read_bounded_protocol_line(&mut empty_reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap()
            .is_none()
    );
    for input in [&b"NOOP"[..], &b"NOOP\n"[..], &b"NOOP\r"[..]] {
        let error = read_bounded_protocol_line(&mut BufReader::new(input), MAXIMUM_LINE_SIZE)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}

/// Undo one leading escape dot, preserve binary bytes, and leave subsequent commands unread.
#[tokio::test]
async fn message_data_preserves_binary_bytes_and_following_commands() {
    let protocol_bytes = b"..first\r\n...second\r\ninside.dot\r\n\xff\0\r\n.\r\nNOOP\r\n";
    let mut reader = BufReader::with_capacity(2, protocol_bytes.as_slice());
    assert_eq!(
        read_message_data(&mut reader).await.unwrap(),
        b".first\r\n..second\r\ninside.dot\r\n\xff\0\r\n"
    );
    assert_eq!(
        read_bounded_protocol_line(&mut reader, MAXIMUM_LINE_SIZE)
            .await
            .unwrap(),
        Some(b"NOOP".to_vec())
    );
}

/// Reject DATA closed before its standalone terminator rather than accepting partial content.
#[tokio::test]
async fn message_data_rejects_end_of_stream_before_terminator() {
    for input in [&b""[..], &b"Subject: incomplete\r\n\r\nbody\r\n"[..]] {
        let error = read_message_data(&mut BufReader::new(input))
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
    let protocol_bytes = framed_message_data(MAXIMUM_MESSAGE_SIZE);
    let data = read_message_data(&mut BufReader::new(protocol_bytes.as_slice()))
        .await
        .unwrap();
    assert_eq!(data.len(), MAXIMUM_MESSAGE_SIZE);
    assert_eq!(data.as_slice(), &protocol_bytes[..MAXIMUM_MESSAGE_SIZE]);

    let oversized_data = framed_message_data(MAXIMUM_MESSAGE_SIZE + 1);
    assert!(matches!(
        read_message_data(&mut BufReader::new(oversized_data.as_slice())).await,
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
        read_message_data(&mut reader).await,
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
    let error = write_mail_protocol_response(&mut server_stream, "250 ok")
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(started_at.elapsed(), INPUT_OUTPUT_TIMEOUT);
    drop(peer_stream);
}
