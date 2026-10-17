//! Small synchronous HTTP and server-sent events client for the test runner.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// Errors returned by the local HTTP client.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const HEADER_LIMIT: usize = 64 * 1024;
const BODY_LIMIT: usize = 16 * 1024 * 1024;
const LINE_LIMIT: usize = 64 * 1024;
const TRAILER_LIMIT: usize = 64 * 1024;
const EVENT_LIMIT: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP client for the local test server.
pub struct HttpClient {
    /// Port on the loopback HTTP listener.
    pub port: u16,
    /// Optional bearer token attached to requests.
    pub session_token: Option<String>,
}

/// A fully read, bounded HTTP response.
pub struct Response {
    /// Numeric HTTP status code.
    pub status: u16,
    /// Response media type, without optional parameters.
    pub content_type: String,
    /// Raw response body bytes.
    pub body: Vec<u8>,
}

/// An open server-sent events response.
pub struct EventStream {
    reader: BufReader<TcpStream>,
    timeout_stream: TcpStream,
    framing: Framing,
}

#[derive(Clone, Copy)]
enum Framing {
    Length(u64),
    Chunked { remaining: u64, finished: bool },
    UntilEof,
}

impl HttpClient {
    /// Sends one HTTP/1.1 request to the loopback server.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Response> {
        if [method, path]
            .iter()
            .any(|value| value.contains(['\r', '\n']))
            || self
                .session_token
                .as_deref()
                .is_some_and(|token| token.contains(['\r', '\n']))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid HTTP header or request line",
            )
            .into());
        }
        let body_bytes = body
            .map(serde_json::to_vec)
            .transpose()?
            .unwrap_or_default();
        if body_bytes.len() > BODY_LIMIT {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "request body exceeds limit").into(),
            );
        }
        let mut stream = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.port)),
            REQUEST_TIMEOUT,
        )?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
            self.port
        )?;
        if body.is_some() {
            stream.write_all(b"Content-Type: application/json\r\n")?;
        }
        if let Some(token) = &self.session_token {
            write!(stream, "Authorization: Bearer {token}\r\n")?;
        }
        write!(stream, "Content-Length: {}\r\n\r\n", body_bytes.len())?;
        stream.write_all(&body_bytes)?;
        let mut reader = BufReader::new(stream);
        let (status, content_type, framing) = read_response_headers(&mut reader)?;
        let response_body = read_complete_body(&mut reader, framing)?;
        Ok(Response {
            status,
            content_type,
            body: response_body,
        })
    }

    /// Fetches a successful JSON response and decodes its body.
    pub fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        self.json_response("GET", path, None)
    }

    /// Posts JSON and decodes a successful JSON response.
    pub fn post_json(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
        self.json_response("POST", path, Some(&body))
    }

    /// Opens the authorized server-sent events endpoint.
    pub fn events(&self) -> Result<EventStream> {
        let mut stream = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.port)),
            REQUEST_TIMEOUT,
        )?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        write!(
            stream,
            "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
            self.port
        )?;
        if let Some(token) = &self.session_token {
            if token.contains(['\r', '\n']) {
                return Err(
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid session token").into(),
                );
            }
            write!(stream, "Authorization: Bearer {token}\r\n")?;
        }
        stream.write_all(b"Accept: text/event-stream\r\n\r\n")?;
        let timeout_stream = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        let (status, content_type, framing) = read_response_headers(&mut reader)?;
        if status != 200 || content_type != "text/event-stream" {
            return Err(io::Error::other(format!(
                "expected HTTP 200 text/event-stream, got {status} {content_type}"
            ))
            .into());
        }
        Ok(EventStream {
            reader,
            timeout_stream,
            framing,
        })
    }

    /// Decodes JSON while including the HTTP status and response body in failures.
    fn json_response(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let response = self.request(method, path, body)?;
        if !(200..300).contains(&response.status) {
            return Err(io::Error::other(format!(
                "{method} {path} returned HTTP {}: {}",
                response.status,
                String::from_utf8_lossy(&response.body)
            ))
            .into());
        }
        Ok(serde_json::from_slice(&response.body)?)
    }
}

