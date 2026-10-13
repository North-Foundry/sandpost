//! Byte-level parsing primitives shared by IMAP command parsers.

use crate::protocol::{
    mailbox_name,
    sequence::SequenceSet,
    syntax::{is_astring_byte, is_atom_byte, is_list_byte, is_tag_byte},
};

/// Parsing position within one command.
pub(super) struct Cursor<'input> {
    pub(super) input: &'input [u8],
    pub(super) position: usize,
}

pub(super) type ParseResult<T> = Result<T, String>;

impl<'input> Cursor<'input> {
    /// Start parsing at the beginning of one input command.
    pub(super) fn new(input: &'input [u8]) -> Self {
        Self { input, position: 0 }
    }

    /// The next byte without consuming it.
    pub(super) fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    /// Report whether every byte was consumed.
    pub(super) fn at_end(&self) -> bool {
        self.position >= self.input.len()
    }

    /// Consume one expected byte.
    pub(super) fn expect(&mut self, byte: u8, description: &str) -> ParseResult<()> {
        if self.peek() == Some(byte) {
            self.position += 1;
            Ok(())
        } else {
            Err(format!("expected {description}"))
        }
    }

    /// Consume exactly one space.
    pub(super) fn space(&mut self) -> ParseResult<()> {
        self.expect(b' ', "a space")
    }

    /// Require the end of the command.
    pub(super) fn end(&self) -> ParseResult<()> {
        if self.at_end() {
            Ok(())
        } else {
            Err("unexpected extra arguments".into())
        }
    }

    /// Consume the longest run of bytes accepted by `accept`.
    pub(super) fn take_while(&mut self, accept: impl Fn(u8) -> bool) -> &'input [u8] {
        let start = self.position;
        while self.peek().is_some_and(&accept) {
            self.position += 1;
        }
        &self.input[start..self.position]
    }

    /// Read a tag: one or more astring characters other than `+`.
    pub(super) fn tag(&mut self) -> ParseResult<String> {
        let tag = self.take_while(is_tag_byte);
        if tag.is_empty() {
            return Err("missing or invalid tag".into());
        }
        Ok(String::from_utf8_lossy(tag).into_owned())
    }

    /// Read an atom as text.
    pub(super) fn atom(&mut self) -> ParseResult<&'input str> {
        let atom = self.take_while(is_atom_byte);
        if atom.is_empty() {
            return Err("expected an atom".into());
        }
        std::str::from_utf8(atom).map_err(|_| "invalid atom".to_owned())
    }

    /// Read a positive or zero 32-bit number.
    pub(super) fn number(&mut self) -> ParseResult<u32> {
        let digits = self.take_while(|byte| byte.is_ascii_digit());
        if digits.is_empty() || digits.len() > 10 {
            return Err("expected a number".into());
        }
        std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| digits.parse().ok())
            .ok_or_else(|| "number out of range".to_owned())
    }

    /// Read a quoted string, resolving `\"` and `\\` escapes.
    pub(super) fn quoted(&mut self) -> ParseResult<Vec<u8>> {
        self.expect(b'"', "a quoted string")?;
        let mut value = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\r') | Some(b'\n') | Some(0) => {
                    return Err("unterminated quoted string".into());
                }
                Some(b'"') => {
                    self.position += 1;
                    return Ok(value);
                }
                Some(b'\\') => {
                    self.position += 1;
                    match self.peek() {
                        Some(escaped @ (b'"' | b'\\')) => {
                            value.push(escaped);
                            self.position += 1;
                        }
                        _ => return Err("invalid escape in quoted string".into()),
                    }
                }
                Some(byte) => {
                    value.push(byte);
                    self.position += 1;
                }
            }
        }
    }

    /// Read a literal whose octets follow its `{n}` CRLF announcement inline.
    pub(super) fn literal(&mut self) -> ParseResult<Vec<u8>> {
        self.expect(b'{', "a literal")?;
        let length = self.number()? as usize;
        if self.peek() == Some(b'+') {
            self.position += 1;
        }
        self.expect(b'}', "a closing brace")?;
        self.expect(b'\r', "a carriage return before the literal line break")?;
        self.expect(b'\n', "a line break after the literal length")?;
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.input.len())
            .ok_or_else(|| "literal is shorter than announced".to_owned())?;
        let value = &self.input[self.position..end];
        if value.contains(&0) {
            return Err("literal contains NUL".into());
        }
        let value = value.to_vec();
        self.position = end;
        Ok(value)
    }

    /// Read `string`: a quoted string or a literal.
    pub(super) fn string(&mut self) -> ParseResult<Vec<u8>> {
        match self.peek() {
            Some(b'"') => self.quoted(),
            Some(b'{') => self.literal(),
            _ => Err("expected a string".into()),
        }
    }

    /// Read `astring`: an atom-like run of astring characters, or a string.
    pub(super) fn astring(&mut self) -> ParseResult<Vec<u8>> {
        match self.peek() {
            Some(b'"') | Some(b'{') => self.string(),
            _ => {
                let value = self.take_while(is_astring_byte);
                if value.is_empty() {
                    Err("expected an astring".into())
                } else {
                    Ok(value.to_vec())
                }
            }
        }
    }

    /// Read an astring that must be valid UTF-8 text.
    pub(super) fn text_astring(&mut self) -> ParseResult<String> {
        String::from_utf8(self.astring()?).map_err(|_| "argument is not valid UTF-8".to_owned())
    }

    /// Read `nstring`: NIL or a string.
    pub(super) fn nstring(&mut self) -> ParseResult<Option<Vec<u8>>> {
        if self.peek() == Some(b'"') || self.peek() == Some(b'{') {
            return self.string().map(Some);
        }
        let atom = self.atom()?;
        if atom.eq_ignore_ascii_case("NIL") {
            Ok(None)
        } else {
            Err("expected a string or NIL".into())
        }
    }

    /// Read a mailbox name and decode it from modified UTF-7.
    pub(super) fn mailbox(&mut self) -> ParseResult<String> {
        let raw = self.astring()?;
        let name = mailbox_name::decode(&raw).ok_or_else(|| "invalid mailbox name".to_owned())?;
        Ok(if mailbox_name::is_inbox(&name) {
            "INBOX".to_owned()
        } else {
            name
        })
    }

    /// Read `list-mailbox`: list characters including wildcards, or a string.
    pub(super) fn list_mailbox(&mut self) -> ParseResult<String> {
        let raw = match self.peek() {
            Some(b'"') | Some(b'{') => self.string()?,
            _ => {
                let value = self.take_while(is_list_byte);
                if value.is_empty() {
                    return Err("expected a mailbox pattern".into());
                }
                value.to_vec()
            }
        };
        mailbox_name::decode(&raw).ok_or_else(|| "invalid mailbox pattern".to_owned())
    }

    /// Read a sequence set token.
    pub(super) fn sequence_set(&mut self) -> ParseResult<SequenceSet> {
        let token =
            self.take_while(|byte| byte.is_ascii_digit() || matches!(byte, b':' | b',' | b'*'));
        std::str::from_utf8(token)
            .ok()
            .and_then(SequenceSet::parse)
            .ok_or_else(|| "invalid sequence set".to_owned())
    }
}
