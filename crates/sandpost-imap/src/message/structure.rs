//! MIME entity tree with exact byte ranges, and IMAP body-section resolution.
//!
//! IMAP addresses message parts by number (RFC 3501 section 6.4.5) and returns their octets
//! unchanged, so the tree records where every header and body starts and ends in the original
//! message instead of decoding content. Recursive parsing is bounded to depth 32 and 2048
//! entities. Composite entities exceeding either limit become application/octet-stream leaves
//! over their entire original body: nested sections and SEARCH content there are unavailable,
//! but raw headers/bodies and complete, valid BODY/BODYSTRUCTURE output remain available.
use crate::message::headers::{
    HeaderField, StructuredValue, first_value, parse_fields, transfer_encoding,
};
use crate::protocol::command::{Section, SectionText};
use std::ops::Range;

/// Deepest nesting of multipart and encapsulated messages that is parsed; deeper content is
/// treated as an opaque single part.
const MAXIMUM_DEPTH: usize = 32;
/// Most entities parsed in one message, including the root and opaque fallback entities.
const MAXIMUM_ENTITY_COUNT: usize = 2048;

/// One MIME entity: a header block, a body, and nested entities.
#[derive(Debug, Clone)]
pub struct Entity {
    /// The header block including its terminating blank line when there is one.
    pub header: Range<usize>,
    /// The body octets, still transfer-encoded.
    pub body: Range<usize>,
    /// Header fields of this entity.
    pub fields: Vec<HeaderField>,
    /// Lowercase media type, such as `text`.
    pub media_type: String,
    /// Lowercase media subtype, such as `plain`.
    pub media_subtype: String,
    /// Content-Type parameters with lowercase names.
    pub parameters: Vec<(String, Vec<u8>)>,
    /// Lowercase, comment-free Content-Transfer-Encoding, defaulting to 7bit.
    pub transfer_encoding: String,
    pub children: Children,
}

/// What an entity contains.
#[derive(Debug, Clone)]
pub enum Children {
    /// A leaf part.
    None,
    /// The parts of a multipart entity, in order.
    Multipart(Vec<Entity>),
    /// The encapsulated message of a `message/rfc822` entity.
    Message(Box<Entity>),
}

/// Parse a complete message into its entity tree.
pub fn parse_message(message: &[u8]) -> Entity {
    let mut budget = MAXIMUM_ENTITY_COUNT;
    parse_entity(message, 0..message.len(), false, 0, &mut budget)
}

