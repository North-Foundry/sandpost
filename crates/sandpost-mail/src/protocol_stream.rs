//! Bounded protocol lines, message data framing, and response writes.
use crate::limits::{INPUT_OUTPUT_TIMEOUT, MAXIMUM_LINE_SIZE, MAXIMUM_MESSAGE_SIZE};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};

/// Read one CRLF-terminated protocol line while enforcing its byte limit.
pub(crate) async fn read_bounded_protocol_line<Reader: AsyncBufRead + Unpin>(
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
                "line too long",
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

#[derive(Debug)]
pub(crate) enum MessageDataError {
    MessageTooLarge,
    ReadTimeout,
    InputOutput(std::io::Error),
}

/// Read mail protocol DATA, undo dot-stuffing, and enforce the complete message size limit.
pub(crate) async fn read_message_data<Reader: AsyncBufRead + Unpin>(
    reader: &mut Reader,
) -> Result<Vec<u8>, MessageDataError> {
    let mut raw_message = Vec::new();
    loop {
        let message_data_line = match timeout(
            INPUT_OUTPUT_TIMEOUT,
            read_bounded_protocol_line(reader, MAXIMUM_LINE_SIZE),
        )
        .await
        {
            Ok(result) => result
                .map_err(MessageDataError::InputOutput)?
                .ok_or_else(|| {
                    MessageDataError::InputOutput(std::io::Error::from(
                        std::io::ErrorKind::UnexpectedEof,
                    ))
                })?,
            Err(_) => return Err(MessageDataError::ReadTimeout),
        };
        if message_data_line == b"." {
            return Ok(raw_message);
        }
        let message_data_line = if message_data_line.starts_with(b"..") {
            &message_data_line[1..]
        } else {
            &message_data_line[..]
        };
        if raw_message
            .len()
            .saturating_add(message_data_line.len() + 2)
            > MAXIMUM_MESSAGE_SIZE
        {
            return Err(MessageDataError::MessageTooLarge);
        }
        raw_message.extend_from_slice(message_data_line);
        raw_message.extend_from_slice(b"\r\n");
    }
}

/// Write one SMTP response line with a bounded network-write duration.
pub(crate) async fn write_mail_protocol_response<Writer: AsyncWrite + Unpin>(
    writer: &mut Writer,
    response_text: &str,
) -> std::io::Result<()> {
    match timeout(INPUT_OUTPUT_TIMEOUT, async {
        writer.write_all(response_text.as_bytes()).await?;
        writer.write_all(b"\r\n").await
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
