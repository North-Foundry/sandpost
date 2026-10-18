//! Persistent SMTP sessions shared by smoke checks and synthetic mail bursts.

use std::error::Error;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SMTP_LINE_BYTES: usize = 64 * 1024;
const MAX_SMTP_REPLY_BYTES: usize = 1024 * 1024;

type Result<T> = crate::Result<T>;

/// A persistent SMTP connection with bounded network operations.
pub struct SmtpClient {
    stream: BufReader<TcpStream>,
}

impl SmtpClient {
    /// Connect, consume the greeting, issue EHLO, and optionally authenticate once.
    pub fn connect(host: &str, port: u16, credentials: Option<(&str, &str)>) -> Result<Self> {
        if let Some((username, password)) = credentials {
            validate_smtp_text("username", username, true, false)?;
            validate_smtp_text("password", password, true, false)?;
        }
        let stream = connect(host, port)?;
        let mut client = Self {
            stream: BufReader::new(stream),
        };
        client.expect_code(220)?;
        client.command("EHLO smoke.test", 250)?;
        if let Some((username, password)) = credentials {
            client.command("AUTH LOGIN", 334)?;
            client.command(&base64(username.as_bytes()), 334)?;
            client.command(&base64(password.as_bytes()), 235)?;
        }
        Ok(client)
    }

    /// Send a message over the existing session and require the final DATA reply to be 250.
    pub fn send(&mut self, sender: &str, recipient: &str, message: &[u8]) -> Result<()> {
        validate_smtp_text("sender", sender, true, true)?;
        validate_smtp_text("recipient", recipient, false, true)?;
        self.command(&format!("MAIL FROM:<{sender}>"), 250)?;
        self.command(&format!("RCPT TO:<{recipient}>"), 250)?;
        self.command("DATA", 354)?;
        self.write_message(message)?;
        self.expect_code(250)?;
        Ok(())
    }

    /// Send QUIT and require the server's 221 completion.
    pub fn quit(&mut self) -> Result<()> {
        self.command("QUIT", 221)?;
        Ok(())
    }

    /// Write one SMTP command and require its expected reply code.
    fn command(&mut self, command: &str, expected: u16) -> Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(command.len() + 2);
        bytes.extend_from_slice(command.as_bytes());
        bytes.extend_from_slice(b"\r\n");
        self.stream.get_mut().write_all(&bytes)?;
        self.expect_code(expected)
    }

    /// Read a bounded SMTP multiline reply and require its expected code.
    fn expect_code(&mut self, expected: u16) -> Result<Vec<u8>> {
        let mut reply = Vec::new();
        loop {
            let line = read_smtp_line(&mut self.stream)?;
            if line.len() < 4 || !line[..3].iter().all(u8::is_ascii_digit) {
                return Err(error(format!(
                    "malformed SMTP reply: {}",
                    String::from_utf8_lossy(&line)
                )));
            }
            let code = std::str::from_utf8(&line[..3])?.parse::<u16>()?;
            if code != expected {
                return Err(error(format!(
                    "SMTP reply {code}, expected {expected}: {}",
                    String::from_utf8_lossy(&line)
                )));
            }
            let complete = line[3] == b' ';
            if line[3] != b' ' && line[3] != b'-' {
                return Err(error(format!(
                    "malformed SMTP reply: {}",
                    String::from_utf8_lossy(&line)
                )));
            }
            if reply.len().saturating_add(line.len()) > MAX_SMTP_REPLY_BYTES {
                return Err(error("SMTP reply exceeded the size limit"));
            }
            reply.extend_from_slice(&line);
            if complete {
                return Ok(reply);
            }
        }
    }

    /// Write a message with SMTP dot transparency and the DATA terminator.
    fn write_message(&mut self, message: &[u8]) -> Result<()> {
        let mut data = Vec::with_capacity(message.len() + 5);
        let mut line_start = true;
        for byte in message {
            if line_start && *byte == b'.' {
                data.push(b'.');
            }
            data.push(*byte);
            line_start = *byte == b'\n';
        }
        if !message.ends_with(b"\r\n") {
            data.extend_from_slice(b"\r\n");
        }
        data.extend_from_slice(b".\r\n");
        self.stream.get_mut().write_all(&data)?;
        Ok(())
    }
}

/// Connect and verify that LOGIN authentication fails with reply code 535.
pub fn reject_password(port: u16, username: &str) -> Result<()> {
    validate_smtp_text("username", username, true, false)?;
    let stream = connect("127.0.0.1", port)?;
    let mut client = SmtpClient {
        stream: BufReader::new(stream),
    };
    client.expect_code(220)?;
    client.command("EHLO smoke.test", 250)?;
    client.command("AUTH LOGIN", 334)?;
    client.command(&base64(username.as_bytes()), 334)?;
    client.command(&base64(b"not the generated password"), 535)?;
    Ok(())
}

