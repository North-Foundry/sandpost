//! Mail protocol commands, transaction state, and ingestion handoff.
use crate::{
    MailError,
    limits::{INPUT_OUTPUT_TIMEOUT, MAXIMUM_LINE_SIZE, MAXIMUM_RECIPIENT_COUNT},
    message_parsing::{normalize_mailbox, parse_message},
    protocol_stream::{
        MessageDataError, read_bounded_protocol_line, read_message_data,
        write_mail_protocol_response,
    },
};
use sandpost_core::Message;
use std::{fmt::Display, future::Future, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite, BufReader},
    time::timeout,
};

/// Run one SMTP conversation and send accepted messages to the persistence handler.
pub(crate) async fn mail_protocol_session<Stream, Handler, HandlerFuture, HandlerError>(
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
pub(crate) fn parse_mail_protocol_path(
    path_argument: &str,
    allow_empty: bool,
) -> Option<Option<String>> {
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
    normalize_mailbox(mailbox_path)
        .ok()
        .map(|mailbox| Some(mailbox.address))
}

/// Return the current Unix timestamp in whole seconds.
fn current_unix_timestamp_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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
    handler(message).await.map_err(|error| error.to_string())
}
