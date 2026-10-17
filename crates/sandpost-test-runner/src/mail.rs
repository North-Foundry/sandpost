//! Synchronous SMTP and IMAP clients used by the end-to-end smoke runner.

use std::error::Error;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_IMAP_LINE_BYTES: usize = 64 * 1024;
const MAX_IMAP_LITERAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_IMAP_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Send one message over SMTP, optionally authenticating with LOGIN credentials.
pub fn send_mail(
    port: u16,
    sender: &str,
    recipient: &str,
    message: &[u8],
    credentials: Option<(&str, &str)>,
) -> Result<()> {
    let mut client = SmtpClient::connect(port)?;
    client.command("EHLO smoke.test", 250)?;
    if let Some((username, password)) = credentials {
        client.command("AUTH LOGIN", 334)?;
        client.command(&base64(username.as_bytes()), 334)?;
        client.command(&base64(password.as_bytes()), 235)?;
    }
    client.command(&format!("MAIL FROM:<{sender}>"), 250)?;
    client.command(&format!("RCPT TO:<{recipient}>"), 250)?;
    client.command("DATA", 354)?;
    client.write_message(message)?;
    client.expect_code(250)?;
    client.command("QUIT", 221)?;
    Ok(())
}

/// Verify that an invalid SMTP password is rejected with reply code 535.
pub fn reject_smtp_password(port: u16, username: &str) -> Result<()> {
    let mut client = SmtpClient::connect(port)?;
    client.command("EHLO smoke.test", 250)?;
    client.command("AUTH LOGIN", 334)?;
    client.command(&base64(username.as_bytes()), 334)?;
    client.command(&base64(b"not the generated password"), 535)?;
    Ok(())
}

/// Exercise IMAP authentication, mailbox operations, literal fetches, IDLE delivery, and MOVE.
///
/// The returned UIDVALIDITY is retained by the caller to verify persistence after restart.
pub fn check_imap(
    port: u16,
    username: &str,
    password: &str,
    captured_message: &[u8],
    smtp_port: u16,
) -> Result<String> {
    let mut client = ImapClient::connect(port)?;
    let capabilities = client.command("CAPABILITY")?;
    for capability in [
        "IMAP4REV1",
        "IDLE",
        "UIDPLUS",
        "MOVE",
        "UNSELECT",
        "AUTH=PLAIN",
    ] {
        verify(
            contains_ascii(&capabilities, capability),
            format!("missing IMAP capability {capability}"),
        )?;
    }
    verify(
        !contains_ascii(&capabilities, "IMAP4REV2"),
        "IMAP4REV2 must not be advertised",
    )?;

    let (failed_login, failed_login_status) =
        client.command_any(&format!("LOGIN {} \"wrong password\"", quote(username)))?;
    verify(
        failed_login_status == b"NO" && contains_ascii(&failed_login, "AUTHENTICATIONFAILED"),
        "invalid IMAP credentials were accepted",
    )?;
    client.command(&format!("LOGIN {} {}", quote(username), quote(password)))?;
    let mailboxes = client.command("LIST \"\" *")?;
    verify(
        contains_ascii(&mailboxes, "INBOX"),
        "INBOX missing from LIST",
    )?;
    verify(
        contains_ascii(&mailboxes, "\\TRASH") && contains_ascii(&mailboxes, "Trash"),
        "Trash missing from LIST",
    )?;

    let selection = client.command("SELECT INBOX")?;
    verify(
        response_has_line(&selection, b"* 1 EXISTS"),
        "INBOX did not contain one message",
    )?;
    let validity = response_code_value(&selection, "UIDVALIDITY")
        .ok_or_else(|| error("SELECT response omitted UIDVALIDITY"))?;
    let fetched_header =
        client.command("UID FETCH 1:* (UID RFC822.SIZE BODY.PEEK[HEADER.FIELDS (SUBJECT)])")?;
    verify(
        contains_ascii(&fetched_header, "Subject: Sand Post smoke"),
        "header FETCH omitted the subject",
    )?;
    verify(
        response_has_value(
            &fetched_header,
            b"RFC822.SIZE",
            captured_message.len().to_string().as_bytes(),
        ),
        "header FETCH returned an incorrect RFC822.SIZE",
    )?;
    let found = client.command("UID SEARCH CHARSET UTF-8 SUBJECT \"post smoke\"")?;
    verify(
        response_has_line(&found, b"* SEARCH 1"),
        "UID SEARCH did not find message 1",
    )?;
    let body = client.command("FETCH 1 (BODY[])")?;
    verify(
        literal_payload(&body) == Some(captured_message),
        "BODY[] literal differs from captured SMTP message",
    )?;
    let seen = client.command("FETCH 1 (FLAGS)")?;
    verify(
        contains_ascii(&seen, "\\Seen"),
        "BODY[] did not mark message as Seen",
    )?;
    let flagged = client.command("UID STORE 1 +FLAGS (\\Flagged)")?;
    verify(
        contains_ascii(&flagged, "\\Flagged"),
        "STORE did not set Flagged",
    )?;

    client.begin_idle()?;
    send_mail(
        smtp_port,
        "dev@sender.test",
        "user@recipient.test",
        b"Subject: pushed over IDLE\r\n\r\nidle\r\n",
        None,
    )?;
    let idle_event = client.read_response()?;
    verify(
        idle_event == b"* 2 EXISTS\r\n",
        format!(
            "unexpected IDLE event: {}",
            String::from_utf8_lossy(&idle_event)
        ),
    )?;
    client.write_all(b"DONE\r\n")?;
    let (_, idle_status) = client.read_tagged(b"A-IDLE")?;
    verify(idle_status == b"OK", "IMAP IDLE command failed")?;
    client.command("UID MOVE 2 Trash")?;
    let trash_status = client.command("STATUS Trash (MESSAGES UIDNEXT)")?;
    verify(
        response_has_value(&trash_status, b"MESSAGES", b"1"),
        "Trash did not contain one message",
    )?;
    client.command("LOGOUT")?;
    Ok(validity)
}

