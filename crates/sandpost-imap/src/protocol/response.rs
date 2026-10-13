//! Server response serialization (RFC 3501 sections 7 and 9).
//!
//! [`Output`] accumulates response bytes. Strings are written as atoms, quoted strings, or
//! literals depending on content. Values with CR, LF, or 8-bit octets use literals; ordinary
//! IMAP4rev1 literals cannot contain NUL. FETCH rejects binary message content before serialization.
use crate::protocol::{
    flags::{Flags, SystemFlag},
    syntax::{is_atom, is_quotable},
};

/// Longest value written as a quoted string; longer values are sent as literals.
const MAXIMUM_QUOTED_LENGTH: usize = 1024;

/// A growable buffer of response bytes with IMAP data-type writers.
#[derive(Debug, Default)]
pub struct Output {
    bytes: Vec<u8>,
}

impl Output {
    /// An empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// The accumulated bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Number of accumulated bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Report whether nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Discard the accumulated bytes after they have been sent.
    pub fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Append protocol text verbatim; callers pass only syntax they constructed.
    pub fn raw(&mut self, text: &str) -> &mut Self {
        self.bytes.extend_from_slice(text.as_bytes());
        self
    }

    /// Append raw octets verbatim.
    pub fn raw_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// Append a decimal number.
    pub fn number(&mut self, value: impl std::fmt::Display) -> &mut Self {
        self.raw(&value.to_string())
    }

    /// Terminate the current response line.
    pub fn end_line(&mut self) -> &mut Self {
        self.raw("\r\n")
    }

    /// Write `string`: quoted when it is short 7-bit text, otherwise a literal.
    pub fn string(&mut self, value: &[u8]) -> &mut Self {
        if value.len() <= MAXIMUM_QUOTED_LENGTH && is_quotable(value) {
            self.bytes.push(b'"');
            for byte in value {
                if matches!(byte, b'"' | b'\\') {
                    self.bytes.push(b'\\');
                }
                self.bytes.push(*byte);
            }
            self.bytes.push(b'"');
            self
        } else {
            self.literal(value)
        }
    }

    /// Write a NUL-free literal: `{length}` CRLF followed by the octets.
    /// Callers validate binary content before constructing a response (RFC 3501 section 4.3.1).
    pub fn literal(&mut self, value: &[u8]) -> &mut Self {
        self.raw(&format!("{{{}}}\r\n", value.len()));
        self.raw_bytes(value)
    }

    /// Write `nstring`: `NIL` or a string.
    pub fn nstring(&mut self, value: Option<&[u8]>) -> &mut Self {
        match value {
            Some(value) => self.string(value),
            None => self.raw("NIL"),
        }
    }

    /// Write `astring`: a bare atom when possible, otherwise a string.
    pub fn astring(&mut self, value: &[u8]) -> &mut Self {
        if is_atom(value) && !value.eq_ignore_ascii_case(b"NIL") {
            self.raw_bytes(value)
        } else {
            self.string(value)
        }
    }

    /// Write a parenthesized flag list, adding `\Recent` when the session marks it.
    pub fn flag_list(&mut self, flags: &Flags, recent: bool) -> &mut Self {
        self.raw("(");
        let mut first = true;
        let mut separator = |output: &mut Self| {
            if !std::mem::take(&mut first) {
                output.raw(" ");
            }
        };
        for flag in SystemFlag::ALL {
            if flags.contains(flag) {
                separator(self);
                self.raw(flag.name());
            }
        }
        if recent {
            separator(self);
            self.raw("\\Recent");
        }
        for keyword in flags.keywords() {
            separator(self);
            self.raw(keyword);
        }
        self.raw(")")
    }

    /// Write a complete untagged response line: `* text CRLF`.
    pub fn untagged(&mut self, text: &str) -> &mut Self {
        self.raw("* ").raw(text).end_line()
    }

    /// Write a complete status response line with an optional response code.
    pub fn status(
        &mut self,
        tag: Option<&str>,
        status: Status,
        code: Option<&ResponseCode>,
        text: &str,
    ) -> &mut Self {
        self.raw(tag.unwrap_or("*"))
            .raw(" ")
            .raw(status.keyword())
            .raw(" ");
        if let Some(code) = code {
            self.raw("[").raw(&code.render()).raw("] ");
        }
        self.raw(&sanitize_text(text)).end_line()
    }

    /// Write a command continuation request: `+ text CRLF`.
    pub fn continuation(&mut self, text: &str) -> &mut Self {
        self.raw("+ ").raw(&sanitize_text(text)).end_line()
    }

    /// Write an empty SASL challenge: `+ ` CRLF (RFC 3501 section 6.2.2).
    pub fn empty_challenge(&mut self) -> &mut Self {
        self.raw("+ ").end_line()
    }
}