/// Parse the entity occupying `range`; `digest_child` selects the `multipart/digest` default.
fn parse_entity(
    message: &[u8],
    range: Range<usize>,
    digest_child: bool,
    depth: usize,
    budget: &mut usize,
) -> Entity {
    *budget = budget.saturating_sub(1);
    let header_end = header_end(message, range.clone());
    let header = range.start..header_end;
    let fields = parse_fields(message, header.clone());
    let content_type = first_value(message, &fields, "content-type")
        .map(|raw| StructuredValue::parse(&raw))
        .filter(|value| {
            value
                .value
                .split_once('/')
                .is_some_and(|(media_type, subtype)| !media_type.is_empty() && !subtype.is_empty())
        });
    let (media_type, media_subtype, parameters) = match content_type {
        Some(value) => {
            let (media_type, subtype) = value.value.split_once('/').expect("filtered");
            (media_type.to_owned(), subtype.to_owned(), value.parameters)
        }
        None if digest_child => ("message".to_owned(), "rfc822".to_owned(), Vec::new()),
        None => (
            "text".to_owned(),
            "plain".to_owned(),
            vec![("charset".to_owned(), b"us-ascii".to_vec())],
        ),
    };
    let body = header_end..range.end;
    let transfer_encoding = transfer_encoding(message, &fields);
    let mut entity = Entity {
        header,
        body: body.clone(),
        fields,
        media_type,
        media_subtype,
        parameters,
        transfer_encoding,
        children: Children::None,
    };
    if depth >= MAXIMUM_DEPTH || *budget == 0 {
        if entity.media_type == "multipart"
            || (entity.media_type == "message"
                && matches!(entity.media_subtype.as_str(), "rfc822" | "global"))
        {
            make_opaque(&mut entity);
        }
        return entity;
    }
    if entity.media_type == "multipart" {
        let boundary = entity
            .parameters
            .iter()
            .find(|(name, _)| name == "boundary")
            .map(|(_, value)| value.clone())
            .filter(|boundary| !boundary.is_empty());
        let digest = entity.media_subtype == "digest";
        let parts = match boundary {
            Some(boundary) => {
                let Some(parts) = split_multipart(message, body.clone(), &boundary, *budget) else {
                    make_opaque(&mut entity);
                    return entity;
                };
                parts
            }
            None => Vec::new(),
        };
        let children = if parts.is_empty() {
            // A multipart body needs at least one part; treat an unparsable one as one empty
            // text part at the end of the body.
            *budget -= 1;
            vec![Entity {
                header: body.end..body.end,
                body: body.end..body.end,
                fields: Vec::new(),
                media_type: "text".to_owned(),
                media_subtype: "plain".to_owned(),
                parameters: vec![("charset".to_owned(), b"us-ascii".to_vec())],
                transfer_encoding: "7bit".to_owned(),
                children: Children::None,
            }]
        } else {
            let part_count = parts.len();
            let mut children = Vec::with_capacity(part_count);
            for (index, part) in parts.into_iter().enumerate() {
                // Reserve one entity for every later sibling before descending into this one.
                let reserved = part_count - index - 1;
                let mut child_budget = *budget - reserved;
                children.push(parse_entity(
                    message,
                    part,
                    digest,
                    depth + 1,
                    &mut child_budget,
                ));
                *budget = child_budget + reserved;
            }
            children
        };
        entity.children = Children::Multipart(children);
    } else if entity.media_type == "message"
        && matches!(entity.media_subtype.as_str(), "rfc822" | "global")
    {
        if matches!(
            entity.transfer_encoding.as_str(),
            "base64" | "quoted-printable"
        ) {
            // Encoded embedded messages cannot use ranges into the undecoded source.
            make_opaque(&mut entity);
            return entity;
        }
        entity.children = Children::Message(Box::new(parse_entity(
            message,
            body,
            false,
            depth + 1,
            budget,
        )));
    }
    entity
}

/// Replace unavailable composite structure by a valid opaque part, preserving all raw ranges.
fn make_opaque(entity: &mut Entity) {
    entity.media_type = "application".to_owned();
    entity.media_subtype = "octet-stream".to_owned();
    entity.parameters.clear();
    entity.children = Children::None;
}

/// Find where the header block ends: just after the first empty line, or at the end of the
/// range when there is no empty line (the whole entity is header).
fn header_end(message: &[u8], range: Range<usize>) -> usize {
    let mut position = range.start;
    while position < range.end {
        let line_end = message[position..range.end]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(range.end, |offset| position + offset + 1);
        let line = &message[position..line_end];
        if line == b"\r\n" || line == b"\n" {
            return line_end;
        }
        position = line_end;
    }
    range.end
}

/// Split a multipart body into its part ranges (RFC 2046 section 5.1.1).
///
/// The line break before each delimiter belongs to the delimiter, so parts exclude it. The
/// preamble and epilogue are not parts. A missing close delimiter ends the last part at the end
/// of the body.
/// Return None before allocating a part range beyond `maximum_parts`.
fn split_multipart(
    message: &[u8],
    body: Range<usize>,
    boundary: &[u8],
    maximum_parts: usize,
) -> Option<Vec<Range<usize>>> {
    let mut parts = Vec::new();
    let mut current_start: Option<usize> = None;
    let mut position = body.start;
    while position < body.end {
        let line_end = message[position..body.end]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(body.end, |offset| position + offset + 1);
        let line = &message[position..line_end];
        let content = line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"))
            .unwrap_or(line);
        if let Some(rest) = content.strip_prefix(b"--")
            && let Some(after) = rest.strip_prefix(boundary)
        {
            let closing = after.starts_with(b"--");
            let after = if closing { &after[2..] } else { after };
            if after.iter().all(|byte| matches!(byte, b' ' | b'\t')) {
                if let Some(start) = current_start.take() {
                    if parts.len() == maximum_parts {
                        return None;
                    }
                    parts.push(start..delimiter_start(message, start, position));
                }
                if closing {
                    return Some(parts);
                }
                current_start = Some(line_end);
            }
        }
        position = line_end;
    }
    if let Some(start) = current_start {
        if parts.len() == maximum_parts {
            return None;
        }
        parts.push(start..body.end);
    }
    Some(parts)
}

