//! Parsing for FETCH attributes, body sections, and header field selectors.

use super::cursor::{Cursor, ParseResult};
use crate::protocol::command::{CommandKind, FetchAttribute, Section, SectionText};

/// Most header field names in one HEADER.FIELDS list.
const MAXIMUM_HEADER_FIELDS: usize = 64;
/// Most nested part numbers in one section.
const MAXIMUM_PART_DEPTH: usize = 16;

impl<'input> Cursor<'input> {
    /// Read `[UID] FETCH set (items)`, a single item, or a macro.
    pub(super) fn fetch(&mut self, uid: bool) -> ParseResult<CommandKind> {
        self.space()?;
        let set = self.sequence_set()?;
        self.space()?;
        let mut attributes = Vec::new();
        if self.peek() == Some(b'(') {
            self.position += 1;
            loop {
                attributes.push(self.fetch_attribute()?);
                match self.peek() {
                    Some(b' ') => self.position += 1,
                    Some(b')') => {
                        self.position += 1;
                        break;
                    }
                    _ => return Err("expected a space or closing parenthesis".into()),
                }
                if attributes.len() > 64 {
                    return Err("too many FETCH items".into());
                }
            }
        } else {
            let start = self.position;
            let name = self.atom_or_section_name()?;
            match name.to_ascii_uppercase().as_str() {
                "ALL" => attributes.extend([
                    FetchAttribute::Flags,
                    FetchAttribute::InternalDate,
                    FetchAttribute::Rfc822Size,
                    FetchAttribute::Envelope,
                ]),
                "FAST" => attributes.extend([
                    FetchAttribute::Flags,
                    FetchAttribute::InternalDate,
                    FetchAttribute::Rfc822Size,
                ]),
                "FULL" => attributes.extend([
                    FetchAttribute::Flags,
                    FetchAttribute::InternalDate,
                    FetchAttribute::Rfc822Size,
                    FetchAttribute::Envelope,
                    FetchAttribute::Body,
                ]),
                _ => {
                    self.position = start;
                    attributes.push(self.fetch_attribute()?);
                }
            }
        }
        Ok(CommandKind::Fetch {
            uid,
            set,
            attributes,
        })
    }

    /// Read a FETCH item name such as `BODY.PEEK` up to a bracket, space, or parenthesis.
    pub(super) fn atom_or_section_name(&mut self) -> ParseResult<String> {
        let name = self.take_while(|byte| byte.is_ascii_alphanumeric() || byte == b'.');
        if name.is_empty() {
            return Err("expected a FETCH item".into());
        }
        Ok(String::from_utf8_lossy(name).into_owned())
    }

    /// Read one FETCH data item.
    pub(super) fn fetch_attribute(&mut self) -> ParseResult<FetchAttribute> {
        let name = self.atom_or_section_name()?.to_ascii_uppercase();
        let attribute = match name.as_str() {
            "ENVELOPE" => FetchAttribute::Envelope,
            "FLAGS" => FetchAttribute::Flags,
            "INTERNALDATE" => FetchAttribute::InternalDate,
            "RFC822.SIZE" => FetchAttribute::Rfc822Size,
            "UID" => FetchAttribute::Uid,
            "BODYSTRUCTURE" => FetchAttribute::BodyStructure,
            "RFC822" => FetchAttribute::Rfc822,
            "RFC822.HEADER" => FetchAttribute::Rfc822Header,
            "RFC822.TEXT" => FetchAttribute::Rfc822Text,
            "BODY" if self.peek() != Some(b'[') => FetchAttribute::Body,
            "BODY" | "BODY.PEEK" => {
                let section = self.section()?;
                let partial = if self.peek() == Some(b'<') {
                    self.position += 1;
                    let origin = self.number()?;
                    self.expect(b'.', "a partial length")?;
                    let length = self.number()?;
                    if length == 0 {
                        return Err("partial length must be non-zero".into());
                    }
                    self.expect(b'>', "a closing angle bracket")?;
                    Some((origin, length))
                } else {
                    None
                };
                FetchAttribute::Section {
                    peek: name == "BODY.PEEK",
                    section,
                    partial,
                }
            }
            _ => return Err(format!("unsupported FETCH item {name}")),
        };
        Ok(attribute)
    }

    /// Read `section`: `[` [part path] [text selector] `]`.
    pub(super) fn section(&mut self) -> ParseResult<Section> {
        self.expect(b'[', "a section")?;
        let mut part = Vec::new();
        let mut text = None;
        loop {
            if self.peek() == Some(b']') {
                break;
            }
            if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                if text.is_some() {
                    return Err("part numbers must precede the section text".into());
                }
                let number = self.number()?;
                if number == 0 {
                    return Err("part numbers start at 1".into());
                }
                part.push(number);
                if part.len() > MAXIMUM_PART_DEPTH {
                    return Err("section nesting is too deep".into());
                }
                match self.peek() {
                    Some(b'.') => {
                        self.position += 1;
                        continue;
                    }
                    Some(b']') => break,
                    _ => return Err("invalid section".into()),
                }
            }
            let selector = self
                .take_while(|byte| byte.is_ascii_alphabetic() || byte == b'.')
                .to_ascii_uppercase();
            text = Some(match selector.as_slice() {
                b"HEADER" => SectionText::Header,
                b"TEXT" => SectionText::Text,
                b"MIME" if !part.is_empty() => SectionText::Mime,
                b"HEADER.FIELDS" | b"HEADER.FIELDS.NOT" => {
                    self.space()?;
                    SectionText::HeaderFields {
                        not: selector.ends_with(b".NOT"),
                        fields: self.header_list()?,
                    }
                }
                _ => return Err("invalid section text".into()),
            });
            break;
        }
        self.expect(b']', "a closing bracket")?;
        Ok(Section { part, text })
    }

    /// Read a parenthesized list of header field names.
    pub(super) fn header_list(&mut self) -> ParseResult<Vec<String>> {
        self.expect(b'(', "a header field list")?;
        let mut fields = Vec::new();
        loop {
            let field = self.text_astring()?;
            if field.is_empty()
                || !field
                    .bytes()
                    .all(|byte| (0x21..=0x7e).contains(&byte) && byte != b':')
            {
                return Err("invalid header field name".into());
            }
            fields.push(field);
            if fields.len() > MAXIMUM_HEADER_FIELDS {
                return Err("too many header field names".into());
            }
            match self.peek() {
                Some(b' ') => self.position += 1,
                Some(b')') => {
                    self.position += 1;
                    return Ok(fields);
                }
                _ => return Err("expected a space or closing parenthesis".into()),
            }
        }
    }
}