impl EventStream {
    /// Reads one dispatched SSE frame, skipping comments and frames without data.
    pub fn read_event(&mut self) -> Result<String> {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        let mut fields = Vec::new();
        let mut event_bytes = 0;
        loop {
            let Some(line) = read_body_line(
                &mut self.reader,
                &mut self.framing,
                &self.timeout_stream,
                deadline,
            )?
            else {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "SSE stream closed before the expected event",
                )
                .into());
            };
            if let Some(event) = process_sse_line(&line, &mut fields, &mut event_bytes)? {
                return Ok(event);
            }
        }
    }
}

/// Adds one SSE line to the current frame and returns data frames at blank lines.
fn process_sse_line(
    raw_line: &str,
    fields: &mut Vec<String>,
    event_bytes: &mut usize,
) -> Result<Option<String>> {
    let line = raw_line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        if fields.iter().any(|field| field.starts_with("data:")) {
            return Ok(Some(fields.join("\n")));
        }
        fields.clear();
        *event_bytes = 0;
    } else if !line.starts_with(':') {
        *event_bytes += line.len();
        if *event_bytes > EVENT_LIMIT {
            return Err(io::Error::other("SSE event exceeded limit").into());
        }
        fields.push(line.to_owned());
    }
    Ok(None)
}

/// Reads bounded HTTP headers and chooses the response body framing.
fn read_response_headers(reader: &mut BufReader<TcpStream>) -> Result<(u16, String, Framing)> {
    let mut total_bytes = 0;
    let status_line = read_header_line(reader, &mut total_bytes)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| io::Error::other("invalid HTTP status line"))?
        .parse()?;
    let mut content_type = String::new();
    let mut content_length = None;
    let mut chunked = false;
    loop {
        let line = read_header_line(reader, &mut total_bytes)?;
        if line == "\r\n" || line == "\n" {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::other("invalid HTTP header"))?;
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value
                .trim()
                .split(';')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
        } else if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(value.trim().parse::<u64>()?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"));
        }
    }
    let framing = if chunked {
        Framing::Chunked {
            remaining: 0,
            finished: false,
        }
    } else if let Some(length) = content_length {
        Framing::Length(length)
    } else {
        Framing::UntilEof
    };
    Ok((status, content_type, framing))
}

/// Reads one CRLF-terminated HTTP header line within the aggregate header bound.
fn read_header_line(reader: &mut BufReader<TcpStream>, total_bytes: &mut usize) -> Result<String> {
    let mut line = Vec::new();
    let count = reader
        .take((HEADER_LIMIT + 1) as u64)
        .read_until(b'\n', &mut line)?;
    *total_bytes = total_bytes.saturating_add(count);
    if count == 0 || *total_bytes > HEADER_LIMIT {
        return Err(io::Error::other("HTTP headers exceeded limit or ended early").into());
    }
    Ok(String::from_utf8(line)?)
}

/// Reads a complete bounded body using its declared HTTP framing.
fn read_complete_body(reader: &mut BufReader<TcpStream>, mut framing: Framing) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(byte) = read_body_byte(reader, &mut framing)? {
        if body.len() == BODY_LIMIT {
            return Err(io::Error::other("HTTP response body exceeded limit").into());
        }
        body.push(byte);
    }
    Ok(body)
}

