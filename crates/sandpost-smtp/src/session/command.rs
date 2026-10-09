//! SMTP command syntax: one command line becomes a typed [`Command`] without any I/O.
use super::mailbox::{is_valid_domain, parse_mailbox, parse_path};
use sandpost_core::Mailbox;

/// Why the argument of `MAIL FROM` or `RCPT TO` was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgumentError {
    /// The path is not a valid SMTP mailbox (or `<>` where the null sender is allowed); `501`.
    InvalidPath,
    /// An ESMTP parameter this server does not implement; `555`.
    UnsupportedParameter,
    /// A known ESMTP parameter with a malformed or repeated value; `501`.
    InvalidParameter,
}

/// A `MAIL FROM` reverse path with the ESMTP parameters the session acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MailRequest {
    /// The sender; `None` is the null sender `<>`.
    pub(crate) sender: Option<Mailbox>,
    /// The size the client announced with `SIZE=` (RFC 1870), if any.
    pub(crate) declared_size: Option<usize>,
    /// Whether the client supplied any ESMTP parameter, for EHLO negotiation enforcement.
    pub(crate) has_parameters: bool,
}

/// One client command with its argument already validated.
///
/// Malformed paths are kept as errors rather than rejected here, because the session first
/// answers state errors (such as a missing `MAIL FROM`) before argument errors.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command<'line> {
    /// `EHLO` (extended) or `HELO`, with a validated client domain or address literal.
    Hello { extended: bool, domain: &'line str },
    /// `AUTH` with its mechanism and optional initial response, still unparsed.
    Authenticate { argument: &'line str },
    /// `STARTTLS` (RFC 3207).
    StartTls,
    /// `MAIL FROM:<path> [parameters]`.
    Mail {
        request: Result<MailRequest, ArgumentError>,
    },
    /// `RCPT TO:<path>`.
    Recipient {
        recipient: Result<Mailbox, ArgumentError>,
    },
    /// `DATA`.
    Data,
    /// `RSET`.
    Reset,
    /// `NOOP`, whose optional argument is ignored.
    NoOperation,
    /// `QUIT`.
    Quit,
    /// `VRFY`, which a catcher answers without confirming any mailbox.
    Verify,
    /// `HELP`.
    Help,
    /// A standard command this server deliberately does not implement, such as `EXPN`.
    NotImplemented,
    /// A command name this server does not know.
    Unrecognized,
    /// A known command whose argument is malformed; the text explains the expected form.
    InvalidSyntax(&'static str),
}

impl<'line> Command<'line> {
    /// Parse one command line (without its CRLF).
    ///
    /// Command names are case-insensitive and may be preceded by ASCII whitespace; arguments are
    /// trimmed using the same ASCII whitespace rules.
    pub(crate) fn parse(command_line: &'line str) -> Self {
        let (command_name, argument) = split_command(command_line);
        match command_name.as_str() {
            "EHLO" | "HELO" => {
                let Some(domain) =
                    single_argument(argument).filter(|domain| is_valid_domain(domain))
                else {
                    return Self::InvalidSyntax(
                        "EHLO and HELO require a valid domain or address literal",
                    );
                };
                Self::Hello {
                    extended: command_name == "EHLO",
                    domain,
                }
            }
            "AUTH" => Self::Authenticate { argument },
            "STARTTLS" if argument.is_empty() => Self::StartTls,
            "STARTTLS" => Self::InvalidSyntax("STARTTLS takes no argument"),
            "MAIL" => match strip_parameter(argument, "FROM:") {
                Some(rest) => Self::Mail {
                    request: parse_mail_request(rest),
                },
                None => Self::InvalidSyntax("expected MAIL FROM:<reverse-path>"),
            },
            "RCPT" => match strip_parameter(argument, "TO:") {
                Some(rest) => Self::Recipient {
                    recipient: parse_recipient(rest),
                },
                None => Self::InvalidSyntax("expected RCPT TO:<forward-path>"),
            },
            "DATA" if argument.is_empty() => Self::Data,
            "DATA" => Self::InvalidSyntax("DATA takes no argument"),
            "RSET" if argument.is_empty() => Self::Reset,
            "RSET" => Self::InvalidSyntax("RSET takes no argument"),
            "QUIT" if argument.is_empty() => Self::Quit,
            "QUIT" => Self::InvalidSyntax("QUIT takes no argument"),
            "NOOP" => Self::NoOperation,
            "VRFY" if argument.is_empty() => Self::InvalidSyntax("VRFY requires an argument"),
            "VRFY" => Self::Verify,
            "HELP" => Self::Help,
            "EXPN" | "TURN" | "ETRN" | "ATRN" | "BDAT" | "BURL" | "SEND" | "SOML" | "SAML" => {
                Self::NotImplemented
            }
            _ => Self::Unrecognized,
        }
    }
}

/// Split a command line into its uppercase command name and ASCII-trimmed argument.
fn split_command(command_line: &str) -> (String, &str) {
    let command_line =
        command_line.trim_start_matches(|character: char| character.is_ascii_whitespace());
    let (command_name, argument) = command_line
        .split_once(|character: char| character.is_ascii_whitespace())
        .unwrap_or((command_line, ""));
    (
        command_name.to_ascii_uppercase(),
        argument.trim_matches(|character: char| character.is_ascii_whitespace()),
    )
}

/// Return a single non-whitespace argument, rejecting trailing words.
fn single_argument(argument: &str) -> Option<&str> {
    let mut words = argument.split_ascii_whitespace();
    let word = words.next()?;
    words.next().is_none().then_some(word)
}

/// Return the rest of an argument after a case-insensitive parameter name such as `FROM:`.
fn strip_parameter<'argument>(argument: &'argument str, parameter: &str) -> Option<&'argument str> {
    argument
        .get(..parameter.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(parameter))
        .map(|_| &argument[parameter.len()..])
}

