//! Buffered, bounded, cancel-safe I/O for one IMAP connection.
//!
//! Lines are accumulated in connection state rather than in a future's locals, so a pending read
//! can be dropped inside `select!` (for example while idling) without losing input.
use crate::protocol::response::Output;
use crate::tls::SessionStream;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    time::timeout,
};

/// The outcome of waiting for one client line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LineRead {
    /// A complete line without its line break.
    Line(Vec<u8>),
    /// The client closed the connection between lines.
    Closed,
    /// The line exceeded its limit; its remainder was discarded up to the line break.
    TooLong,
}

/// One client connection with its pending input and buffered output.
pub(crate) struct Connection<Stream> {
    reader: BufReader<SessionStream<Stream>>,
    /// Octets of the line being received, kept across cancelled reads.
    pending_line: Vec<u8>,
    /// Set while the rest of an overlong line is being discarded.
    discarding: bool,
    /// Responses waiting to be written.
    pub(crate) output: Output,
    input_output_timeout: Duration,
}

impl<Stream: AsyncRead + AsyncWrite + Unpin> Connection<Stream> {
    /// Wrap a possibly already encrypted stream.
    pub(crate) fn new(stream: SessionStream<Stream>, input_output_timeout: Duration) -> Self {
        Self {
            reader: BufReader::new(stream),
            pending_line: Vec::new(),
            discarding: false,
            output: Output::new(),
            input_output_timeout,
        }
    }

    /// Report whether the connection is protected by TLS.
    pub(crate) fn is_encrypted(&self) -> bool {
        self.reader.get_ref().is_encrypted()
    }

    /// Drop every received octet that has not been consumed yet.
    pub(crate) fn discard_buffered_input(&mut self) {
        let buffered = self.reader.buffer().len();
        self.reader.consume(buffered);
        self.pending_line.clear();
        self.discarding = false;
    }

