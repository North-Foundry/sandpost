//! ENVELOPE construction (RFC 3501 section 7.4.2) from raw RFC 5322 header fields.
//!
//! Values are taken from the header octets without decoding RFC 2047 encoded words, as the
//! envelope is defined over the header text; clients decode them. Address lists are parsed with
//! the RFC 5322 grammar, including groups, source routes, quoted local parts, and comments.
use crate::message::headers::{HeaderField, metadata_value, unquote};
use crate::protocol::response::Output;

/// Host written for an address without a domain, so it is not mistaken for group syntax.
const MISSING_DOMAIN: &[u8] = b"MISSING_DOMAIN";

/// One element of an envelope address list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressItem {
    /// A mailbox: display name, source route, local part, and domain.
    Mailbox {
        name: Option<Vec<u8>>,
        route: Option<Vec<u8>>,
        local: Vec<u8>,
        domain: Option<Vec<u8>>,
    },
    /// The start of a group, with its display name.
    GroupStart(Vec<u8>),
    /// The end of the current group.
    GroupEnd,
}

/// Write the ENVELOPE of an entity's header fields.
pub fn write_envelope(output: &mut Output, message: &[u8], fields: &[HeaderField]) {
    let value = |name: &str| metadata_value(message, fields, name);
    let addresses = |name: &str| {
        value(name)
            .map(|raw| parse_address_list(&raw))
            .filter(|items| !items.is_empty())
    };
    let from = addresses("from");
    let sender = addresses("sender").or_else(|| from.clone());
    let reply_to = addresses("reply-to").or_else(|| from.clone());
    output.raw("(");
    output.nstring(value("date").as_deref()).raw(" ");
    output.nstring(value("subject").as_deref()).raw(" ");
    write_address_list(output, from.as_deref());
    output.raw(" ");
    write_address_list(output, sender.as_deref());
    output.raw(" ");
    write_address_list(output, reply_to.as_deref());
    output.raw(" ");
    write_address_list(output, addresses("to").as_deref());
    output.raw(" ");
    write_address_list(output, addresses("cc").as_deref());
    output.raw(" ");
    write_address_list(output, addresses("bcc").as_deref());
    output.raw(" ");
    output.nstring(value("in-reply-to").as_deref()).raw(" ");
    output.nstring(value("message-id").as_deref());
    output.raw(")");
}

/// Write a parenthesized address list, or NIL when absent.
fn write_address_list(output: &mut Output, items: Option<&[AddressItem]>) {
    let Some(items) = items else {
        output.raw("NIL");
        return;
    };
    output.raw("(");
    for item in items {
        match item {
            AddressItem::Mailbox {
                name,
                route,
                local,
                domain,
            } => {
                output.raw("(");
                output.nstring(name.as_deref()).raw(" ");
                output.nstring(route.as_deref()).raw(" ");
                output.string(local).raw(" ");
                output.string(domain.as_deref().unwrap_or(MISSING_DOMAIN));
                output.raw(")");
            }
            AddressItem::GroupStart(name) => {
                output.raw("(NIL NIL ").string(name).raw(" NIL)");
            }
            AddressItem::GroupEnd => {
                output.raw("(NIL NIL NIL NIL)");
            }
        }
    }
    output.raw(")");
}

/// A lexical token of an RFC 5322 address list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Atom(Vec<u8>),
    Quoted(Vec<u8>),
    Comment(Vec<u8>),
    DomainLiteral(Vec<u8>),
    Special(u8),
}

