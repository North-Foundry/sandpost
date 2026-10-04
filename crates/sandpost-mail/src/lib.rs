//! MIME normalization and a bounded local SMTP catcher.

use mail_parser::{Address, MessageParser, MimeHeaders};
use sandpost_core::{Attachment, Mailbox, Message, MessageFacts, MessageIdentifier};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Display, future::Future, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpListener,
    task::JoinSet,
    time::timeout,
};

const MAXIMUM_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
const MAXIMUM_LINE_SIZE: usize = 1000;
const MAXIMUM_RECIPIENT_COUNT: usize = 100;
const MAXIMUM_CONNECTION_COUNT: usize = 32;
const INPUT_OUTPUT_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum MailError {
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("SMTP I/O: {0}")]
    InputOutput(#[from] std::io::Error),
}

/// Parse a MIME message and normalize its mailbox and body fields.
pub fn parse_message(
    envelope_from: Option<&str>,
    envelope_to: &[String],
    raw_message: Vec<u8>,
    received_at: i64,
) -> Result<Message, MailError> {
    if raw_message.len() > MAXIMUM_MESSAGE_SIZE {
        return Err(MailError::InvalidMessage("message exceeds 10 MiB".into()));
    }
    let parsed = MessageParser::new()
        .with_mime_headers()
        .with_address_headers()
        .with_message_ids()
        .default_header_text()
        .parse(&raw_message)
        .ok_or_else(|| MailError::InvalidMessage("no parseable headers".into()))?;

    let envelope_from = envelope_from.map(mailbox).transpose()?;
    let envelope_to = envelope_to
        .iter()
        .map(|address| mailbox(address))
        .collect::<Result<_, _>>()?;
    let from = parsed
        .from()
        .into_iter()
        .flat_map(Address::iter)
        .filter_map(|address| address.address())
        .map(mailbox)
        .collect::<Result<_, _>>()?;
    let to = parsed
        .all_to()
        .flat_map(Address::iter)
        .filter_map(|address| address.address())
        .map(mailbox)
        .collect::<Result<_, _>>()?;
    let carbon_copy = parsed
        .all_cc()
        .flat_map(Address::iter)
        .filter_map(|address| address.address())
        .map(mailbox)
        .collect::<Result<_, _>>()?;

    let mut headers = BTreeMap::<String, Vec<String>>::new();
    for (header_index, (header_name, raw_header_value)) in parsed.headers_raw().enumerate() {
        let value = parsed
            .headers()
            .get(header_index)
            .and_then(|header| header.value().as_text())
            .unwrap_or(raw_header_value);
        headers
            .entry(header_name.to_ascii_lowercase())
            .or_default()
            .push(unfold_header(value));
    }
    let text = (0..parsed.text_body_count())
        .filter_map(|body_part_index| parsed.body_text(body_part_index))
        .map(|part| part.into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let markup_body = (0..parsed.html_body_count())
        .filter_map(|body_part_index| parsed.body_html(body_part_index))
        .map(|part| part.into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let attachments = parsed
        .attachments()
        .map(|part| {
            let attachment_bytes = part.contents();
            let content_type = part
                .content_type()
                .map(|content_type| match content_type.subtype() {
                    Some(subtype) => format!("{}/{}", content_type.ctype(), subtype),
                    None => content_type.ctype().to_owned(),
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            Attachment {
                filename: part.attachment_name().map(str::to_owned),
                content_type,
                size: attachment_bytes.len() as u64,
                content_hash: format!("{:x}", Sha256::digest(attachment_bytes)),
            }
        })
        .collect::<Vec<_>>();

    Ok(Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            envelope_from,
            envelope_to,
            from,
            to,
            carbon_copy,
            subject: parsed.subject().unwrap_or_default().to_owned(),
            text,
            markup_body,
            message_identifier: parsed.message_id().map(str::to_owned),
            received_at,
            size: raw_message.len() as u64,
            attachment_count: attachments.len() as u64,
            headers,
        },
        raw_message,
        attachments,
    })
}

/// Normalize an email mailbox and derive its lowercase domain.
fn mailbox(mailbox_text: &str) -> Result<Mailbox, MailError> {
    let normalized_address = mailbox_text.trim().to_ascii_lowercase();
    let (local_part, domain) = normalized_address
        .split_once('@')
        .ok_or_else(|| MailError::InvalidMessage(format!("invalid mailbox {mailbox_text:?}")))?;
    if local_part.is_empty() || domain.is_empty() || domain.contains('@') {
        return Err(MailError::InvalidMessage(format!(
            "invalid mailbox {mailbox_text:?}"
        )));
    }
    let domain = domain.to_owned();
    Ok(Mailbox {
        address: normalized_address,
        domain,
    })
}

/// Replace folded header line breaks with spaces and trim surrounding whitespace.
fn unfold_header(header_value: &str) -> String {
    let mut unfolded_value = String::with_capacity(header_value.len());
    let mut characters = header_value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' && characters.peek() == Some(&'\n') {
            characters.next();
            while matches!(characters.peek(), Some(' ' | '\t')) {
                characters.next();
            }
            unfolded_value.push(' ');
        } else {
            unfolded_value.push(character);
        }
    }
    unfolded_value.trim().to_owned()
}