/// The start of a delimiter including the line break that precedes it, never before `floor`.
fn delimiter_start(message: &[u8], floor: usize, delimiter_line: usize) -> usize {
    let mut start = delimiter_line;
    if start > floor && message[start - 1] == b'\n' {
        start -= 1;
        if start > floor && message[start - 1] == b'\r' {
            start -= 1;
        }
    }
    start
}

/// Resolve a numeric part path to its entity (RFC 3501 section 6.4.5 part numbering).
///
/// At the top level, a non-multipart message has only part 1, the message itself. Inside a
/// `message/rfc822` part, numbers address the parts of the encapsulated message.
pub fn resolve_part<'tree>(root: &'tree Entity, path: &[u32]) -> Option<&'tree Entity> {
    let mut current = root;
    for (depth, number) in path.iter().enumerate() {
        let index = usize::try_from(*number).ok()?.checked_sub(1)?;
        // The entity whose numbering applies, and whether it is a whole message: the top-level
        // message itself, or the message encapsulated by a `message/rfc822` part.
        let (container, is_message) = match &current.children {
            _ if depth == 0 => (current, true),
            Children::Message(inner) => (inner.as_ref(), true),
            _ => (current, false),
        };
        current = match &container.children {
            Children::Multipart(children) => children.get(index)?,
            // A non-multipart message has exactly one part: its own body.
            _ if is_message && index == 0 => container,
            _ => return None,
        };
    }
    Some(current)
}

/// The octets a section selects, or `None` when the section does not exist in this message.
pub fn section_bytes(message: &[u8], root: &Entity, section: &Section) -> Option<Vec<u8>> {
    let part = resolve_part(root, &section.part)?;
    match &section.text {
        None if section.part.is_empty() => Some(message.to_vec()),
        None => Some(message[part.body.clone()].to_vec()),
        Some(SectionText::Mime) => Some(message[part.header.clone()].to_vec()),
        Some(text) => {
            // HEADER and TEXT address a message: the whole message, or an encapsulated one.
            let target = if section.part.is_empty() {
                part
            } else {
                match &part.children {
                    Children::Message(inner) => inner,
                    _ => return None,
                }
            };
            Some(match text {
                SectionText::Header => message[target.header.clone()].to_vec(),
                SectionText::Text => message[target.body.clone()].to_vec(),
                SectionText::HeaderFields { not, fields } => {
                    select_fields(message, target, fields, *not)
                }
                SectionText::Mime => unreachable!("handled above"),
            })
        }
    }
}