/// Reads the next decoded body byte, advancing chunk boundaries and trailers.
fn read_body_byte<R: BufRead>(reader: &mut R, framing: &mut Framing) -> io::Result<Option<u8>> {
    match framing {
        Framing::Length(remaining) => {
            if *remaining == 0 {
                return Ok(None);
            }
            let byte =
                reader.fill_buf()?.first().copied().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "short HTTP body")
                })?;
            reader.consume(1);
            *remaining -= 1;
            Ok(Some(byte))
        }
        Framing::UntilEof => {
            let byte = reader.fill_buf()?.first().copied();
            if byte.is_some() {
                reader.consume(1);
            }
            Ok(byte)
        }
        Framing::Chunked {
            remaining,
            finished,
        } => loop {
            if *finished {
                return Ok(None);
            }
            if *remaining > 0 {
                let byte = reader.fill_buf()?.first().copied().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "short chunked body")
                })?;
                reader.consume(1);
                *remaining -= 1;
                if *remaining == 0 {
                    let mut ending = [0; 2];
                    reader.read_exact(&mut ending)?;
                    if ending != *b"\r\n" {
                        return Err(io::Error::other("invalid chunk ending"));
                    }
                }
                return Ok(Some(byte));
            }
            let mut size_line = Vec::new();
            reader
                .take((LINE_LIMIT + 1) as u64)
                .read_until(b'\n', &mut size_line)?;
            if size_line.len() > LINE_LIMIT {
                return Err(io::Error::other("chunk size line exceeded limit"));
            }
            let size_text = std::str::from_utf8(&size_line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .trim()
                .split(';')
                .next()
                .unwrap_or_default();
            *remaining = u64::from_str_radix(size_text, 16).map_err(io::Error::other)?;
            if *remaining == 0 {
                let mut trailer_bytes = 0_usize;
                loop {
                    let mut trailer = Vec::new();
                    let count = reader
                        .take((LINE_LIMIT + 1) as u64)
                        .read_until(b'\n', &mut trailer)?;
                    if count == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated chunk trailers",
                        ));
                    }
                    trailer_bytes = trailer_bytes.saturating_add(count);
                    if trailer.len() > LINE_LIMIT || trailer_bytes > TRAILER_LIMIT {
                        return Err(io::Error::other("chunk trailer exceeded limit"));
                    }
                    if trailer == b"\r\n" || trailer == b"\n" {
                        break;
                    }
                }
                *finished = true;
            }
        },
    }
}

/// Reads one bounded UTF-8 line from the decoded HTTP body.
fn read_body_line(
    reader: &mut BufReader<TcpStream>,
    framing: &mut Framing,
    timeout_stream: &TcpStream,
    deadline: Instant,
) -> Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for an SSE event",
            )
            .into());
        }
        timeout_stream.set_read_timeout(Some(remaining))?;
        let Some(byte) = read_body_byte(reader, framing)? else {
            break;
        };
        if line.len() == LINE_LIMIT {
            return Err(io::Error::other("SSE line exceeded limit").into());
        }
        line.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    if line.is_empty() {
        Ok(None)
    } else {
        Ok(Some(String::from_utf8(line)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes arbitrary bytes from a chunked body, including chunk extensions.
    #[test]
    fn decodes_chunked_binary_body() {
        let mut reader = io::BufReader::new(&b"2\r\n\x00\xff\r\n3;v=1\r\nabc\r\n0\r\n\r\n"[..]);
        let mut framing = Framing::Chunked {
            remaining: 0,
            finished: false,
        };
        let mut bytes = Vec::new();
        while let Some(byte) = read_body_byte(&mut reader, &mut framing).unwrap() {
            bytes.push(byte);
        }
        assert_eq!(bytes, [0, 255, b'a', b'b', b'c']);
    }

    /// Rejects a zero chunk whose trailer section ends before its blank line.
    #[test]
    fn rejects_truncated_chunk_trailers() {
        let mut reader = io::BufReader::new(&b"0\r\nX-Trailer: value\r\n"[..]);
        let mut framing = Framing::Chunked {
            remaining: 0,
            finished: false,
        };
        let error = read_body_byte(&mut reader, &mut framing).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    /// Bounds total trailer bytes even when each individual trailer line is short.
    #[test]
    fn bounds_aggregate_chunk_trailers() {
        let mut body = b"0\r\n".to_vec();
        for _ in 0..=(TRAILER_LIMIT / 2) {
            body.extend_from_slice(b"x\n");
        }
        let mut reader = io::BufReader::new(body.as_slice());
        let mut framing = Framing::Chunked {
            remaining: 0,
            finished: false,
        };
        let error = read_body_byte(&mut reader, &mut framing).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    /// Skips heartbeat comments and empty frames before returning SSE data.
    #[test]
    fn skips_sse_heartbeats() {
        let mut reader = io::Cursor::new(b": heartbeat\n\n\ndata: x\nevent: ready\n\n");
        let mut fields = Vec::new();
        let mut event_bytes = 0;
        let mut event = None;
        while event.is_none() {
            let mut raw_line = String::new();
            reader.read_line(&mut raw_line).unwrap();
            event = process_sse_line(&raw_line, &mut fields, &mut event_bytes).unwrap();
        }
        assert_eq!(event.unwrap(), "data: x\nevent: ready");
    }
}
