//! Bounded protocol lines, message data framing, and reply writes.
use crate::limits::{MAXIMUM_LINE_SIZE, MAXIMUM_REPLY_LINE_SIZE};
use std::time::Duration;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};

/// The outcome of waiting for one client line.
#[derive(Debug)]
pub(crate) enum ReceivedLine {
    /// A complete line without its CRLF.
    Line(Vec<u8>),
    /// The client closed the connection between lines.
    Closed,
    /// No complete line arrived in time.
    TimedOut,
}

/// Private marker carried by `InvalidData` errors for a line that exceeded its byte bound.
#[derive(Debug)]
struct LineTooLong;

impl std::fmt::Display for LineTooLong {
    /// Describe an overlong protocol line.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("line too long")
    }
}

impl std::error::Error for LineTooLong {}

/// Return whether an input error was raised because a line exceeded its configured bound.
pub(crate) fn is_line_too_long(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|source| source.is::<LineTooLong>())
}

/// Wait for one protocol line using a caller-selected byte limit.
///
/// An overlong, unterminated, or bare-LF line is an `InvalidData` error. Overlength errors carry
/// a private marker detectable with [`is_line_too_long`].
pub(crate) async fn read_line_with_limit<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    read_timeout: Duration,
    limit: usize,
) -> std::io::Result<ReceivedLine> {
    match timeout(read_timeout, read_bounded_line(reader, limit)).await {
        Ok(Ok(Some(line))) => Ok(ReceivedLine::Line(line)),
        Ok(Ok(None)) => Ok(ReceivedLine::Closed),
        Ok(Err(error)) => Err(error),
        Err(_) => Ok(ReceivedLine::TimedOut),
    }
}

/// Read one CRLF-terminated protocol line while enforcing its byte limit.
///
/// Returns `None` when the stream ends cleanly before any byte of a new line.
pub(crate) async fn read_bounded_line<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    limit: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line_bytes = Vec::with_capacity(128);
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if line_bytes.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unterminated line",
                ))
            };
        }
        let byte_count = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if line_bytes.len().saturating_add(byte_count) > limit {
            let discarded_byte_count = (limit - line_bytes.len() + 1).min(available.len());
            reader.consume(discarded_byte_count);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                LineTooLong,
            ));
        }
        let line_is_complete = available[byte_count - 1] == b'\n';
        line_bytes.extend_from_slice(&available[..byte_count]);
        reader.consume(byte_count);
        if line_is_complete {
            if line_bytes.len() < 2 || line_bytes[line_bytes.len() - 2] != b'\r' {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing CRLF",
                ));
            }
            line_bytes.truncate(line_bytes.len() - 2);
            return Ok(Some(line_bytes));
        }
    }
}

/// Why message data could not be received.
#[derive(Debug)]
pub(crate) enum MessageDataError {
    MessageTooLarge,
    ReadTimeout,
    InputOutput(std::io::Error),
}

/// Read DATA up to the lone `.` line, undo dot-stuffing, and enforce the message size limit.
///
/// Each line must arrive within `read_timeout`; the returned bytes end every line with CRLF.
pub(crate) async fn read_message_data<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
    maximum_size: usize,
    read_timeout: Duration,
) -> Result<Vec<u8>, MessageDataError> {
    let mut raw_message = Vec::new();
    loop {
        // DATA allows a single extra physical octet for a stuffed leading dot; the decoded line
        // is checked below against the RFC limit.
        let line = match read_line_with_limit(reader, read_timeout, MAXIMUM_LINE_SIZE + 1).await {
            Ok(ReceivedLine::Line(line)) => line,
            Ok(ReceivedLine::Closed) => {
                return Err(MessageDataError::InputOutput(std::io::Error::from(
                    std::io::ErrorKind::UnexpectedEof,
                )));
            }
            Ok(ReceivedLine::TimedOut) => return Err(MessageDataError::ReadTimeout),
            Err(error) => return Err(MessageDataError::InputOutput(error)),
        };
        if line == b"." {
            return Ok(raw_message);
        }
        // A client adds one leading dot to every content line that begins with a dot.
        let unstuffed_line = if line.starts_with(b".") {
            &line[1..]
        } else {
            &line[..]
        };
        if unstuffed_line.len().saturating_add(2) > MAXIMUM_LINE_SIZE {
            return Err(MessageDataError::InputOutput(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "DATA line too long",
            )));
        }
        if raw_message.len().saturating_add(unstuffed_line.len() + 2) > maximum_size {
            return Err(MessageDataError::MessageTooLarge);
        }
        raw_message.extend_from_slice(unstuffed_line);
        raw_message.extend_from_slice(b"\r\n");
    }
}

/// Write one ASCII reply line of at most [`MAXIMUM_REPLY_LINE_SIZE`] octets.
///
/// Rejects embedded CR/LF and non-ASCII text before writing, and fails with `TimedOut` if the
/// client stops reading.
pub(crate) async fn write_reply<Writer: AsyncWrite + Unpin>(
    writer: &mut Writer,
    reply: &str,
    write_timeout: Duration,
) -> std::io::Result<()> {
    if reply.len().saturating_add(2) > MAXIMUM_REPLY_LINE_SIZE
        || !reply.is_ascii()
        || reply.bytes().any(|byte| matches!(byte, b'\r' | b'\n'))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid SMTP reply line",
        ));
    }
    match timeout(write_timeout, async {
        writer.write_all(reply.as_bytes()).await?;
        writer.write_all(b"\r\n").await?;
        // TLS buffers records; flushing makes the reply leave before the next read.
        writer.flush().await
    })
    .await
    {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "SMTP write timed out",
        )),
    }
}