/// Parse a `MAIL FROM` reverse path and its supported ESMTP parameters.
fn parse_mail_request(argument: &str) -> Result<MailRequest, ArgumentError> {
    let (path, parameters) = split_path(argument)?;
    let sender = if path.is_empty() {
        None
    } else {
        Some(parse_path(path).map_err(|_| ArgumentError::InvalidPath)?)
    };
    let mut request = MailRequest {
        sender,
        declared_size: None,
        has_parameters: !parameters.is_empty(),
    };
    let mut has_size = false;
    let mut has_body = false;
    let mut has_auth = false;
    for parameter in parameters.split_ascii_whitespace() {
        let (keyword, value) = parameter.split_once('=').unwrap_or((parameter, ""));
        if !valid_esmtp_parameter(keyword, value, parameter.contains('=')) {
            return Err(ArgumentError::InvalidParameter);
        }
        match keyword.to_ascii_uppercase().as_str() {
            "SIZE" => {
                if has_size {
                    return Err(ArgumentError::InvalidParameter);
                }
                has_size = true;
                request.declared_size = Some(parse_decimal(value)?);
            }
            "BODY" => {
                if has_body {
                    return Err(ArgumentError::InvalidParameter);
                }
                has_body = true;
                if !value.eq_ignore_ascii_case("7BIT") && !value.eq_ignore_ascii_case("8BITMIME") {
                    return Err(ArgumentError::InvalidParameter);
                }
            }
            "AUTH" => {
                if has_auth {
                    return Err(ArgumentError::InvalidParameter);
                }
                has_auth = true;
                if !valid_auth_identity(value) {
                    return Err(ArgumentError::InvalidParameter);
                }
            }
            _ => return Err(ArgumentError::UnsupportedParameter),
        }
    }
    Ok(request)
}

/// Validate the ASCII keyword and optional value syntax of an ESMTP parameter.
fn valid_esmtp_parameter(keyword: &str, value: &str, has_value: bool) -> bool {
    let mut keyword_bytes = keyword.bytes();
    keyword_bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && keyword_bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && (!has_value
            || (!value.is_empty()
                && value
                    .bytes()
                    .all(|byte| (0x21..=0x7e).contains(&byte) && byte != b'=')))
}

/// Validate an AUTH identity after strict xtext decoding.
fn valid_auth_identity(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' if index + 2 < bytes.len() => {
                let Some(high) = hex_value(bytes[index + 1]) else {
                    return false;
                };
                let Some(low) = hex_value(bytes[index + 2]) else {
                    return false;
                };
                decoded.push(high * 16 + low);
                index += 3;
            }
            byte if (0x21..=0x7e).contains(&byte) && byte != b'=' && byte != b'+' => {
                decoded.push(byte);
                index += 1;
            }
            _ => return false,
        }
    }
    let Ok(identity) = std::str::from_utf8(&decoded) else {
        return false;
    };
    identity == "<>" || parse_mailbox(identity).is_ok()
}

/// Convert one ASCII hexadecimal digit to its value.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Parse a `RCPT TO` forward path, which must name a mailbox and takes no parameters here.
fn parse_recipient(argument: &str) -> Result<Mailbox, ArgumentError> {
    let (path, parameters) = split_path(argument)?;
    if path.is_empty() {
        return Err(ArgumentError::InvalidPath);
    }
    let recipient = if path.eq_ignore_ascii_case("postmaster") {
        Mailbox {
            address: path.to_owned(),
            domain: String::new(),
        }
    } else {
        parse_path(path).map_err(|_| ArgumentError::InvalidPath)?
    };
    if !parameters.is_empty() {
        return Err(ArgumentError::UnsupportedParameter);
    }
    Ok(recipient)
}

/// Split `<path> parameters` while recognizing `>` inside quoted local parts and route literals.
fn split_path(argument: &str) -> Result<(&str, &str), ArgumentError> {
    let bracketed = argument
        .trim_start_matches(|character: char| character.is_ascii_whitespace())
        .strip_prefix('<')
        .ok_or(ArgumentError::InvalidPath)?;
    let mut quoted = false;
    let mut escaped = false;
    let mut closing_index = None;
    for (index, character) in bracketed.char_indices() {
        if !character.is_ascii() || character.is_ascii_control() {
            return Err(ArgumentError::InvalidPath);
        }
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if !quoted && character == '>' {
            closing_index = Some(index);
            break;
        }
    }
    let closing_index = closing_index.ok_or(ArgumentError::InvalidPath)?;
    let parameters = &bracketed[closing_index + 1..];
    if !parameters.is_empty()
        && !parameters.starts_with(|character: char| character.is_ascii_whitespace())
    {
        return Err(ArgumentError::InvalidPath);
    }
    Ok((
        &bracketed[..closing_index],
        parameters.trim_matches(|character: char| character.is_ascii_whitespace()),
    ))
}

/// Parse a non-empty run of at most 20 ASCII digits, saturating values that exceed `usize`.
fn parse_decimal(value: &str) -> Result<usize, ArgumentError> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ArgumentError::InvalidParameter);
    }
    Ok(value.parse().unwrap_or(usize::MAX))
}