/// Verify INBOX UID and flag persistence, and the moved message's Trash membership after restart.
pub fn check_imap_restart(port: u16, username: &str, password: &str, validity: &str) -> Result<()> {
    let mut client = ImapClient::connect(port)?;
    client.command(&format!("LOGIN {} {}", quote(username), quote(password)))?;
    let selection = client.command("EXAMINE INBOX")?;
    verify(
        response_has_line(&selection, b"* 3 EXISTS"),
        "restarted INBOX count was not three",
    )?;
    verify(
        response_code_value(&selection, "UIDVALIDITY").as_deref() == Some(validity),
        "UIDVALIDITY changed across restart",
    )?;
    let search = client.command("UID SEARCH ALL")?;
    verify(
        response_has_line(&search, b"* SEARCH 1 3 4"),
        "restarted INBOX UIDs differ",
    )?;
    let flags = client.command("UID FETCH 1 (FLAGS)")?;
    verify(
        contains_ascii(&flags, "\\Seen") && contains_ascii(&flags, "\\Flagged"),
        "message flags did not persist",
    )?;
    let trash = client.command("STATUS Trash (MESSAGES)")?;
    verify(
        response_has_value(&trash, b"MESSAGES", b"1"),
        "Trash count did not persist",
    )?;
    client.command("LOGOUT")?;
    Ok(())
}

/// A buffered SMTP connection that reads complete multiline replies.
struct SmtpClient {
    stream: BufReader<TcpStream>,
}

impl SmtpClient {
    /// Connect to the loopback SMTP listener and consume its greeting.
    fn connect(port: u16) -> Result<Self> {
        let stream = connect(port)?;
        let mut client = Self {
            stream: BufReader::new(stream),
        };
        client.expect_code(220)?;
        Ok(client)
    }

    /// Write one SMTP command and require the expected reply code.
    fn command(&mut self, command: &str, expected: u16) -> Result<Vec<u8>> {
        self.stream.get_mut().write_all(command.as_bytes())?;
        self.stream.get_mut().write_all(b"\r\n")?;
        self.expect_code(expected)
    }

    /// Read an SMTP multiline reply and require its numeric code.
    fn expect_code(&mut self, expected: u16) -> Result<Vec<u8>> {
        let mut reply = Vec::new();
        loop {
            let mut line = Vec::new();
            self.stream.read_until(b'\n', &mut line)?;
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
            reply.extend_from_slice(&line);
            if complete {
                return Ok(reply);
            }
        }
    }

