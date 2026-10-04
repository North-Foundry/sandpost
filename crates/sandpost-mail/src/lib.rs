//! MIME normalization and a bounded local SMTP catcher.

use mail_parser::{Address, MessageParser, MimeHeaders};
use sandpost_core::{Attachment, Mailbox, Message, MessageFacts, MessageId};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Display, future::Future, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpListener,
    task::JoinSet,
    time::timeout,
};

const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
const MAX_LINE_SIZE: usize = 1000;
const MAX_RECIPIENTS: usize = 100;
const MAX_CONNECTIONS: usize = 32;
const IO_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum MailError {
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("SMTP I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Parse once and normalize all mailbox local-parts and domains to lowercase.
pub fn parse_message(
    envelope_from: Option<&str>,
    envelope_to: &[String],
    raw: Vec<u8>,
    received_at: i64,
) -> Result<Message, MailError> {
    if raw.len() > MAX_MESSAGE_SIZE {
        return Err(MailError::InvalidMessage("message exceeds 10 MiB".into()));
    }
    let parsed = MessageParser::new()
        .with_mime_headers()
        .with_address_headers()
        .with_message_ids()
        .default_header_text()
        .parse(&raw)
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
        .filter_map(|a| a.address())
        .map(mailbox)
        .collect::<Result<_, _>>()?;
    let to = parsed
        .all_to()
        .flat_map(Address::iter)
        .filter_map(|a| a.address())
        .map(mailbox)
        .collect::<Result<_, _>>()?;
    let cc = parsed
        .all_cc()
        .flat_map(Address::iter)
        .filter_map(|a| a.address())
        .map(mailbox)
        .collect::<Result<_, _>>()?;

    let mut headers = BTreeMap::<String, Vec<String>>::new();
    for (index, (name, raw_value)) in parsed.headers_raw().enumerate() {
        let value = parsed
            .headers()
            .get(index)
            .and_then(|header| header.value().as_text())
            .unwrap_or(raw_value);
        headers
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(unfold_header(value));
    }
    let text = (0..parsed.text_body_count())
        .filter_map(|index| parsed.body_text(index))
        .map(|part| part.into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let html = (0..parsed.html_body_count())
        .filter_map(|index| parsed.body_html(index))
        .map(|part| part.into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let attachments = parsed
        .attachments()
        .map(|part| {
            let bytes = part.contents();
            let content_type = part
                .content_type()
                .map(|ct| match ct.subtype() {
                    Some(subtype) => format!("{}/{}", ct.ctype(), subtype),
                    None => ct.ctype().to_owned(),
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            Attachment {
                filename: part.attachment_name().map(str::to_owned),
                content_type,
                size: bytes.len() as u64,
                content_hash: format!("{:x}", Sha256::digest(bytes)),
            }
        })
        .collect::<Vec<_>>();

    Ok(Message {
        id: MessageId::new(),
        facts: MessageFacts {
            envelope_from,
            envelope_to,
            from,
            to,
            cc,
            subject: parsed.subject().unwrap_or_default().to_owned(),
            text,
            html,
            message_id: parsed.message_id().map(str::to_owned),
            received_at,
            size: raw.len() as u64,
            attachment_count: attachments.len() as u64,
            headers,
        },
        raw_mime: raw,
        attachments,
    })
}

fn mailbox(input: &str) -> Result<Mailbox, MailError> {
    let address = input.trim().to_ascii_lowercase();
    let (local, domain) = address
        .split_once('@')
        .ok_or_else(|| MailError::InvalidMessage(format!("invalid mailbox {input:?}")))?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return Err(MailError::InvalidMessage(format!(
            "invalid mailbox {input:?}"
        )));
    }
    let domain = domain.to_owned();
    Ok(Mailbox { address, domain })
}

fn unfold_header(value: &str) -> String {
    let mut unfolded = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
            while matches!(chars.peek(), Some(' ' | '\t')) {
                chars.next();
            }
            unfolded.push(' ');
        } else {
            unfolded.push(ch);
        }
    }
    unfolded.trim().to_owned()
}

/// Serve SMTP sessions. The handler runs after valid DATA and must persist the
/// message before returning `Ok(())`; only then does the server acknowledge it.
/// The handler's error is returned to the SMTP peer as a temporary failure.
/// ESMTP parameters are unsupported and receive `501`.
pub async fn serve<F, Fut, E>(
    listener: TcpListener,
    handler: F,
    shutdown: impl Future<Output = ()>,
) -> Result<(), MailError>
where
    F: Fn(Message) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
    E: Display,
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
                if sessions.len() >= MAX_CONNECTIONS {
                    drop(stream);
                    continue;
                }
                let handler = Arc::clone(&handler);
                sessions.spawn(async move {
                    if let Err(error) = session(stream, handler).await {
                        tracing::debug!(%error, "SMTP session ended");
                    }
                });
            }
        }
    }
    if timeout(SHUTDOWN_DRAIN, async {
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

async fn session<S, F, Fut, E>(stream: S, handler: Arc<F>) -> Result<(), MailError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: Fn(Message) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
    E: Display,
{
    let mut reader = BufReader::new(stream);
    reply(reader.get_mut(), "220 sandpost local SMTP ready").await?;
    let mut has_mail_from = false;
    let mut mail_from: Option<String> = None;
    let mut recipients = Vec::<String>::new();
    loop {
        let line = match timeout(IO_TIMEOUT, read_line_bounded(&mut reader, MAX_LINE_SIZE)).await {
            Ok(Ok(Some(line))) => String::from_utf8(line)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
                return Err(error.into());
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {
                reply(reader.get_mut(), "421 timeout").await?;
                return Ok(());
            }
        };
        let command = line
            .split_ascii_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let argument = line.get(command.len()..).unwrap_or("").trim();
        match command.as_str() {
            "EHLO" | "HELO" => {
                has_mail_from = false;
                mail_from = None;
                recipients.clear();
                reply(reader.get_mut(), "250 sandpost").await?;
            }
            "NOOP" => reply(reader.get_mut(), "250 ok").await?,
            "RSET" => {
                has_mail_from = false;
                mail_from = None;
                recipients.clear();
                reply(reader.get_mut(), "250 reset").await?;
            }
            "QUIT" => {
                reply(reader.get_mut(), "221 bye").await?;
                return Ok(());
            }
            "MAIL" if starts_parameter(argument, "FROM:") => {
                let Some(address) = smtp_path(&argument[5..], true) else {
                    reply(
                        reader.get_mut(),
                        "501 invalid sender or unsupported parameter",
                    )
                    .await?;
                    continue;
                };
                has_mail_from = true;
                mail_from = address;
                recipients.clear();
                reply(reader.get_mut(), "250 sender ok").await?;
            }
            "RCPT" if starts_parameter(argument, "TO:") => {
                if !has_mail_from {
                    reply(reader.get_mut(), "503 send MAIL FROM first").await?;
                    continue;
                }
                if recipients.len() >= MAX_RECIPIENTS {
                    reply(reader.get_mut(), "452 too many recipients").await?;
                    continue;
                }
                let Some(Some(address)) = smtp_path(&argument[3..], false) else {
                    reply(
                        reader.get_mut(),
                        "501 invalid recipient or unsupported parameter",
                    )
                    .await?;
                    continue;
                };
                recipients.push(address);
                reply(reader.get_mut(), "250 recipient ok").await?;
            }
            "DATA" if argument.is_empty() => {
                if !has_mail_from || recipients.is_empty() {
                    reply(reader.get_mut(), "503 MAIL and RCPT required").await?;
                    continue;
                }
                reply(reader.get_mut(), "354 end with <CRLF>.<CRLF>").await?;
                let raw = match read_data(&mut reader).await {
                    Ok(raw) => raw,
                    Err(DataError::TooLarge) => {
                        reply(reader.get_mut(), "552 message exceeds 10 MiB").await?;
                        return Ok(());
                    }
                    Err(DataError::Io(error)) => return Err(error.into()),
                    Err(DataError::Timeout) => {
                        reply(reader.get_mut(), "421 timeout").await?;
                        return Ok(());
                    }
                };
                has_mail_from = false;
                let envelope_from = mail_from.take();
                let envelope_to = std::mem::take(&mut recipients);
                let received_at = now_seconds();
                let parsed = tokio::task::spawn_blocking(move || {
                    parse_message(envelope_from.as_deref(), &envelope_to, raw, received_at)
                })
                .await;
                match parsed {
                    Err(error) => {
                        tracing::error!(%error, "SMTP MIME parser task failed");
                        reply(reader.get_mut(), "451 message parsing failed").await?;
                    }
                    Ok(Err(error)) => {
                        reply(reader.get_mut(), &format!("550 {error}")).await?;
                    }
                    Ok(Ok(message)) => {
                        match timeout(IO_TIMEOUT, persist(Arc::clone(&handler), message)).await {
                            Ok(Ok(())) => reply(reader.get_mut(), "250 message accepted").await?,
                            Ok(Err(error)) => {
                                tracing::warn!(%error, "SMTP message persistence failed");
                                reply(reader.get_mut(), "451 persistence failed").await?;
                            }
                            Err(_) => reply(reader.get_mut(), "451 persistence timeout").await?,
                        }
                    }
                }
            }
            _ => {
                reply(
                    reader.get_mut(),
                    "503 command out of sequence or unsupported",
                )
                .await?
            }
        }
    }
}

fn starts_parameter(argument: &str, parameter: &str) -> bool {
    argument
        .get(..parameter.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(parameter))
}

fn smtp_path(input: &str, allow_empty: bool) -> Option<Option<String>> {
    let input = input.trim();
    let path = input.strip_prefix('<')?;
    let close = path.find('>')?;
    if !path[close + 1..].trim().is_empty() {
        return None;
    }
    let path = &path[..close];
    if path.is_empty() {
        return allow_empty.then_some(None);
    }
    mailbox(path).ok().map(|mailbox| Some(mailbox.address))
}

async fn read_line_bounded<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::with_capacity(128);
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unterminated line",
                ))
            };
        }
        let count = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(count) > limit {
            let discard = (limit - bytes.len() + 1).min(available.len());
            reader.consume(discard);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line too long",
            ));
        }
        let complete = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if complete {
            if bytes.len() < 2 || bytes[bytes.len() - 2] != b'\r' {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing CRLF",
                ));
            }
            bytes.truncate(bytes.len() - 2);
            return Ok(Some(bytes));
        }
    }
}

