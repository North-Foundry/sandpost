//! On-demand tokenization of PersonaScript source.
//!
//! The lexer is incremental rather than tokenizing the whole source up front: when the parser
//! reaches a condition it hands the condition text to `sandpost-query` and then repositions the
//! lexer after the condition's closing parenthesis. Query syntax therefore never has to be valid
//! PersonaScript syntax, and vice versa.
use crate::diagnostics::Diagnostic;
use std::time::Duration;

/// The kind and decoded value of one token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenKind {
    /// A keyword or argument name such as `open`, `link` or `text`.
    Identifier(String),
    /// A quoted string with escapes decoded.
    String(String),
    /// A non-negative duration with its unit, such as `500ms`, `5s` or `2m`.
    Duration(Duration),
    Semicolon,
    LeftBrace,
    RightBrace,
    LeftParenthesis,
    RightParenthesis,
    Colon,
    Comma,
    /// The end of the source.
    End,
}

/// One token with its byte range in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Scans tokens from a source on demand.
pub(crate) struct Lexer<'source> {
    source: &'source str,
    position: usize,
}

impl<'source> Lexer<'source> {
    /**
     * Start scanning at the beginning of `source`.
     */
    pub(crate) fn new(source: &'source str) -> Self {
        Self {
            source,
            position: 0,
        }
    }

    /**
     * Continue scanning from a byte offset, such as just after an embedded condition.
     */
    pub(crate) fn seek(&mut self, position: usize) {
        self.position = position.min(self.source.len());
    }

    /**
     * Scan the next token, skipping whitespace and `//` or `/* */` comments.
     */
    pub(crate) fn next_token(&mut self) -> Result<Token, Diagnostic> {
        self.skip_trivia()?;
        let bytes = self.source.as_bytes();
        let start = self.position;
        let Some(&byte) = bytes.get(start) else {
            return Ok(Token {
                kind: TokenKind::End,
                start,
                end: start,
            });
        };
        let punctuation = match byte {
            b';' => Some(TokenKind::Semicolon),
            b'{' => Some(TokenKind::LeftBrace),
            b'}' => Some(TokenKind::RightBrace),
            b'(' => Some(TokenKind::LeftParenthesis),
            b')' => Some(TokenKind::RightParenthesis),
            b':' => Some(TokenKind::Colon),
            b',' => Some(TokenKind::Comma),
            _ => None,
        };
        let kind = if let Some(kind) = punctuation {
            self.position += 1;
            kind
        } else if byte == b'"' || byte == b'\'' {
            TokenKind::String(self.scan_string(byte)?)
        } else if byte.is_ascii_digit() {
            TokenKind::Duration(self.scan_duration()?)
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            while self.position < bytes.len()
                && (bytes[self.position].is_ascii_alphanumeric() || bytes[self.position] == b'_')
            {
                self.position += 1;
            }
            TokenKind::Identifier(self.source[start..self.position].to_owned())
        } else {
            let character = self.source[start..].chars().next().unwrap_or('\u{fffd}');
            return Err(Diagnostic::at(
                start,
                format!("unexpected character `{}`", character.escape_debug()),
            ));
        };
        Ok(Token {
            kind,
            start,
            end: self.position,
        })
    }

    /**
     * Skip whitespace and comments; an unterminated block comment is an error.
     */
    fn skip_trivia(&mut self) -> Result<(), Diagnostic> {
        let bytes = self.source.as_bytes();
        loop {
            while self.position < bytes.len() && bytes[self.position].is_ascii_whitespace() {
                self.position += 1;
            }
            let rest = &self.source[self.position..];
            if rest.starts_with("//") {
                self.position += rest.find('\n').unwrap_or(rest.len());
            } else if let Some(comment) = rest.strip_prefix("/*") {
                let Some(end) = comment.find("*/") else {
                    return Err(Diagnostic::at(self.position, "unterminated `/*` comment"));
                };
                self.position += end + 4;
            } else {
                return Ok(());
            }
        }
    }

    /**
     * Scan a quoted string, decoding `\n`, `\r` and `\t`; other escapes keep the escaped
     * character, exactly like Sandpost query strings.
     */
    fn scan_string(&mut self, quote: u8) -> Result<String, Diagnostic> {
        let start = self.position;
        self.position += 1;
        let mut value = String::new();
        loop {
            let Some(character) = self.source[self.position..].chars().next() else {
                return Err(Diagnostic::at(start, "unterminated string"));
            };
            self.position += character.len_utf8();
            if character as u32 == u32::from(quote) {
                return Ok(value);
            }
            if character == '\\' {
                let Some(escaped) = self.source[self.position..].chars().next() else {
                    return Err(Diagnostic::at(start, "unterminated string"));
                };
                self.position += escaped.len_utf8();
                value.push(match escaped {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
            } else {
                value.push(character);
            }
        }
    }

    /**
     * Scan an integer followed directly by a `ms`, `s` or `m` unit.
     */
    fn scan_duration(&mut self) -> Result<Duration, Diagnostic> {
        let bytes = self.source.as_bytes();
        let start = self.position;
        while self.position < bytes.len() && bytes[self.position].is_ascii_digit() {
            self.position += 1;
        }
        let digits = &self.source[start..self.position];
        let unit_start = self.position;
        while self.position < bytes.len() && bytes[self.position].is_ascii_alphanumeric() {
            self.position += 1;
        }
        let unit = &self.source[unit_start..self.position];
        let amount: u64 = digits
            .parse()
            .map_err(|_| Diagnostic::at(start, "duration is too large"))?;
        let duration = match unit {
            "ms" => Some(Duration::from_millis(amount)),
            "s" => Some(Duration::from_secs(amount)),
            "m" => amount.checked_mul(60).map(Duration::from_secs),
            "" => {
                return Err(Diagnostic::at(
                    start,
                    "a duration needs a unit: `ms`, `s` or `m`, as in `5s`",
                ));
            }
            other => {
                return Err(Diagnostic::at(
                    unit_start,
                    format!("unknown duration unit `{other}`; use `ms`, `s` or `m`"),
                ));
            }
        };
        duration.ok_or_else(|| Diagnostic::at(start, "duration is too large"))
    }
}