    /// Send message bytes using SMTP dot transparency and terminate DATA.
    fn write_message(&mut self, message: &[u8]) -> Result<()> {
        let mut data = Vec::with_capacity(message.len());
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

/// An IMAP connection that preserves response literals as raw bytes.
struct ImapClient {
    stream: BufReader<TcpStream>,
    next_tag: u32,
}

impl ImapClient {
    /// Connect to the loopback IMAP listener and consume its greeting.
    fn connect(port: u16) -> Result<Self> {
        let stream = connect(port)?;
        let mut client = Self {
            stream: BufReader::new(stream),
            next_tag: 1,
        };
        let greeting = client.read_response()?;
        verify(greeting.starts_with(b"* OK "), "invalid IMAP greeting")?;
        Ok(client)
    }

    /// Send a tagged IMAP command and collect responses through its tagged completion.
    fn command(&mut self, command: &str) -> Result<Vec<u8>> {
        let (response, status) = self.command_any(command)?;
        verify(
            status == b"OK",
            format!(
                "IMAP command failed: {}",
                String::from_utf8_lossy(&response)
            ),
        )?;
        Ok(response)
    }

    /// Send a tagged IMAP command and return its complete response, including tagged NO/BAD.
    fn command_any(&mut self, command: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag += 1;
        self.write_all(format!("{tag} {command}\r\n").as_bytes())?;
        self.read_tagged(tag.as_bytes())
    }

    /// Collect complete IMAP responses until the requested tag completes.
    fn read_tagged(&mut self, tag: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut responses = Vec::new();
        loop {
            let response = self.read_response()?;
            let complete = response.starts_with(tag) && response.get(tag.len()) == Some(&b' ');
            if responses.len().saturating_add(response.len()) > MAX_IMAP_RESPONSE_BYTES {
                return Err(error("IMAP command response exceeded the size limit"));
            }
            let status = if complete {
                tagged_status(&response, tag)
            } else {
                None
            };
            responses.extend_from_slice(&response);
            if complete {
                return status
                    .map(|status| (responses, status.to_vec()))
                    .ok_or_else(|| error("malformed tagged IMAP completion"));
            }
        }
    }

    /// Read one response line and any literal payloads declared within it.
    fn read_response(&mut self) -> Result<Vec<u8>> {
        let mut response = Vec::new();
        loop {
            let line = read_imap_line(&mut self.stream)?;
            if line.is_empty() {
                return Err(error("IMAP connection closed"));
            }
            let literal_length = literal_length(&line);
            if response.len().saturating_add(line.len()) > MAX_IMAP_RESPONSE_BYTES {
                return Err(error("IMAP response exceeded the size limit"));
            }
            response.extend_from_slice(&line);
            if let Some(length) = literal_length {
                if length > MAX_IMAP_LITERAL_BYTES
                    || response.len().saturating_add(length) > MAX_IMAP_RESPONSE_BYTES
                {
                    return Err(error("IMAP literal exceeded the size limit"));
                }
                let start = response.len();
                response.resize(start + length, 0);
                self.stream.read_exact(&mut response[start..])?;
            } else {
                return Ok(response);
            }
        }
    }

    /// Write raw bytes to the connected IMAP stream.
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.get_mut().write_all(bytes)?;
        Ok(())
    }

    /// Enter IDLE and consume its continuation response.
    fn begin_idle(&mut self) -> Result<()> {
        self.write_all(b"A-IDLE IDLE\r\n")?;
        let continuation = self.read_response()?;
        verify(
            continuation.starts_with(b"+ idling"),
            "IMAP IDLE continuation was missing",
        )
    }
}

/// Open a loopback TCP connection with bounded connect, read, and write operations.
fn connect(port: u16) -> Result<TcpStream> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(stream)
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

/// Quote an IMAP astring, escaping backslashes and double quotes.
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Find an IMAP literal byte count from a trailing `{length}` marker.
fn literal_length(line: &[u8]) -> Option<usize> {
    let marker_line = line
        .strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))?;
    let closing = marker_line.len().checked_sub(1)?;
    if marker_line[closing] != b'}' {
        return None;
    }
    let opening = marker_line[..closing]
        .iter()
        .rposition(|byte| *byte == b'{')?;
    std::str::from_utf8(&marker_line[opening + 1..closing])
        .ok()?
        .parse()
        .ok()
}