/// Split an unfolded address-list value into tokens.
fn tokenize(raw: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        let byte = raw[index];
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'"' => {
                let start = index;
                index += 1;
                while index < raw.len() && raw[index] != b'"' {
                    index += if raw[index] == b'\\' { 2 } else { 1 };
                }
                index = (index + 1).min(raw.len());
                tokens.push(Token::Quoted(unquote(&raw[start..index])));
            }
            b'(' => {
                let start = index + 1;
                let mut depth = 1;
                index += 1;
                while index < raw.len() && depth > 0 {
                    match raw[index] {
                        b'\\' => index += 1,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    index += 1;
                }
                let end = if depth == 0 { index - 1 } else { raw.len() };
                tokens.push(Token::Comment(raw[start..end.max(start)].to_vec()));
            }
            b'[' => {
                let start = index;
                while index < raw.len() && raw[index] != b']' {
                    index += if raw[index] == b'\\' { 2 } else { 1 };
                }
                index = (index + 1).min(raw.len());
                tokens.push(Token::DomainLiteral(raw[start..index].to_vec()));
            }
            b'<' | b'>' | b',' | b':' | b';' | b'@' | b'.' => {
                tokens.push(Token::Special(byte));
                index += 1;
            }
            _ => {
                let start = index;
                while index < raw.len()
                    && !matches!(
                        raw[index],
                        b' ' | b'\t'
                            | b'\r'
                            | b'\n'
                            | b'"'
                            | b'('
                            | b'['
                            | b'<'
                            | b'>'
                            | b','
                            | b':'
                            | b';'
                            | b'@'
                            | b'.'
                    )
                {
                    index += 1;
                }
                tokens.push(Token::Atom(raw[start..index].to_vec()));
            }
        }
    }
    tokens
}

/// Parse an address list into envelope items, skipping elements that contain no address.
pub fn parse_address_list(raw: &[u8]) -> Vec<AddressItem> {
    let tokens = tokenize(raw);
    let mut items = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index] == Token::Special(b',') {
            index += 1;
            continue;
        }
        if let Some(colon) = group_colon(&tokens[index..]) {
            let name = phrase(&tokens[index..index + colon]);
            items.push(AddressItem::GroupStart(name));
            index += colon + 1;
            while index < tokens.len() && tokens[index] != Token::Special(b';') {
                let end = element_end(&tokens, index);
                if let Some(mailbox) = parse_mailbox(&tokens[index..end]) {
                    items.push(mailbox);
                }
                index = end;
                if index < tokens.len() && tokens[index] == Token::Special(b',') {
                    index += 1;
                }
            }
            index += 1;
            items.push(AddressItem::GroupEnd);
            continue;
        }
        let end = element_end(&tokens, index);
        if let Some(mailbox) = parse_mailbox(&tokens[index..end]) {
            items.push(mailbox);
        }
        index = end.max(index + 1);
    }
    items
}

/// Find a group's colon: one that appears before any angle bracket, at-sign, comma, or
/// semicolon.
fn group_colon(tokens: &[Token]) -> Option<usize> {
    for (position, token) in tokens.iter().enumerate() {
        match token {
            Token::Special(b':') => return Some(position),
            Token::Special(b'<' | b'@' | b',' | b';') => return None,
            _ => {}
        }
    }
    None
}

/// The index after one list element: the next top-level comma or semicolon.
fn element_end(tokens: &[Token], start: usize) -> usize {
    let mut in_angle = false;
    for (offset, token) in tokens[start..].iter().enumerate() {
        match token {
            Token::Special(b'<') => in_angle = true,
            Token::Special(b'>') => in_angle = false,
            Token::Special(b',' | b';') if !in_angle => return start + offset,
            _ => {}
        }
    }
    tokens.len()
}

/// Parse one mailbox: `[phrase] <[route:]addr-spec>` or a bare `addr-spec` with an optional
/// comment used as its display name.
fn parse_mailbox(tokens: &[Token]) -> Option<AddressItem> {
    if let Some(open) = tokens
        .iter()
        .position(|token| *token == Token::Special(b'<'))
    {
        let close = tokens[open..]
            .iter()
            .position(|token| *token == Token::Special(b'>'))
            .map_or(tokens.len(), |offset| open + offset);
        let inside = &tokens[open + 1..close];
        let (route, address) = match inside
            .iter()
            .position(|token| *token == Token::Special(b':'))
        {
            Some(colon) => (Some(route_text(&inside[..colon])), &inside[colon + 1..]),
            None => (None, inside),
        };
        let (local, domain) = address_spec(address)?;
        let name = phrase(&tokens[..open]);
        return Some(AddressItem::Mailbox {
            name: (!name.is_empty()).then_some(name),
            route: route.filter(|route| !route.is_empty()),
            local,
            domain,
        });
    }
    let (local, domain) = address_spec(tokens)?;
    let comment = tokens.iter().rev().find_map(|token| match token {
        Token::Comment(text) if !text.trim_ascii().is_empty() => Some(text.trim_ascii().to_vec()),
        _ => None,
    });
    Some(AddressItem::Mailbox {
        name: comment,
        route: None,
        local,
        domain,
    })
}

