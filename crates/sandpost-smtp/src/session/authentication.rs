//! SMTP AUTH (RFC 4954) exchanges for the PLAIN (RFC 4616) and LOGIN mechanisms.
//!
//! Only mechanisms that transmit the password itself are offered: the server stores one-way
//! password hashes, so challenge-response mechanisms such as CRAM-MD5 cannot be verified.
use crate::{
    Credentials,
    framing::{ReceivedLine, is_line_too_long, read_line_with_limit, write_reply},
    limits::MAXIMUM_AUTHENTICATION_LINE_SIZE,
};
use mail_parser::decoders::base64::base64_decode;
use sandpost_core::SmtpAuthenticationMechanism;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};

/// The EHLO capability line listing the supported mechanisms.
pub(crate) const AUTHENTICATION_CAPABILITY: &str = "AUTH PLAIN LOGIN";
/// LOGIN challenge carrying base64 "Username:".
const USERNAME_CHALLENGE: &str = "334 VXNlcm5hbWU6";
/// LOGIN challenge carrying base64 "Password:".
const PASSWORD_CHALLENGE: &str = "334 UGFzc3dvcmQ6";
/// Empty continuation used by PLAIN when the client sent no initial response.
const EMPTY_CHALLENGE: &str = "334 ";
/// Largest encoded SASL response that can contain three 255-byte identities or passwords.
const MAXIMUM_ENCODED_RESPONSE_SIZE: usize = 1024;
/// Supported decoded identity or password size, meeting RFC 4616's required minimum capacity.
const MAXIMUM_CREDENTIAL_FIELD_SIZE: usize = 255;