    /// Read one line of at most `limit` octets excluding its line break.
    ///
    /// Cancel-safe: consumed octets are kept in the connection until the line completes. Both
    /// CRLF and a bare LF end a line.
    pub(crate) async fn read_line(&mut self, limit: usize) -> std::io::Result<LineRead> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                return if self.pending_line.is_empty() && !self.discarding {
                    Ok(LineRead::Closed)
                } else {
                    Err(std::io::ErrorKind::UnexpectedEof.into())
                };
            }
            let (consumed, complete) = match available.iter().position(|byte| *byte == b'\n') {
                Some(position) => (position + 1, true),
                None => (available.len(), false),
            };
            if !self.discarding {
                self.pending_line.extend_from_slice(&available[..consumed]);
            }
            self.reader.consume(consumed);
            if !self.discarding && self.pending_line.len() > limit.saturating_add(2) {
                self.pending_line.clear();
                self.discarding = true;
            }
            if complete {
                if std::mem::take(&mut self.discarding) {
                    return Ok(LineRead::TooLong);
                }
                let mut line = std::mem::take(&mut self.pending_line);
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.len() > limit {
                    return Ok(LineRead::TooLong);
                }
                return Ok(LineRead::Line(line));
            }
        }
    }

    /// Read exactly `length` literal octets into `buffer`, failing if they do not arrive within
    /// the I/O timeout.
    pub(crate) async fn read_literal(
        &mut self,
        length: usize,
        buffer: &mut Vec<u8>,
    ) -> std::io::Result<()> {
        let start = buffer.len();
        buffer.resize(start + length, 0);
        match timeout(
            self.input_output_timeout,
            self.reader.read_exact(&mut buffer[start..]),
        )
        .await
        {
            Ok(result) => result.map(|_| ()),
            Err(_) => Err(std::io::ErrorKind::TimedOut.into()),
        }
    }

    /// Read and discard exactly `length` octets of an unwanted literal.
    pub(crate) async fn discard_literal(&mut self, mut length: usize) -> std::io::Result<()> {
        let mut scratch = [0u8; 8192];
        while length > 0 {
            let chunk = length.min(scratch.len());
            match timeout(
                self.input_output_timeout,
                self.reader.read_exact(&mut scratch[..chunk]),
            )
            .await
            {
                Ok(result) => {
                    result?;
                }
                Err(_) => return Err(std::io::ErrorKind::TimedOut.into()),
            }
            length -= chunk;
        }
        Ok(())
    }

    /// Write and clear buffered responses, failing if the client stops reading.
    pub(crate) async fn flush(&mut self) -> std::io::Result<()> {
        if self.output.is_empty() {
            return Ok(());
        }
        let result = timeout(self.input_output_timeout, async {
            self.reader
                .get_mut()
                .write_all(self.output.as_bytes())
                .await?;
            self.reader.get_mut().flush().await
        })
        .await;
        self.output.clear();
        match result {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "IMAP write timed out",
            )),
        }
    }

    /// Hand the underlying stream to a TLS upgrade and get it back.
    pub(crate) fn stream_mut(&mut self) -> &mut SessionStream<Stream> {
        self.reader.get_mut()
    }

    /// Close the connection, sending a TLS close notification when encrypted.
    pub(crate) async fn shutdown(&mut self) -> std::io::Result<()> {
        match timeout(self.input_output_timeout, self.reader.get_mut().shutdown()).await {
            Ok(result) => result,
            Err(_) => Err(std::io::ErrorKind::TimedOut.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    /// Build a connection over an in-memory pipe and return the client side.
    fn connection() -> (Connection<tokio::io::DuplexStream>, tokio::io::DuplexStream) {
        let (client, server) = tokio::io::duplex(1024);
        (
            Connection::new(SessionStream::Plain(server), Duration::from_secs(1)),
            client,
        )
    }

    /// Lines split across reads are joined; CRLF and LF both end lines.
    #[tokio::test]
    async fn partial_reads_join_into_lines() {
        let (mut connection, mut client) = connection();
        client.write_all(b"a1 NO").await.unwrap();
        let read = tokio::spawn(async move {
            let first = connection.read_line(100).await.unwrap();
            let second = connection.read_line(100).await.unwrap();
            let third = connection.read_line(100).await.unwrap();
            (first, second, third)
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        client.write_all(b"OP\r\na2 LOGOUT\n").await.unwrap();
        drop(client);
        let (first, second, third) = read.await.unwrap();
        assert_eq!(first, LineRead::Line(b"a1 NOOP".to_vec()));
        assert_eq!(second, LineRead::Line(b"a2 LOGOUT".to_vec()));
        assert_eq!(third, LineRead::Closed);
    }

    /// A cancelled read keeps the partial line for the next read.
    #[tokio::test]
    async fn cancelled_reads_keep_partial_input() {
        let (mut connection, mut client) = connection();
        client.write_all(b"DO").await.unwrap();
        let cancelled =
            tokio::time::timeout(Duration::from_millis(20), connection.read_line(100)).await;
        assert!(cancelled.is_err());
        client.write_all(b"NE\r\n").await.unwrap();
        assert_eq!(
            connection.read_line(100).await.unwrap(),
            LineRead::Line(b"DONE".to_vec())
        );
    }

    /// Overlong lines are discarded through their line break and reading resumes after them.
    #[tokio::test]
    async fn overlong_lines_are_discarded() {
        let (mut connection, mut client) = connection();
        let mut input = vec![b'x'; 300];
        input.extend_from_slice(b"\r\nok\r\n");
        client.write_all(&input).await.unwrap();
        assert_eq!(connection.read_line(100).await.unwrap(), LineRead::TooLong);
        assert_eq!(
            connection.read_line(100).await.unwrap(),
            LineRead::Line(b"ok".to_vec())
        );
        client.write_all(&[b'y'; 101]).await.unwrap();
        client.write_all(b"\r\n").await.unwrap();
        assert_eq!(connection.read_line(100).await.unwrap(), LineRead::TooLong);
    }

    /// End of input inside a line is an error, not a clean close.
    #[tokio::test]
    async fn unterminated_input_is_an_error() {
        let (mut connection, mut client) = connection();
        client.write_all(b"partial").await.unwrap();
        drop(client);
        assert!(connection.read_line(100).await.is_err());
    }
}