/// Return the header fields whose names do (or, with `exclude`, do not) match, followed by the
/// blank line when the header has one.
fn select_fields(message: &[u8], entity: &Entity, names: &[String], exclude: bool) -> Vec<u8> {
    let mut selected = Vec::new();
    for field in &entity.fields {
        let listed = names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&field.name));
        if listed != exclude {
            selected.extend_from_slice(&message[field.range.clone()]);
            if !selected.ends_with(b"\n") {
                selected.extend_from_slice(b"\r\n");
            }
        }
    }
    let header = &message[entity.header.clone()];
    if header.ends_with(b"\n\n") || header.ends_with(b"\r\n\r\n") || header == b"\r\n" {
        selected.extend_from_slice(b"\r\n");
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message with nested multiparts and an encapsulated message (RFC 3501 example shape).
    const COMPLEX: &[u8] = b"From: a@example.test\r\n\
Content-Type: multipart/mixed; boundary=outer\r\n\
\r\n\
preamble\r\n\
--outer\r\n\
Content-Type: text/plain\r\n\
\r\n\
part one\r\n\
--outer\r\n\
Content-Type: message/rfc822\r\n\
\r\n\
Subject: inner\r\n\
Content-Type: multipart/alternative; boundary=inner\r\n\
\r\n\
--inner\r\n\
\r\n\
inner text\r\n\
--inner\r\n\
Content-Type: text/html\r\n\
\r\n\
<p>inner</p>\r\n\
--inner--\r\n\
--outer--\r\n\
epilogue\r\n";

    /// Fetch one section of the complex message as text.
    fn section(path: &[u32], text: Option<SectionText>) -> Option<String> {
        let root = parse_message(COMPLEX);
        section_bytes(
            COMPLEX,
            &root,
            &Section {
                part: path.to_vec(),
                text,
            },
        )
        .map(|bytes| String::from_utf8(bytes).unwrap())
    }

    /// Part numbers address multipart children, encapsulated messages, and their parts.
    #[test]
    fn sections_follow_rfc_numbering() {
        assert_eq!(section(&[1], None).unwrap(), "part one");
        assert_eq!(
            section(&[1], Some(SectionText::Mime)).unwrap(),
            "Content-Type: text/plain\r\n\r\n"
        );
        assert!(
            section(&[2], None)
                .unwrap()
                .starts_with("Subject: inner\r\n")
        );
        assert_eq!(
            section(&[2], Some(SectionText::Header)).unwrap(),
            "Subject: inner\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n"
        );
        assert!(
            section(&[2], Some(SectionText::Text))
                .unwrap()
                .starts_with("--inner\r\n")
        );
        assert_eq!(section(&[2, 1], None).unwrap(), "inner text");
        assert_eq!(section(&[2, 2], None).unwrap(), "<p>inner</p>");
        assert!(section(&[3], None).is_none());
        assert!(section(&[1, 1], None).is_none(), "a leaf has no children");
        assert!(
            section(&[1], Some(SectionText::Text)).is_none(),
            "TEXT needs a message part"
        );
        assert!(
            section(&[], Some(SectionText::Text))
                .unwrap()
                .starts_with("preamble")
        );
    }

    /// A single-part message has only part 1, which is its body.
    #[test]
    fn single_part_message_has_part_one() {
        let message = b"Subject: hi\r\n\r\nbody text";
        let root = parse_message(message);
        assert_eq!(resolve_part(&root, &[1]).unwrap().body, root.body);
        assert!(resolve_part(&root, &[2]).is_none());
        assert!(resolve_part(&root, &[1, 1]).is_none());
        let part_one = section_bytes(
            message,
            &root,
            &Section {
                part: vec![1],
                text: None,
            },
        );
        assert_eq!(part_one.unwrap(), b"body text");
        assert_eq!(root.media_type, "text");
    }

    /// HEADER.FIELDS selects whole fields case-insensitively and keeps the blank line.
    #[test]
    fn header_field_subsets_keep_blank_line() {
        let message = b"From: a@b\r\nSubject: one\r\n  two\r\nTo: c@d\r\n\r\nbody";
        let root = parse_message(message);
        let fields = section_bytes(
            message,
            &root,
            &Section {
                part: vec![],
                text: Some(SectionText::HeaderFields {
                    not: false,
                    fields: vec!["subject".into(), "X-Missing".into()],
                }),
            },
        )
        .unwrap();
        assert_eq!(fields, b"Subject: one\r\n  two\r\n\r\n");
        let excluded = section_bytes(
            message,
            &root,
            &Section {
                part: vec![],
                text: Some(SectionText::HeaderFields {
                    not: true,
                    fields: vec!["SUBJECT".into()],
                }),
            },
        )
        .unwrap();
        assert_eq!(excluded, b"From: a@b\r\nTo: c@d\r\n\r\n");
        let headerless = b"Subject: no body";
        let root = parse_message(headerless);
        assert_eq!(root.body, headerless.len()..headerless.len());
    }

    /// Malformed multiparts still expose one part, and missing close delimiters are tolerated.
    #[test]
    fn malformed_multiparts_degrade_gracefully() {
        let without_boundary = b"Content-Type: multipart/mixed\r\n\r\ntext";
        let root = parse_message(without_boundary);
        let Children::Multipart(children) = &root.children else {
            panic!()
        };
        assert_eq!(children.len(), 1);
        let unterminated = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nonly";
        let root = parse_message(unterminated);
        let Children::Multipart(children) = &root.children else {
            panic!()
        };
        assert_eq!(&unterminated[children[0].body.clone()], b"only");
    }

    /// Deeply nested input is bounded instead of recursing without limit.
    #[test]
    fn nesting_depth_is_bounded() {
        let mut message = Vec::new();
        for _ in 0..200 {
            message.extend_from_slice(b"Content-Type: message/rfc822\r\n\r\n");
        }
        message.extend_from_slice(b"Subject: deep\r\n\r\nbody");
        let root = parse_message(&message);
        let mut depth = 0;
        let mut current = &root;
        while let Children::Message(inner) = &current.children {
            current = inner;
            depth += 1;
        }
        assert_eq!(depth, MAXIMUM_DEPTH);
        assert_eq!(current.media_type, "application");
        assert_eq!(current.media_subtype, "octet-stream");
        assert!(message[current.body.clone()].ends_with(b"body"));
    }

    /// Count every allocated entity in a parsed tree, including opaque fallbacks.
    fn entity_count(entity: &Entity) -> usize {
        1 + match &entity.children {
            Children::None => 0,
            Children::Message(inner) => entity_count(inner),
            Children::Multipart(children) => children.iter().map(entity_count).sum(),
        }
    }

    /// Wide multipart overflow preserves the entire body as one opaque part, without truncation.
    #[test]
    fn multipart_entity_count_is_bounded() {
        for (part_count, opaque) in [(MAXIMUM_ENTITY_COUNT - 1, false), (3000, true)] {
            let raw = format!(
                "Content-Type: multipart/mixed; boundary=b\r\n\r\n{}--b--\r\n",
                "--b\r\n\r\nx\r\n".repeat(part_count)
            );
            let root = parse_message(raw.as_bytes());
            assert!(entity_count(&root) <= MAXIMUM_ENTITY_COUNT);
            assert_eq!(root.media_type == "application", opaque);
            assert_eq!(
                section_bytes(
                    raw.as_bytes(),
                    &root,
                    &Section {
                        part: vec![],
                        text: None
                    }
                )
                .unwrap(),
                raw.as_bytes()
            );
            if opaque {
                assert!(matches!(root.children, Children::None));
                assert_eq!(
                    section_bytes(
                        raw.as_bytes(),
                        &root,
                        &Section {
                            part: vec![1],
                            text: None
                        }
                    )
                    .unwrap(),
                    &raw.as_bytes()[root.body.clone()]
                );
            } else {
                assert_eq!(entity_count(&root), MAXIMUM_ENTITY_COUNT);
            }
        }
    }

    /// Range splitting stops before allocating siblings beyond its explicit budget.
    #[test]
    fn multipart_ranges_are_bounded_before_construction() {
        let raw = b"--b\r\n\r\nx\r\n--b\r\n\r\ny\r\n--b\r\n\r\nz\r\n--b--\r\n";
        assert!(split_multipart(raw, 0..raw.len(), b"b", 2).is_none());
        assert_eq!(
            split_multipart(raw, 0..raw.len(), b"b", 3).unwrap().len(),
            3
        );
    }

    /// Nested parts reserve room for later siblings and become opaque when their room runs out.
    #[test]
    fn entity_budget_reserves_later_siblings() {
        let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: message/rfc822\r\n\r\nContent-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\nbody\r\n--b\r\n\r\nlast\r\n--b--\r\n";
        let mut budget = 4;
        let root = parse_entity(raw, 0..raw.len(), false, 0, &mut budget);
        assert_eq!(budget, 0);
        assert_eq!(entity_count(&root), 4);
        let Children::Multipart(children) = &root.children else {
            panic!("multipart root");
        };
        let Children::Message(inner) = &children[0].children else {
            panic!("first message");
        };
        assert_eq!(inner.media_type, "application");
        assert_eq!(&raw[children[1].body.clone()], b"last");
    }

    /// Type/encoding normalization changes interpretation only, never the original section bytes.
    #[test]
    fn normalized_mime_preserves_raw_sections() {
        let raw = b"Content-Type: multipart (comment) / mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text / plain\r\nContent-Transfer-Encoding: base64 (comment)\r\n\r\naGVsbG8=\r\n--b--\r\n";
        let root = parse_message(raw);
        let part = resolve_part(&root, &[1]).unwrap();
        assert_eq!(part.media_type, "text");
        assert_eq!(part.media_subtype, "plain");
        assert_eq!(part.transfer_encoding, "base64");
        assert_eq!(
            section_bytes(
                raw,
                &root,
                &Section {
                    part: vec![1],
                    text: None
                }
            )
            .unwrap(),
            b"aGVsbG8="
        );
        assert_eq!(
            section_bytes(
                raw,
                &root,
                &Section {
                    part: vec![1],
                    text: Some(SectionText::Mime)
                }
            )
            .unwrap(),
            b"Content-Type: text / plain\r\nContent-Transfer-Encoding: base64 (comment)\r\n\r\n"
        );
    }
}