enum DataError {
    TooLarge,
    Timeout,
    Io(std::io::Error),
}

async fn read_data<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, DataError> {
    let mut raw = Vec::new();
    loop {
        let line = match timeout(IO_TIMEOUT, read_line_bounded(reader, MAX_LINE_SIZE)).await {
            Ok(result) => result.map_err(DataError::Io)?.ok_or_else(|| {
                DataError::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
            })?,
            Err(_) => return Err(DataError::Timeout),
        };
        if line == b"." {
            return Ok(raw);
        }
        let line = if line.starts_with(b"..") {
            &line[1..]
        } else {
            &line[..]
        };
        if raw.len().saturating_add(line.len() + 2) > MAX_MESSAGE_SIZE {
            return Err(DataError::TooLarge);
        }
        raw.extend_from_slice(line);
        raw.extend_from_slice(b"\r\n");
    }
}

async fn reply<W: AsyncWrite + Unpin>(stream: &mut W, text: &str) -> std::io::Result<()> {
    match timeout(IO_TIMEOUT, async {
        stream.write_all(text.as_bytes()).await?;
        stream.write_all(b"\r\n").await
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

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn display_error(error: impl Display) -> String {
    error.to_string()
}

async fn persist<F, Fut, E>(handler: Arc<F>, message: Message) -> Result<(), String>
where
    F: Fn(Message) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    E: Display,
{
    handler(message).await.map_err(display_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt;

    async fn client<F, Fut>(handler: F) -> tokio::io::BufReader<tokio::io::DuplexStream>
    where
        F: Fn(Message) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), std::io::Error>> + Send + 'static,
    {
        let (client, server) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let _ = session(server, Arc::new(handler)).await;
        });
        tokio::io::BufReader::new(client)
    }

    async fn line(reader: &mut tokio::io::BufReader<tokio::io::DuplexStream>) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        line
    }

    async fn command(
        reader: &mut tokio::io::BufReader<tokio::io::DuplexStream>,
        command: &str,
    ) -> String {
        reader
            .get_mut()
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .unwrap();
        line(reader).await
    }

    #[tokio::test]
    async fn null_sender_and_binary_data_are_preserved() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let mut reader = client(move |message| {
            let tx = tx.clone();
            async move { tx.send(message).await.map_err(std::io::Error::other) }
        })
        .await;
        assert!(line(&mut reader).await.starts_with("220"));
        assert!(command(&mut reader, "EHLO local").await.starts_with("250"));
        assert!(
            command(&mut reader, "RCPT TO:<dest@example.com>")
                .await
                .starts_with("503")
        );
        assert!(
            command(&mut reader, "MAIL FROM:<> SIZE=4")
                .await
                .starts_with("501")
        );
        assert!(
            command(&mut reader, "MAIL FROM:<>")
                .await
                .starts_with("250")
        );
        assert!(
            command(&mut reader, "RCPT TO:<dest@example.com>")
                .await
                .starts_with("250")
        );
        assert!(command(&mut reader, "RSET").await.starts_with("250"));
        assert!(command(&mut reader, "DATA").await.starts_with("503"));
        assert!(
            command(&mut reader, "MAIL FROM:<>")
                .await
                .starts_with("250")
        );
        assert!(
            command(&mut reader, "RCPT TO:<dest@example.com>")
                .await
                .starts_with("250")
        );
        assert!(command(&mut reader, "DATA").await.starts_with("354"));
        let mut wire = b"From: a@example.com\r\nSubject: Dot\r\n\r\n..dot\r\n".to_vec();
        wire.extend_from_slice(b"\xff\0\r\n.\r\n");
        reader.get_mut().write_all(&wire).await.unwrap();
        assert!(line(&mut reader).await.starts_with("250"));
        let message = rx.recv().await.unwrap();
        assert!(message.facts.envelope_from.is_none());
        assert_eq!(
            message.raw_mime,
            b"From: a@example.com\r\nSubject: Dot\r\n\r\n.dot\r\n\xff\0\r\n"
        );
        assert_eq!(message.facts.envelope_to[0].address, "dest@example.com");
    }

    #[tokio::test]
    async fn data_limit_is_enforced() {
        let mut reader = client(|_| async { Ok::<_, std::io::Error>(()) }).await;
        let _ = line(&mut reader).await;
        let _ = command(&mut reader, "EHLO local").await;
        let _ = command(&mut reader, "MAIL FROM:<sender@example.com>").await;
        let _ = command(&mut reader, "RCPT TO:<dest@example.com>").await;
        assert!(command(&mut reader, "DATA").await.starts_with("354"));
        let mut input = Vec::with_capacity(MAX_MESSAGE_SIZE + 64);
        for _ in 0..10_500 {
            input.extend_from_slice(&[b'x'; 998]);
            input.extend_from_slice(b"\r\n");
        }
        input.extend_from_slice(b".\r\n");
        let _ = reader.get_mut().write_all(&input).await;
        assert!(line(&mut reader).await.starts_with("552"));
    }

    #[tokio::test]
    async fn persistence_failure_and_success_both_reset_the_transaction() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler_calls = Arc::clone(&calls);
        let mut reader = client(move |_| {
            let call = handler_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if call == 0 {
                    Err(std::io::Error::other("storage unavailable"))
                } else {
                    Ok(())
                }
            }
        })
        .await;
        let _ = line(&mut reader).await;
        let _ = command(&mut reader, "EHLO local").await;

        assert!(
            command(&mut reader, "MAIL FROM:<sender@example.com>")
                .await
                .starts_with("250")
        );
        assert!(
            command(&mut reader, "RCPT TO:<dest@example.com>")
                .await
                .starts_with("250")
        );
        assert!(command(&mut reader, "DATA").await.starts_with("354"));
        reader
            .get_mut()
            .write_all(b"Subject: fail\r\n\r\nbody\r\n.\r\n")
            .await
            .unwrap();
        assert!(line(&mut reader).await.starts_with("451"));
        assert!(command(&mut reader, "DATA").await.starts_with("503"));

        assert!(
            command(&mut reader, "MAIL FROM:<sender@example.com>")
                .await
                .starts_with("250")
        );
        assert!(
            command(&mut reader, "RCPT TO:<dest@example.com>")
                .await
                .starts_with("250")
        );
        assert!(command(&mut reader, "DATA").await.starts_with("354"));
        reader
            .get_mut()
            .write_all(b"Subject: success\r\n\r\nbody\r\n.\r\n")
            .await
            .unwrap();
        assert!(line(&mut reader).await.starts_with("250"));
        assert!(command(&mut reader, "DATA").await.starts_with("503"));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn oversized_command_closes_without_reading_trailing_commands() {
        let mut reader = client(|_| async { Ok::<_, std::io::Error>(()) }).await;
        let _ = line(&mut reader).await;
        let mut wire = vec![b'X'; MAX_LINE_SIZE + 1];
        wire.extend_from_slice(b"\r\nNOOP\r\n");
        reader.get_mut().write_all(&wire).await.unwrap();
        let mut response = String::new();
        assert_eq!(reader.read_line(&mut response).await.unwrap(), 0);
    }
}