/// Make human-readable response text a valid `text`: printable 7-bit characters only.
fn sanitize_text(text: &str) -> String {
    let sanitized: String = text
        .chars()
        .map(|character| {
            if (' '..='~').contains(&character) {
                character
            } else {
                '?'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "OK".to_owned()
    } else {
        sanitized
    }
}

/// The result keyword of a status response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    No,
    Bad,
    PreAuthenticated,
    Bye,
}

impl Status {
    /// The keyword as written on the wire.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::No => "NO",
            Self::Bad => "BAD",
            Self::PreAuthenticated => "PREAUTH",
            Self::Bye => "BYE",
        }
    }
}

/// A bracketed response code (RFC 3501 section 7.1, RFC 4315 section 3, RFC 5530, RFC 4469).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseCode {
    Alert,
    Capability(String),
    Parse,
    PermanentFlags(String),
    ReadOnly,
    ReadWrite,
    TryCreate,
    UidNext(u32),
    UidValidity(u32),
    Unseen(u32),
    CopyUid {
        uid_validity: u32,
        source: String,
        destination: String,
    },
    Unavailable,
    AuthenticationFailed,
    PrivacyRequired,
    Cannot,
    NonExistent,
    AlreadyExists,
    ExpungeIssued,
    NoPermission,
    ClientBug,
    ServerBug,
    Limit,
    BadCharset(String),
    TooBig,
    Closed,
}

impl ResponseCode {
    /// The response code text inside its brackets.
    pub fn render(&self) -> String {
        match self {
            Self::Alert => "ALERT".into(),
            Self::Capability(list) => format!("CAPABILITY {list}"),
            Self::Parse => "PARSE".into(),
            Self::PermanentFlags(list) => format!("PERMANENTFLAGS {list}"),
            Self::ReadOnly => "READ-ONLY".into(),
            Self::ReadWrite => "READ-WRITE".into(),
            Self::TryCreate => "TRYCREATE".into(),
            Self::UidNext(value) => format!("UIDNEXT {value}"),
            Self::UidValidity(value) => format!("UIDVALIDITY {value}"),
            Self::Unseen(value) => format!("UNSEEN {value}"),
            Self::CopyUid {
                uid_validity,
                source,
                destination,
            } => format!("COPYUID {uid_validity} {source} {destination}"),
            Self::Unavailable => "UNAVAILABLE".into(),
            Self::AuthenticationFailed => "AUTHENTICATIONFAILED".into(),
            Self::PrivacyRequired => "PRIVACYREQUIRED".into(),
            Self::Cannot => "CANNOT".into(),
            Self::NonExistent => "NONEXISTENT".into(),
            Self::AlreadyExists => "ALREADYEXISTS".into(),
            Self::ExpungeIssued => "EXPUNGEISSUED".into(),
            Self::NoPermission => "NOPERM".into(),
            Self::ClientBug => "CLIENTBUG".into(),
            Self::ServerBug => "SERVERBUG".into(),
            Self::Limit => "LIMIT".into(),
            Self::BadCharset(list) => format!("BADCHARSET {list}"),
            Self::TooBig => "TOOBIG".into(),
            Self::Closed => "CLOSED".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Strings choose quoted form for short 7-bit text and literals otherwise.
    #[test]
    fn strings_choose_quoted_or_literal_form() {
        let mut output = Output::new();
        output.string(b"He said \"hi\" \\o/");
        assert_eq!(output.as_bytes(), b"\"He said \\\"hi\\\" \\\\o/\"");
        output.clear();
        output.string(b"two\r\nlines");
        assert_eq!(output.as_bytes(), b"{10}\r\ntwo\r\nlines");
        output.clear();
        output.string("caf\u{e9}".as_bytes());
        assert_eq!(output.as_bytes(), b"{5}\r\ncaf\xc3\xa9");
        output.clear();
        output
            .nstring(None)
            .raw(" ")
            .astring(b"INBOX")
            .raw(" ")
            .astring(b"NIL");
        assert_eq!(output.as_bytes(), b"NIL INBOX \"NIL\"");
    }

    /// Flag lists use canonical system order, then `\Recent`, then keywords.
    #[test]
    fn flag_lists_are_canonical() {
        let mut flags = Flags::new();
        flags.insert(SystemFlag::Seen);
        flags.insert(SystemFlag::Answered);
        flags.insert_keyword("$Forwarded");
        let mut output = Output::new();
        output.flag_list(&flags, true);
        assert_eq!(
            output.as_bytes(),
            b"(\\Answered \\Seen \\Recent $Forwarded)"
        );
        output.clear();
        output.flag_list(&Flags::new(), false);
        assert_eq!(output.as_bytes(), b"()");
    }

    /// Status lines carry tags, codes, and sanitized human text.
    #[test]
    fn status_lines_are_well_formed() {
        let mut output = Output::new();
        output.status(
            Some("a1"),
            Status::Ok,
            Some(&ResponseCode::CopyUid {
                uid_validity: 7,
                source: "1:2".into(),
                destination: "9:10".into(),
            }),
            "Done",
        );
        output.status(None, Status::Bye, None, "caf\u{e9}\r\n");
        output.continuation("");
        assert_eq!(
            output.as_bytes(),
            b"a1 OK [COPYUID 7 1:2 9:10] Done\r\n* BYE caf???\r\n+ OK\r\n"
        );
    }
}