/// Read one newline-terminated IMAP line without exceeding its byte limit.
fn read_imap_line(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(line)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete IMAP line",
                ))
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(take) > MAX_IMAP_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IMAP line exceeded the size limit",
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

/// Extract the exact bytes of the first IMAP literal in a response.
fn literal_payload(response: &[u8]) -> Option<&[u8]> {
    let line_end = response.iter().position(|byte| *byte == b'\n')? + 1;
    let length = literal_length(&response[..line_end])?;
    let payload_end = line_end.checked_add(length)?;
    response.get(line_end..payload_end)
}

/// Return the status token only when a response line completes the requested IMAP tag.
fn tagged_status<'response>(response: &'response [u8], tag: &[u8]) -> Option<&'response [u8]> {
    let line_end = response
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(response.len());
    let line = response[..line_end]
        .strip_suffix(b"\r")
        .unwrap_or(&response[..line_end]);
    let mut fields = line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    (fields.next()? == tag)
        .then_some(fields.next()?)
        .filter(|status| *status == b"OK" || *status == b"NO" || *status == b"BAD")
}

/// Check whether a response contains a complete protocol line equal to the expected bytes.
fn response_has_line(response: &[u8], expected: &[u8]) -> bool {
    response
        .split(|byte| *byte == b'\n')
        .any(|line| line.strip_suffix(b"\r").unwrap_or(line) == expected)
}

/// Check whether one response line contains an exact adjacent key and value token pair.
fn response_has_value(response: &[u8], key: &[u8], value: &[u8]) -> bool {
    response.split(|byte| *byte == b'\n').any(|line| {
        let mut previous_is_key = false;
        for field in line.split(|byte| byte.is_ascii_whitespace() || *byte == b'(' || *byte == b')')
        {
            if previous_is_key && field == value {
                return true;
            }
            previous_is_key = field == key;
        }
        false
    })
}

/// Locate a bracketed response code and return its following value.
fn response_code_value(response: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(response);
    let marker = format!("[{name} ");
    let value = text.split_once(&marker)?.1.split(']').next()?;
    Some(value.to_owned())
}

/// Search a protocol response case-insensitively for an ASCII token.
fn contains_ascii(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Return a boxed diagnostic when a smoke assertion fails.
fn verify(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(error(message))
    }
}

/// Construct a boxed error from a diagnostic string.
fn error(message: impl Into<String>) -> Box<dyn Error> {
    message.into().into()
}

#[cfg(test)]
mod tests {
    use super::{
        base64, literal_length, literal_payload, response_has_line, response_has_value,
        tagged_status,
    };

    /// Check SMTP Base64 padding and IMAP literal length parsing.
    #[test]
    fn protocol_encodings_preserve_lengths_and_padding() {
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(literal_length(b"* 1 FETCH (BODY[] {3}\r\n"), Some(3));
        assert_eq!(literal_length(b"* 1 FETCH (FLAGS (\\Seen))\r\n"), None);
        assert_eq!(literal_length(b"* 1 FETCH (BODY[] {3} trailing}\r\n"), None);
        assert_eq!(
            literal_payload(b"* 1 FETCH (BODY[] {3}\r\nabc)\r\nA0001 OK done\r\n"),
            Some(&b"abc"[..])
        );
        assert_eq!(
            tagged_status(b"A0001 NO rejected\r\n", b"A0001"),
            Some(&b"NO"[..])
        );
        assert_eq!(tagged_status(b"* OK unsolicited\r\n", b"A0001"), None);
        assert!(!response_has_line(
            b"* SEARCH 1 3 4 5\r\n",
            b"* SEARCH 1 3 4"
        ));
        assert!(!response_has_value(
            b"* STATUS Trash (MESSAGES 10)\r\n",
            b"MESSAGES",
            b"1"
        ));
    }
}