/// Parse `local-part ["@" domain]`, ignoring comments; `None` when there is no local part.
fn address_spec(tokens: &[Token]) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let words: Vec<&Token> = tokens
        .iter()
        .filter(|token| !matches!(token, Token::Comment(_)))
        .collect();
    let at = words
        .iter()
        .rposition(|token| **token == Token::Special(b'@'));
    let (local_tokens, domain_tokens) = match at {
        Some(at) => (&words[..at], Some(&words[at + 1..])),
        None => (&words[..], None),
    };
    let local = join_words(local_tokens);
    if local.is_empty() && !matches!(local_tokens, [Token::Quoted(value)] if value.is_empty()) {
        return None;
    }
    let domain = domain_tokens
        .map(join_words)
        .filter(|domain| !domain.is_empty());
    Some((local, domain))
}

/// Join atoms, quoted strings, domain literals, and dots without separators.
fn join_words(tokens: &[&Token]) -> Vec<u8> {
    let mut joined = Vec::new();
    for token in tokens {
        match token {
            Token::Atom(text) | Token::Quoted(text) | Token::DomainLiteral(text) => {
                joined.extend_from_slice(text)
            }
            Token::Special(b'.') => joined.push(b'.'),
            _ => {}
        }
    }
    joined
}

/// Render a source route such as `@a,@b` from the tokens before its colon.
fn route_text(tokens: &[Token]) -> Vec<u8> {
    let mut route = Vec::new();
    for token in tokens {
        match token {
            Token::Atom(text) | Token::DomainLiteral(text) => route.extend_from_slice(text),
            Token::Special(byte @ (b'@' | b',' | b'.')) => route.push(*byte),
            _ => {}
        }
    }
    route
}