/// Resolve all host addresses and try each with a five-second connect timeout.
fn connect(host: &str, port: u16) -> Result<TcpStream> {
    let addresses = (host, port).to_socket_addrs()?;
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, IO_TIMEOUT) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(IO_TIMEOUT))?;
                stream.set_write_timeout(Some(IO_TIMEOUT))?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "host resolved to no addresses"))
        .into())
}

/// Reject protocol control bytes before writing SMTP commands.
fn validate_smtp_text(name: &str, value: &str, allow_empty: bool, is_envelope: bool) -> Result<()> {
    if (!allow_empty && value.is_empty())
        || value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0))
        || (is_envelope && (value.contains('<') || value.contains('>')))
    {
        return Err(error(format!("invalid SMTP {name}")));
    }
    Ok(())
}

/// Read one newline-terminated SMTP line without exceeding its byte limit.
fn read_smtp_line(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "SMTP connection closed",
                ))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete SMTP line",
                ))
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(take) > MAX_SMTP_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SMTP line exceeded the size limit",
            ));
        }
        let complete = available[take - 1] == b'\n';
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if complete {
            return Ok(line);
        }
    }
}

/// Encode bytes as unwrapped standard Base64 for SMTP AUTH LOGIN.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 3) << 4) | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[(((second & 15) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

/// Construct a boxed diagnostic from a message.
fn error(message: impl Into<String>) -> Box<dyn Error> {
    message.into().into()
}

#[cfg(test)]
mod tests {
    use super::base64;

    /// Check standard Base64 padding for all final chunk lengths.
    #[test]
    fn base64_encodes_partial_chunks() {
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    /// Exercise one persistent authenticated session and verify DATA dot transparency.
    #[test]
    fn persistent_session_authenticates_once_and_stuffs_data_lines() {
        use super::SmtpClient;
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::thread;

        /// Read one client command line as UTF-8.
        fn read_command(reader: &mut BufReader<std::net::TcpStream>) -> String {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            line
        }

        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            reader.get_mut().write_all(b"220 ready\r\n").unwrap();
            assert_eq!(read_command(&mut reader), "EHLO smoke.test\r\n");
            reader.get_mut().write_all(b"250 hello\r\n").unwrap();
            assert_eq!(read_command(&mut reader), "AUTH LOGIN\r\n");
            reader.get_mut().write_all(b"334 username\r\n").unwrap();
            assert_eq!(read_command(&mut reader), "dXNlcg==\r\n");
            reader.get_mut().write_all(b"334 password\r\n").unwrap();
            assert_eq!(read_command(&mut reader), "c2VjcmV0\r\n");
            reader
                .get_mut()
                .write_all(b"235 authenticated\r\n")
                .unwrap();

            for _ in 0..2 {
                assert_eq!(read_command(&mut reader), "MAIL FROM:<sender@test>\r\n");
                reader.get_mut().write_all(b"250 sender ok\r\n").unwrap();
                assert_eq!(read_command(&mut reader), "RCPT TO:<recipient@test>\r\n");
                reader.get_mut().write_all(b"250 recipient ok\r\n").unwrap();
                assert_eq!(read_command(&mut reader), "DATA\r\n");
                reader.get_mut().write_all(b"354 send data\r\n").unwrap();
                let mut data = Vec::new();
                loop {
                    let mut line = Vec::new();
                    assert!(
                        reader.read_until(b'\n', &mut line).unwrap() > 0,
                        "unexpected EOF in DATA"
                    );
                    if line == b".\r\n" {
                        break;
                    }
                    data.extend_from_slice(&line);
                }
                assert_eq!(data, b"Subject: test\r\n..leading dot\r\nbody\r\n");
                reader.get_mut().write_all(b"250 queued\r\n").unwrap();
            }
            assert_eq!(read_command(&mut reader), "QUIT\r\n");
            reader.get_mut().write_all(b"221 bye\r\n").unwrap();
        });

        let mut client = SmtpClient::connect("127.0.0.1", port, Some(("user", "secret"))).unwrap();
        client
            .send(
                "sender@test",
                "recipient@test",
                b"Subject: test\r\n.leading dot\r\nbody",
            )
            .unwrap();
        client
            .send(
                "sender@test",
                "recipient@test",
                b"Subject: test\r\n.leading dot\r\nbody",
            )
            .unwrap();
        client.quit().unwrap();
        server.join().unwrap();
    }
}