/// Serve SMTP sessions. The handler runs after valid DATA and must persist the
/// message before returning `Ok(())`; only then does the server acknowledge it.
/// The handler's error is returned to the SMTP peer as a temporary failure.
/// ESMTP parameters are unsupported and receive `501`.
pub async fn serve<Handler, HandlerFuture, HandlerError>(
    listener: TcpListener,
    handler: Handler,
    shutdown: impl Future<Output = ()>,
) -> Result<(), MailError>
where
    Handler: Fn(Message) -> HandlerFuture + Send + Sync + 'static,
    HandlerFuture: Future<Output = Result<(), HandlerError>> + Send + 'static,
    HandlerError: Display,
{
    let handler = Arc::new(handler);
    let mut sessions = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                if sessions.len() >= MAXIMUM_CONNECTION_COUNT {
                    drop(stream);
                    continue;
                }
                let handler = Arc::clone(&handler);
                sessions.spawn(async move {
                    if let Err(error) = mail_protocol_session(stream, handler).await {
                        tracing::debug!(%error, "SMTP session ended");
                    }
                });
            }
        }
    }
    if timeout(SHUTDOWN_DRAIN_TIMEOUT, async {
        while sessions.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        sessions.abort_all();
        while sessions.join_next().await.is_some() {}
    }
    Ok(())
}