/// Render a display-name phrase: words separated by single spaces, comments omitted.
fn phrase(tokens: &[Token]) -> Vec<u8> {
    let mut words: Vec<Vec<u8>> = Vec::new();
    for token in tokens {
        match token {
            Token::Atom(text) | Token::Quoted(text) => words.push(text.clone()),
            Token::Special(b'.') => match words.last_mut() {
                Some(last) => last.push(b'.'),
                None => words.push(b".".to_vec()),
            },
            _ => {}
        }
    }
    words.join(&b' ')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::structure::parse_message;

    /// Build a mailbox item for comparisons.
    fn mailbox(name: Option<&str>, local: &str, domain: Option<&str>) -> AddressItem {
        AddressItem::Mailbox {
            name: name.map(|name| name.as_bytes().to_vec()),
            route: None,
            local: local.as_bytes().to_vec(),
            domain: domain.map(|domain| domain.as_bytes().to_vec()),
        }
    }

    /// Display names, quoted strings, comments, and bare addresses parse as RFC 5322 mailboxes.
    #[test]
    fn mailboxes_and_display_names() {
        let items = parse_address_list(
            b"\"Smith, John\" <john@example.com>, jane@example.com (Jane Doe), <bare@x.y>, Fred Q. Public <fred@x>",
        );
        assert_eq!(
            items,
            [
                mailbox(Some("Smith, John"), "john", Some("example.com")),
                mailbox(Some("Jane Doe"), "jane", Some("example.com")),
                mailbox(None, "bare", Some("x.y")),
                mailbox(Some("Fred Q. Public"), "fred", Some("x")),
            ]
        );
    }

    /// Groups produce start and end markers around their members, including empty groups.
    #[test]
    fn groups_are_bracketed() {
        let items = parse_address_list(b"Team: a@x, b@y;, undisclosed-recipients:;");
        assert_eq!(
            items,
            [
                AddressItem::GroupStart(b"Team".to_vec()),
                mailbox(None, "a", Some("x")),
                mailbox(None, "b", Some("y")),
                AddressItem::GroupEnd,
                AddressItem::GroupStart(b"undisclosed-recipients".to_vec()),
                AddressItem::GroupEnd,
            ]
        );
    }

    /// Source routes, quoted local parts, and missing domains are preserved.
    #[test]
    fn routes_and_unusual_addresses() {
        assert_eq!(
            parse_address_list(b"<@relay.a,@relay.b:joe@final.example>"),
            [AddressItem::Mailbox {
                name: None,
                route: Some(b"@relay.a,@relay.b".to_vec()),
                local: b"joe".to_vec(),
                domain: Some(b"final.example".to_vec()),
            }]
        );
        assert_eq!(
            parse_address_list(b"\"john doe\"@example.com"),
            [mailbox(None, "john doe", Some("example.com"))]
        );
        assert_eq!(
            parse_address_list(b"postmaster"),
            [mailbox(None, "postmaster", None)]
        );
        assert!(parse_address_list(b"").is_empty());
        assert!(parse_address_list(b",,").is_empty());
    }

    /// The envelope follows field order and defaults Sender and Reply-To to From.
    #[test]
    fn envelope_defaults_sender_and_reply_to() {
        let message = b"Date: Mon, 7 Feb 1994 21:52:25 -0800\r\nFrom: Fred <fred@x.example>\r\nSubject: =?utf-8?q?caf=C3=A9?=\r\nTo: a@b, Group:;\r\nMessage-ID: <1@x>\r\n\r\nbody";
        let root = parse_message(message);
        let mut output = Output::new();
        write_envelope(&mut output, message, &root.fields);
        assert_eq!(
            String::from_utf8(output.as_bytes().to_vec()).unwrap(),
            "(\"Mon, 7 Feb 1994 21:52:25 -0800\" \"=?utf-8?q?caf=C3=A9?=\" ((\"Fred\" NIL \"fred\" \"x.example\")) ((\"Fred\" NIL \"fred\" \"x.example\")) ((\"Fred\" NIL \"fred\" \"x.example\")) ((NIL NIL \"a\" \"b\")(NIL NIL \"Group\" NIL)(NIL NIL NIL NIL)) NIL NIL NIL \"<1@x>\")"
        );
    }

    /// A header without addresses or dates yields NIL members.
    #[test]
    fn missing_fields_are_nil() {
        let message = b"X-Other: 1\r\n\r\n";
        let root = parse_message(message);
        let mut output = Output::new();
        write_envelope(&mut output, message, &root.fields);
        assert_eq!(
            output.as_bytes(),
            b"(NIL NIL NIL NIL NIL NIL NIL NIL NIL NIL)"
        );
    }

    /// Explicit empty quoted local parts remain mailboxes and supply Sender/Reply-To defaults.
    #[test]
    fn empty_quoted_local_parts_are_preserved() {
        assert_eq!(
            parse_address_list(b"\"\"@example.test"),
            [mailbox(None, "", Some("example.test"))]
        );
        assert!(parse_address_list(b"@example.test").is_empty());
        let raw = b"From: <\"\"@example.test>\r\n\r\n";
        let mut output = Output::new();
        write_envelope(&mut output, raw, &parse_message(raw).fields);
        assert_eq!(output.as_bytes(), b"(NIL NIL ((NIL NIL \"\" \"example.test\")) ((NIL NIL \"\" \"example.test\")) ((NIL NIL \"\" \"example.test\")) NIL NIL NIL NIL NIL)");
    }

    /// Encoded NUL remains encoded ASCII; literal NUL invalidates only its metadata field.
    #[test]
    fn envelope_metadata_is_nul_free() {
        let raw = b"Subject: =?utf-8?B?AA==?=\r\nFrom: =?utf-8?B?AA==?= <a@b>\r\nMessage-ID: a\0b\r\n\r\n";
        let mut output = Output::new();
        write_envelope(&mut output, raw, &parse_message(raw).fields);
        assert!(!output.as_bytes().contains(&0));
        assert!(String::from_utf8_lossy(output.as_bytes()).contains("=?utf-8?B?AA==?="));
        assert!(output.as_bytes().ends_with(b"NIL)"));
    }
}
