//! Bounded command framing, synchronizing literals, and rejected-input draining.
use super::{Next, Session, connection::LineRead, wait_for_shutdown};
use crate::{
    backend::ImapBackend,
    limits::MAXIMUM_NON_SYNCHRONIZING_LITERAL,
    protocol::{
        parser::{append_mailbox_prefix, command_prefix},
        response::{ResponseCode, Status},
    },
};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};

/// The result of waiting for one complete command.
pub(super) enum CommandRead {
    /// A complete command with its literals inline.
    Command(Vec<u8>),
    /// The command was refused while it was being received; the response is already queued.
    Refused,
    /// The client closed the connection.
    Closed,
    /// No command arrived in time.
    TimedOut,
    /// The server is shutting down.
    ShuttingDown,
    /// The connection must close after the queued response.
    Fatal,
}

/// The literal announced at the end of a line: its size and whether it is synchronizing.
fn literal_announcement(line: &[u8]) -> Option<(usize, bool)> {
    let body = line.strip_suffix(b"}")?;
    let open = body.iter().rposition(|byte| *byte == b'{')?;
    let mut digits = &body[open + 1..];
    let synchronizing = match digits.strip_suffix(b"+") {
        Some(stripped) => {
            digits = stripped;
            false
        }
        None => true,
    };
    if digits.is_empty() || digits.len() > 10 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let size = std::str::from_utf8(digits).ok()?.parse::<u64>().ok()?;
    Some((usize::try_from(size).ok()?, synchronizing))
}

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// Receive one complete command, requesting synchronizing literals as they are announced.
    pub(super) async fn read_command(&mut self, wait: Duration) -> std::io::Result<CommandRead> {
        let limits = self.limits();
        let mut command = Vec::new();
        loop {
            let first_line = command.is_empty();
            let line_timeout = if first_line {
                wait
            } else {
                limits.input_output_timeout
            };
            let line = {
                let read = self.connection.read_line(limits.maximum_line_size);
                let shutdown = wait_for_shutdown(self.shutdown.clone());
                tokio::select! {
                    biased;
                    result = timeout(line_timeout, read) => match result {
                        Ok(line) => line?,
                        Err(_) if first_line => return Ok(CommandRead::TimedOut),
                        Err(_) => return Err(std::io::ErrorKind::TimedOut.into()),
                    },
                    _ = shutdown, if first_line => return Ok(CommandRead::ShuttingDown),
                }
            };
            let line = match line {
                LineRead::Line(line) => line,
                LineRead::Closed if first_line => return Ok(CommandRead::Closed),
                LineRead::Closed => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                LineRead::TooLong => {
                    let tag = command_prefix(&command).map(|(tag, _)| tag);
                    self.connection.output.status(
                        tag.as_deref(),
                        Status::Bad,
                        None,
                        "Command line too long",
                    );
                    return Ok(CommandRead::Refused);
                }
            };
            if first_line && line.is_empty() {
                continue;
            }
            command.extend_from_slice(&line);
            let Some((size, synchronizing)) = literal_announcement(&line) else {
                if command.len() > limits.maximum_command_size {
                    let tag = command_prefix(&command).map(|(tag, _)| tag);
                    self.connection.output.status(
                        tag.as_deref(),
                        Status::Bad,
                        Some(&ResponseCode::TooBig),
                        "Command too large",
                    );
                    return Ok(CommandRead::Refused);
                }
                return Ok(CommandRead::Command(command));
            };
            let prefix = command_prefix(&command);
            let tag = prefix.as_ref().map(|(tag, _)| tag.clone());
            let is_append = prefix.as_ref().is_some_and(|(_, name)| name == "APPEND");
            if is_append && synchronizing {
                // The message store is read-only: refuse before the client uploads the message.
                let mailbox = append_mailbox_prefix(&command);
                return Ok(
                    match self
                        .refuse_append(tag.as_deref(), mailbox.as_deref())
                        .await?
                    {
                        Next::Continue => CommandRead::Refused,
                        Next::Close => CommandRead::Fatal,
                    },
                );
            }
            let too_large = command.len().saturating_add(size) > limits.maximum_command_size;
            if !synchronizing && size > MAXIMUM_NON_SYNCHRONIZING_LITERAL {
                if size <= limits.maximum_command_size {
                    self.connection.discard_literal(size).await?;
                    self.discard_rest_of_command().await?;
                    self.connection.output.status(
                        tag.as_deref(),
                        Status::Bad,
                        Some(&ResponseCode::TooBig),
                        "Non-synchronizing literals are limited to 4096 octets",
                    );
                    return Ok(CommandRead::Refused);
                }
                self.connection.output.status(
                    None,
                    Status::Bye,
                    Some(&ResponseCode::TooBig),
                    "Literal too large",
                );
                return Ok(CommandRead::Fatal);
            }
            if too_large {
                if synchronizing {
                    self.connection.output.status(
                        tag.as_deref(),
                        Status::Bad,
                        Some(&ResponseCode::TooBig),
                        "Command too large",
                    );
                    return Ok(CommandRead::Refused);
                }
                self.connection.discard_literal(size).await?;
                self.discard_rest_of_command().await?;
                self.connection.output.status(
                    tag.as_deref(),
                    Status::Bad,
                    Some(&ResponseCode::TooBig),
                    "Command too large",
                );
                return Ok(CommandRead::Refused);
            }
            if synchronizing {
                self.connection
                    .output
                    .continuation("Ready for literal data");
                self.connection.flush().await?;
            }
            command.extend_from_slice(b"\r\n");
            self.connection.read_literal(size, &mut command).await?;
            if is_append {
                // A non-synchronizing APPEND literal arrived anyway; finish reading the command.
                self.discard_rest_of_command().await?;
                let mailbox = append_mailbox_prefix(&command);
                return Ok(
                    match self
                        .refuse_append(tag.as_deref(), mailbox.as_deref())
                        .await?
                    {
                        Next::Continue => CommandRead::Refused,
                        Next::Close => CommandRead::Fatal,
                    },
                );
            }
        }
    }

    /// Read and drop the remaining lines and literals of a command that was already refused.
    async fn discard_rest_of_command(&mut self) -> std::io::Result<()> {
        let limits = self.limits();
        loop {
            let line = match timeout(
                limits.input_output_timeout,
                self.connection.read_line(limits.maximum_line_size),
            )
            .await
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))??
            {
                LineRead::Line(line) => line,
                LineRead::TooLong => return Ok(()),
                LineRead::Closed => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            };
            match literal_announcement(&line) {
                Some((size, false)) if size <= limits.maximum_command_size => {
                    self.connection.discard_literal(size).await?;
                }
                Some(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "refused command continued with a literal",
                    ));
                }
                None => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Literal announcements are recognized only at the end of a line.
    #[test]
    fn literal_announcements() {
        assert_eq!(literal_announcement(b"a LOGIN {5}"), Some((5, true)));
        assert_eq!(literal_announcement(b"a LOGIN {5+}"), Some((5, false)));
        assert_eq!(literal_announcement(b"a LOGIN {5} x"), None);
        assert_eq!(literal_announcement(b"a LOGIN {}"), None);
        assert_eq!(literal_announcement(b"a LOGIN {+}"), None);
        assert_eq!(literal_announcement(b"a LOGIN {99999999999}"), None);
        assert_eq!(literal_announcement(b"a LOGIN \"{5}\""), None);
    }
}