/// Result of one AUTH command's exchange.
pub(crate) enum AuthenticationExchange {
    /// The client supplied complete credentials for the application to verify.
    Credentials(Credentials),
    /// The client supplied an authorization identity other than its own, which is never
    /// permitted; reported like invalid credentials.
    ForeignAuthorization,
    /// The exchange failed without credentials; send this response and keep the session.
    Refused(&'static str),
    /// The client disconnected or timed out; the session must end.
    Closed,
}

/// Run the exchange for one `AUTH` command argument and collect the presented credentials.
///
/// Each client continuation must arrive within `input_output_timeout`; `encrypted` records whether
/// the session is protected by TLS, for the handler to enforce per-credential rules.
pub(crate) async fn read_authentication_exchange<Stream>(
    reader: &mut BufReader<Stream>,
    argument: &str,
    input_output_timeout: Duration,
    encrypted: bool,
) -> std::io::Result<AuthenticationExchange>
where
    Stream: AsyncRead + AsyncWrite + Unpin,
{
    let mut parts = argument.split_ascii_whitespace();
    let Some(mechanism) = parts.next() else {
        return Ok(AuthenticationExchange::Refused(
            "501 5.5.4 authentication mechanism required",
        ));
    };
    let initial_response = parts.next();
    if parts.next().is_some() {
        return Ok(AuthenticationExchange::Refused("501 5.5.4 syntax error"));
    }
    if mechanism.eq_ignore_ascii_case("PLAIN") {
        let response = match initial_response {
            // RFC 4954: "=" is an empty initial response.
            Some("=") => Vec::new(),
            Some(encoded) => match decode_response(encoded) {
                Some(decoded) => decoded,
                None => return Ok(undecodable()),
            },
            None => match read_response(reader, EMPTY_CHALLENGE, input_output_timeout).await? {
                ClientResponse::Decoded(decoded) => decoded,
                ClientResponse::Exchange(outcome) => return Ok(outcome),
            },
        };
        Ok(plain_credentials(response, encrypted))
    } else if mechanism.eq_ignore_ascii_case("LOGIN") {
        let username = match initial_response {
            Some(encoded) => match decode_response(encoded) {
                Some(decoded) => decoded,
                None => return Ok(undecodable()),
            },
            None => match read_response(reader, USERNAME_CHALLENGE, input_output_timeout).await? {
                ClientResponse::Decoded(decoded) => decoded,
                ClientResponse::Exchange(outcome) => return Ok(outcome),
            },
        };
        let password = match read_response(reader, PASSWORD_CHALLENGE, input_output_timeout).await?
        {
            ClientResponse::Decoded(decoded) => decoded,
            ClientResponse::Exchange(outcome) => return Ok(outcome),
        };
        Ok(
            match (credential_field(username), credential_field(password)) {
                (Some(username), Some(password)) => {
                    AuthenticationExchange::Credentials(Credentials {
                        username,
                        password,
                        mechanism: SmtpAuthenticationMechanism::Login,
                        encrypted,
                    })
                }
                _ => undecodable(),
            },
        )
    } else {
        Ok(AuthenticationExchange::Refused(
            "504 5.5.4 unrecognized authentication mechanism",
        ))
    }
}

/// One decoded client continuation, or the exchange outcome that ended it early.
enum ClientResponse {
    Decoded(Vec<u8>),
    Exchange(AuthenticationExchange),
}

/// Send a challenge and read the client's bounded base64 continuation line.
async fn read_response<Stream>(
    reader: &mut BufReader<Stream>,
    challenge: &str,
    input_output_timeout: Duration,
) -> std::io::Result<ClientResponse>
where
    Stream: AsyncRead + AsyncWrite + Unpin,
{
    write_reply(reader.get_mut(), challenge, input_output_timeout).await?;
    let line = match read_line_with_limit(
        reader,
        input_output_timeout,
        MAXIMUM_AUTHENTICATION_LINE_SIZE,
    )
    .await
    {
        Err(error) if is_line_too_long(&error) => {
            write_reply(
                reader.get_mut(),
                "500 5.5.6 line too long",
                input_output_timeout,
            )
            .await?;
            return Ok(ClientResponse::Exchange(AuthenticationExchange::Closed));
        }
        Err(error) => return Err(error),
        Ok(ReceivedLine::Line(line)) => line,
        Ok(ReceivedLine::Closed) => {
            return Ok(ClientResponse::Exchange(AuthenticationExchange::Closed));
        }
        Ok(ReceivedLine::TimedOut) => {
            write_reply(reader.get_mut(), "421 4.4.2 timeout", input_output_timeout).await?;
            return Ok(ClientResponse::Exchange(AuthenticationExchange::Closed));
        }
    };
    if line == b"*" {
        return Ok(ClientResponse::Exchange(AuthenticationExchange::Refused(
            "501 5.7.0 authentication cancelled",
        )));
    }
    Ok(
        match std::str::from_utf8(&line).ok().and_then(decode_response) {
            Some(decoded) => ClientResponse::Decoded(decoded),
            None => ClientResponse::Exchange(undecodable()),
        },
    )
}

/// Decode one base64 client response; an empty line is an empty response.
fn decode_response(encoded: &str) -> Option<Vec<u8>> {
    if encoded.is_empty() {
        return Some(Vec::new());
    }
    if !valid_base64(encoded.as_bytes()) {
        return None;
    }
    base64_decode(encoded.as_bytes())
}

/// Validate standard padded base64, including final-block position and unused bits.
fn valid_base64(encoded: &[u8]) -> bool {
    if encoded.len() > MAXIMUM_ENCODED_RESPONSE_SIZE || !encoded.len().is_multiple_of(4) {
        return false;
    }
    let Some(last_block_start) = encoded.len().checked_sub(4) else {
        return false;
    };
    for (index, byte) in encoded.iter().copied().enumerate() {
        if byte == b'=' {
            if index < last_block_start + 2 {
                return false;
            }
        } else if !byte.is_ascii_alphanumeric() && !matches!(byte, b'+' | b'/') {
            return false;
        }
    }
    let last = &encoded[last_block_start..];
    match (last[2], last[3]) {
        (b'=', b'=') => base64_value(last[1]).is_some_and(|value| value & 0x0f == 0),
        (_, b'=') => base64_value(last[2]).is_some_and(|value| value & 0x03 == 0),
        (b'=', _) => false,
        _ => true,
    }
}

/// Return the six-bit value for one non-padding base64 alphabet byte.
fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Convert one nonempty, NUL-free UTF-8 credential field of at most 255 bytes.
fn credential_field(bytes: Vec<u8>) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAXIMUM_CREDENTIAL_FIELD_SIZE || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Split a PLAIN response into authorization identity, username, and password.
fn plain_credentials(response: Vec<u8>, encrypted: bool) -> AuthenticationExchange {
    let mut fields = response.split(|byte| *byte == 0);
    let (Some(authorization), Some(username), Some(password), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return AuthenticationExchange::Refused("501 5.5.2 malformed PLAIN response");
    };
    if authorization.len() > MAXIMUM_CREDENTIAL_FIELD_SIZE {
        return undecodable();
    }
    let (Ok(authorization), Some(username), Some(password)) = (
        std::str::from_utf8(authorization),
        credential_field(username.to_vec()),
        credential_field(password.to_vec()),
    ) else {
        return undecodable();
    };
    if !authorization.is_empty() && authorization != username {
        return AuthenticationExchange::ForeignAuthorization;
    }
    AuthenticationExchange::Credentials(Credentials {
        username,
        password,
        mechanism: SmtpAuthenticationMechanism::Plain,
        encrypted,
    })
}

/// Refuse a response that is not valid base64 or UTF-8.
fn undecodable() -> AuthenticationExchange {
    AuthenticationExchange::Refused("501 5.5.2 cannot decode response")
}
