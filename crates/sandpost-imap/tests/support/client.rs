//! A minimal IMAP test client that frames responses (including literals) itself and validates
//! every response with the independent `imap-codec` grammar implementation.
use imap_codec::{GreetingCodec, ResponseCodec, decode::Decoder};
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    time::timeout,
};

/// How long a test waits for any single response.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// A client connection over any byte stream.
pub struct TestClient<Stream> {
    pub reader: BufReader<Stream>,
}

impl<Stream: AsyncRead + AsyncWrite + Unpin> TestClient<Stream> {
    /// Wrap a connected stream.
    pub fn new(stream: Stream) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }

    /// Read one complete response, including the octets of any literals it contains.
    ///
    /// Returns `None` when the server closed the connection.
    pub async fn read_raw_response(&mut self) -> Option<Vec<u8>> {
        let mut response = Vec::new();
        loop {
            let mut line = Vec::new();
            let read = timeout(RESPONSE_TIMEOUT, self.reader.read_until(b'\n', &mut line))
                .await
                .expect("response timed out")
                .expect("read failed");
            if read == 0 {
                return if response.is_empty() {
                    None
                } else {
                    panic!(
                        "connection closed inside a response: {:?}",
                        String::from_utf8_lossy(&response)
                    )
                };
            }
            assert!(
                line.ends_with(b"\r\n"),
                "response line without CRLF: {line:?}"
            );
            response.extend_from_slice(&line);
            let content = &line[..line.len() - 2];
            let Some(length) = literal_length(content) else {
                return Some(response);
            };
            let mut literal = vec![0; length];
            timeout(RESPONSE_TIMEOUT, self.reader.read_exact(&mut literal))
                .await
                .expect("literal timed out")
                .expect("literal read failed");
            response.extend_from_slice(&literal);
        }
    }

    /// Read one response, validate it, and return it as text.
    pub async fn read_response(&mut self) -> Option<String> {
        let response = self.read_raw_response().await?;
        validate_response(&response);
        Some(String::from_utf8_lossy(&response).into_owned())
    }

    /// Read and validate the greeting.
    pub async fn greeting(&mut self) -> String {
        let greeting = self.read_raw_response().await.expect("greeting");
        let (rest, _) = GreetingCodec::default()
            .decode(&greeting)
            .unwrap_or_else(|error| {
                panic!(
                    "invalid greeting {:?}: {error:?}",
                    String::from_utf8_lossy(&greeting)
                )
            });
        assert!(rest.is_empty());
        String::from_utf8_lossy(&greeting).into_owned()
    }

    /// Send raw octets.
    pub async fn send(&mut self, octets: &[u8]) {
        self.reader.get_mut().write_all(octets).await.unwrap();
        self.reader.get_mut().flush().await.unwrap();
    }

    /// Send `tag command` and collect every response through the tagged completion.
    pub async fn command(&mut self, tag: &str, command: &str) -> Vec<String> {
        self.send(format!("{tag} {command}\r\n").as_bytes()).await;
        self.until_tagged(tag).await
    }

    /// Collect responses until the one tagged with `tag`.
    pub async fn until_tagged(&mut self, tag: &str) -> Vec<String> {
        let mut responses = Vec::new();
        loop {
            let response = self
                .read_response()
                .await
                .unwrap_or_else(|| panic!("closed before {tag} completed: {responses:#?}"));
            let done = response.starts_with(&format!("{tag} "));
            responses.push(response);
            if done {
                return responses;
            }
        }
    }

    /// Run a command and return only its tagged completion line.
    pub async fn status_of(&mut self, tag: &str, command: &str) -> String {
        self.command(tag, command).await.pop().unwrap()
    }

    /// Log in with the test credentials, asserting success.
    pub async fn login(&mut self) {
        let completion = self
            .status_of(
                "login",
                &format!(
                    "LOGIN {} \"{}\"",
                    super::backend::USERNAME,
                    super::backend::PASSWORD
                ),
            )
            .await;
        assert!(completion.starts_with("login OK"), "{completion}");
    }

    /// Assert that the server closes the connection without further responses.
    pub async fn expect_closed(&mut self) {
        assert_eq!(self.read_response().await, None);
    }
}

/// The length of a literal announced at the end of a response line.
fn literal_length(line: &[u8]) -> Option<usize> {
    let body = line.strip_suffix(b"}")?;
    let open = body.iter().rposition(|byte| *byte == b'{')?;
    std::str::from_utf8(&body[open + 1..]).ok()?.parse().ok()
}

/// Panic unless the bytes are exactly one valid server response.
pub fn validate_response(response: &[u8]) {
    match ResponseCodec::default().decode(response) {
        Ok((rest, _)) => assert!(
            rest.is_empty(),
            "trailing bytes after response {:?}",
            String::from_utf8_lossy(response)
        ),
        Err(error) => panic!(
            "invalid response {:?}: {error:?}",
            String::from_utf8_lossy(response)
        ),
    }
}