/// Run one SMTP conversation and send accepted messages to the persistence handler.
async fn mail_protocol_session<Stream, Handler, HandlerFuture, HandlerError>(
    stream: Stream,
    handler: Arc<Handler>,
) -> Result<(), MailError>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Handler: Fn(Message) -> HandlerFuture + Send + Sync + 'static,
    HandlerFuture: Future<Output = Result<(), HandlerError>> + Send + 'static,
    HandlerError: Display,
{
    let mut reader = BufReader::new(stream);
    write_mail_protocol_response(reader.get_mut(), "220 sandpost local SMTP ready").await?;
    let mut has_mail_from = false;
    let mut envelope_sender: Option<String> = None;
    let mut recipients = Vec::<String>::new();
    loop {
        let command_line = match timeout(
            INPUT_OUTPUT_TIMEOUT,
            read_bounded_protocol_line(&mut reader, MAXIMUM_LINE_SIZE),
        )
        .await
        {
            Ok(Ok(Some(command_line_bytes))) => String::from_utf8(command_line_bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
                return Err(error.into());
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {
                write_mail_protocol_response(reader.get_mut(), "421 timeout").await?;
                return Ok(());
            }
        };
        let command_name = command_line
            .split_ascii_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let argument = command_line.get(command_name.len()..).unwrap_or("").trim();
        match command_name.as_str() {
            "EHLO" | "HELO" => {
                has_mail_from = false;
                envelope_sender = None;
                recipients.clear();
                write_mail_protocol_response(reader.get_mut(), "250 sandpost").await?;
            }
            "NOOP" => write_mail_protocol_response(reader.get_mut(), "250 ok").await?,
            "RSET" => {
                has_mail_from = false;
                envelope_sender = None;
                recipients.clear();
                write_mail_protocol_response(reader.get_mut(), "250 reset").await?;
            }
            "QUIT" => {
                write_mail_protocol_response(reader.get_mut(), "221 bye").await?;
                return Ok(());
            }
            "MAIL" if starts_with_mail_protocol_parameter(argument, "FROM:") => {
                let Some(address) = parse_mail_protocol_path(&argument[5..], true) else {
                    write_mail_protocol_response(
                        reader.get_mut(),
                        "501 invalid sender or unsupported parameter",
                    )
                    .await?;
                    continue;
                };
                has_mail_from = true;
                envelope_sender = address;
                recipients.clear();
                write_mail_protocol_response(reader.get_mut(), "250 sender ok").await?;
            }
            "RCPT" if starts_with_mail_protocol_parameter(argument, "TO:") => {
                if !has_mail_from {
                    write_mail_protocol_response(reader.get_mut(), "503 send MAIL FROM first")
                        .await?;
                    continue;
                }
                if recipients.len() >= MAXIMUM_RECIPIENT_COUNT {
                    write_mail_protocol_response(reader.get_mut(), "452 too many recipients")
                        .await?;
                    continue;
                }
                let Some(Some(address)) = parse_mail_protocol_path(&argument[3..], false) else {
                    write_mail_protocol_response(
                        reader.get_mut(),
                        "501 invalid recipient or unsupported parameter",
                    )
                    .await?;
                    continue;
                };
                recipients.push(address);
                write_mail_protocol_response(reader.get_mut(), "250 recipient ok").await?;
            }
            "DATA" if argument.is_empty() => {
                if !has_mail_from || recipients.is_empty() {
                    write_mail_protocol_response(reader.get_mut(), "503 MAIL and RCPT required")
                        .await?;
                    continue;
                }
                write_mail_protocol_response(reader.get_mut(), "354 end with <CRLF>.<CRLF>")
                    .await?;
                let raw_message = match read_message_data(&mut reader).await {
                    Ok(raw_message) => raw_message,
                    Err(MessageDataError::MessageTooLarge) => {
                        write_mail_protocol_response(
                            reader.get_mut(),
                            "552 message exceeds 10 MiB",
                        )
                        .await?;
                        return Ok(());
                    }
                    Err(MessageDataError::InputOutput(error)) => return Err(error.into()),
                    Err(MessageDataError::ReadTimeout) => {
                        write_mail_protocol_response(reader.get_mut(), "421 timeout").await?;
                        return Ok(());
                    }
                };
                has_mail_from = false;
                let envelope_from = envelope_sender.take();
                let envelope_to = std::mem::take(&mut recipients);
                let received_at = current_unix_timestamp_seconds();
                let parsed = tokio::task::spawn_blocking(move || {
                    parse_message(
                        envelope_from.as_deref(),
                        &envelope_to,
                        raw_message,
                        received_at,
                    )
                })
                .await;
                match parsed {
                    Err(error) => {
                        tracing::error!(%error, "SMTP MIME parser task failed");
                        write_mail_protocol_response(
                            reader.get_mut(),
                            "451 message parsing failed",
                        )
                        .await?;
                    }
                    Ok(Err(error)) => {
                        write_mail_protocol_response(reader.get_mut(), &format!("550 {error}"))
                            .await?;
                    }
                    Ok(Ok(message)) => {
                        match timeout(INPUT_OUTPUT_TIMEOUT, persist(Arc::clone(&handler), message))
                            .await
                        {
                            Ok(Ok(())) => {
                                write_mail_protocol_response(
                                    reader.get_mut(),
                                    "250 message accepted",
                                )
                                .await?
                            }
                            Ok(Err(error)) => {
                                tracing::warn!(%error, "SMTP message persistence failed");
                                write_mail_protocol_response(
                                    reader.get_mut(),
                                    "451 persistence failed",
                                )
                                .await?;
                            }
                            Err(_) => {
                                write_mail_protocol_response(
                                    reader.get_mut(),
                                    "451 persistence timeout",
                                )
                                .await?
                            }
                        }
                    }
                }
            }
            _ => {
                write_mail_protocol_response(
                    reader.get_mut(),
                    "503 command out of sequence or unsupported",
                )
                .await?
            }
        }
    }
}

/// Check whether an SMTP argument starts with the requested parameter name.
fn starts_with_mail_protocol_parameter(argument: &str, parameter: &str) -> bool {
    argument
        .get(..parameter.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(parameter))
}

/// Parse an SMTP angle-bracket path, optionally allowing a null reverse path.
fn parse_mail_protocol_path(path_argument: &str, allow_empty: bool) -> Option<Option<String>> {
    let trimmed_argument = path_argument.trim();
    let path_contents = trimmed_argument.strip_prefix('<')?;
    let closing_bracket = path_contents.find('>')?;
    if !path_contents[closing_bracket + 1..].trim().is_empty() {
        return None;
    }
    let mailbox_path = &path_contents[..closing_bracket];
    if mailbox_path.is_empty() {
        return allow_empty.then_some(None);
    }
    mailbox(mailbox_path)
        .ok()
        .map(|mailbox| Some(mailbox.address))
}

/// Read one CRLF-terminated protocol line while enforcing its byte limit.
async fn read_bounded_protocol_line<Reader: AsyncBufRead + Unpin>(
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

enum MessageDataError {
    MessageTooLarge,
    ReadTimeout,
    InputOutput(std::io::Error),
}

/// Read mail protocol DATA, undo dot-stuffing, and enforce the complete message size limit.
async fn read_message_data<Reader: AsyncBufRead + Unpin>(
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
async fn write_mail_protocol_response<Writer: AsyncWrite + Unpin>(
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

/// Return the current Unix timestamp in whole seconds.
fn current_unix_timestamp_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Convert a handler error into its SMTP-facing display text.
fn format_handler_error(error: impl Display) -> String {
    error.to_string()
}

/// Run the message handler and convert its error into a displayable string.
async fn persist<Handler, HandlerFuture, HandlerError>(
    handler: Arc<Handler>,
    message: Message,
) -> Result<(), String>
where
    Handler: Fn(Message) -> HandlerFuture,
    HandlerFuture: Future<Output = Result<(), HandlerError>>,
    HandlerError: Display,
{
    handler(message).await.map_err(format_handler_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt;

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
        send_mail_protocol_command: &str,
    ) -> String {
        reader
            .get_mut()
            .write_all(format!("{send_mail_protocol_command}\r\n").as_bytes())
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
}
