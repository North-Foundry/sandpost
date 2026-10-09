//! Shared MIME interpretation for message facts, attachment metadata and downloads.
use crate::{MimeError, attachments::decoded_attachment_bytes};
use mail_parser::{
    ContentType, HeaderName, HeaderValue, Message, MessageParser, MimeHeaders, PartType,
    decoders::charsets::map::charset_decoder,
};
use std::borrow::Cow;

/// Parsed MIME view together with the unchanged source and synthetic delimiter offset mapping.
pub(crate) struct CapturedMessage<'a> {
    message: Message<'a>,
    original: &'a [u8],
    insertions: Vec<(usize, usize)>,
}

impl<'a> std::ops::Deref for CapturedMessage<'a> {
    type Target = Message<'a>;
    /// Expose the parser view without copying its owned header and body values.
    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl CapturedMessage<'_> {
    /// Return the unchanged bytes captured before MIME boundary recovery.
    pub(crate) fn original_message(&self) -> &[u8] {
        self.original
    }

    /// Return the source body without synthetic recovery delimiters, before transfer decoding.
    pub(crate) fn original_body(&self, part: &mail_parser::MessagePart<'_>) -> &[u8] {
        original_body(self.original, &self.insertions, part)
    }
}

/// Translate a parser-view position back to the unchanged captured byte position.
fn original_offset(position: usize, insertions: &[(usize, usize)]) -> usize {
    let mut removed = 0;
    for &(start, length) in insertions {
        if position <= start {
            break;
        }
        removed += (position - start).min(length);
    }
    position - removed
}

/// Map a part's parser-view body range to the original captured message.
fn original_body<'a>(
    original: &'a [u8],
    insertions: &[(usize, usize)],
    part: &mail_parser::MessagePart<'_>,
) -> &'a [u8] {
    let start = original_offset(part.raw_body_offset() as usize, insertions);
    let view_end = part.raw_end_offset() as usize;
    let mut end = original_offset(view_end, insertions);
    if insertions
        .iter()
        .any(|&(position, length)| view_end >= position && view_end <= position + length)
    {
        if original.get(end.saturating_sub(2)..end) == Some(b"\r\n") {
            end -= 2;
        } else if original.get(end.saturating_sub(1)..end) == Some(b"\n") {
            end -= 1;
        }
    }
    original.get(start..end).unwrap_or_default()
}

/// Parse a captured message and apply receiver rules not handled by the dependency.
pub(crate) fn parse_message(raw_message: &[u8]) -> Result<CapturedMessage<'_>, MimeError> {
    let parser = MessageParser::new()
        .with_mime_headers()
        .with_address_headers()
        .with_message_ids()
        .default_header_text();
    let mut captured = parse_mime_view(raw_message, &parser)?;
    preserve_opaque_parts(&mut captured.message, raw_message, &captured.insertions);
    select_body_representations(&mut captured.message);
    classify_related_resources_and_finalize_attachments(&mut captured.message);
    Ok(captured)
}

/// Recover enclosing MIME boundaries, parse the view, and normalize transfer-encoding comments.
fn parse_mime_view<'a>(
    raw_message: &'a [u8],
    parser: &MessageParser,
) -> Result<CapturedMessage<'a>, MimeError> {
    let (recovered, insertions) = crate::boundaries::recover_outer_boundaries(raw_message);
    let mut parsed = match recovered {
        Cow::Borrowed(original) => parser.parse(original),
        Cow::Owned(recovered) => parser.parse(&recovered).map(Message::into_owned),
    }
    .ok_or(MimeError::MissingHeaders)?;
    if let Some(normalized) = normalize_transfer_encoding_comments(&parsed) {
        parsed = parser
            .parse(&normalized)
            .map(Message::into_owned)
            .ok_or(MimeError::MissingHeaders)?;
    }
    Ok(CapturedMessage {
        message: parsed,
        original: raw_message,
        insertions,
    })
}

/// Replace parts with unsupported encodings or charsets by opaque decoded binary parts.
fn preserve_opaque_parts(
    parsed: &mut Message<'_>,
    raw_message: &[u8],
    insertions: &[(usize, usize)],
) {
    for (index, part) in parsed.parts.iter_mut().enumerate() {
        let unknown_encoding = part.content_transfer_encoding().is_some_and(|encoding| {
            !["7bit", "8bit", "binary", "base64", "quoted-printable"]
                .iter()
                .any(|known| encoding.eq_ignore_ascii_case(known))
        });
        let unknown_charset = part
            .content_type()
            .filter(|content_type| content_type.ctype() == "text")
            .and_then(|content_type| content_type.attribute("charset"))
            .is_some_and(|charset| {
                !charset.eq_ignore_ascii_case("utf-8")
                    && !charset.eq_ignore_ascii_case("us-ascii")
                    && charset_decoder(charset.as_bytes()).is_none()
            });
        if unknown_encoding || unknown_charset {
            part.body = PartType::Binary(Cow::Owned(decoded_attachment_bytes(
                part,
                original_body(raw_message, insertions, part),
            )));
            let content_type = HeaderValue::ContentType(ContentType {
                c_type: "application".into(),
                c_subtype: Some("octet-stream".into()),
                attributes: None,
            });
            if let Some(header) = part
                .headers
                .iter_mut()
                .find(|header| header.name == HeaderName::ContentType)
            {
                header.value = content_type;
            } else {
                part.headers.push(mail_parser::Header {
                    name: HeaderName::ContentType,
                    value: content_type,
                    offset_field: 0,
                    offset_start: 0,
                    offset_end: 0,
                });
            }
            if !parsed.attachments.contains(&(index as u32)) {
                parsed.attachments.push(index as u32);
            }
        }
    }
}

/// Select supported text and HTML body parts, using the available form for both representations.
fn select_body_representations(parsed: &mut Message<'_>) {
    let mut text = body_parts(parsed, false);
    let mut html = body_parts(parsed, true);
    // Keep the dependency's useful text/HTML conversion when only one representation exists.
    if text.is_empty() {
        text.clone_from(&html);
    }
    if html.is_empty() {
        html.clone_from(&text);
    }
    parsed.text_body = text;
    parsed.html_body = html;
}

/// Add non-root related resources, then retain only reachable non-body attachments in order.
fn classify_related_resources_and_finalize_attachments(parsed: &mut Message<'_>) {
    // Related resources are attachments; the selected body root ignores its disposition.
    for index in 0..parsed.parts.len() {
        if parsed.parts[index]
            .content_type()
            .and_then(|content_type| content_type.subtype())
            == Some("related")
        {
            let root = related_root_part(parsed, index as u32);
            if let PartType::Multipart(children) = &parsed.parts[index].body {
                for child in children {
                    if Some(*child) != root
                        && !matches!(parsed.parts[*child as usize].body, PartType::Multipart(_))
                    {
                        parsed.attachments.push(*child);
                    }
                }
            }
        }
    }
    let reachable = reachable_parts(parsed);
    parsed.attachments.retain(|index| {
        reachable.contains(index)
            && !parsed.text_body.contains(index)
            && !parsed.html_body.contains(index)
    });
    parsed.attachments.sort_unstable();
    parsed.attachments.dedup();
}

/// List structurally reachable parts, excluding children of opaque recovered multipart entities.
pub(crate) fn reachable_parts(parsed: &Message<'_>) -> std::collections::BTreeSet<u32> {
    let mut pending = vec![0];
    let mut reachable = std::collections::BTreeSet::new();
    while let Some(index) = pending.pop() {
        if reachable.insert(index)
            && let Some(mail_parser::MessagePart {
                body: PartType::Multipart(children),
                ..
            }) = parsed.parts.get(index as usize)
        {
            pending.extend(children);
        }
    }
    reachable
}

/// Remove MIME comments from actual transfer-encoding fields in a view with unchanged offsets.
fn normalize_transfer_encoding_comments(parsed: &Message<'_>) -> Option<Vec<u8>> {
    let mut normalized = None;
    for part in &parsed.parts {
        for header in &part.headers {
            if header.name != HeaderName::ContentTransferEncoding {
                continue;
            }
            let Some(value) = header.value.as_text() else {
                continue;
            };
            if !value.contains('(') {
                continue;
            }
            let Some(token) = transfer_encoding_token(value) else {
                continue;
            };
            if !["7bit", "8bit", "binary", "base64", "quoted-printable"]
                .iter()
                .any(|known| token.eq_ignore_ascii_case(known))
            {
                continue;
            }
            let bytes = normalized.get_or_insert_with(|| parsed.raw_message.to_vec());
            let Some(range) =
                bytes.get_mut(header.offset_start as usize..header.offset_end as usize)
            else {
                continue;
            };
            for byte in range.iter_mut() {
                if !matches!(byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            // A folded first line can be empty; place the token in the first contiguous space run.
            if let Some(start) = range
                .windows(token.len())
                .enumerate()
                .find(|(start, window)| {
                    (*start == 0 || range[*start - 1] != b'\n')
                        && window.iter().all(|byte| *byte == b' ')
                })
                .map(|(start, _)| start)
            {
                range[start..start + token.len()].copy_from_slice(token.as_bytes());
            }
        }
    }
    normalized
}

/// Strip nested MIME comments and surrounding whitespace from a transfer encoding token.
pub(crate) fn transfer_encoding_token(value: &str) -> Option<String> {
    let mut token = String::new();
    let mut depth = 0usize;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if depth > 0 => escaped = true,
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => token.push(character),
            _ => {}
        }
    }
    (depth == 0 && !escaped).then(|| token.trim().to_owned())
}

/// Select actual body representations, choosing the last supported alternative and the related root.
fn body_parts(parsed: &Message<'_>, html: bool) -> Vec<u32> {
    let mut related_roots = vec![false; parsed.parts.len()];
    for (index, part) in parsed.parts.iter().enumerate() {
        if let PartType::Multipart(children) = &part.body {
            if part
                .content_type()
                .and_then(|content_type| content_type.subtype())
                == Some("related")
            {
                if let Some(root) = related_root_part(parsed, index as u32) {
                    related_roots[root as usize] = true;
                }
            } else if related_roots[index] {
                for child in children {
                    related_roots[*child as usize] = true;
                }
            }
        }
    }
    let native_bodies = if html {
        &parsed.html_body
    } else {
        &parsed.text_body
    };
    let mut selected = vec![Vec::new(); parsed.parts.len()];
    // The parser allocates parents before their children. Consume child selections in reverse
    // order, avoiding recursion and repeated copies for deeply nested untrusted messages.
    for (index, part) in parsed.parts.iter().enumerate().rev() {
        selected[index] = match &part.body {
            PartType::Text(_)
                if !html && (related_roots[index] || native_bodies.contains(&(index as u32))) =>
            {
                vec![index as u32]
            }
            PartType::Html(_)
                if html && (related_roots[index] || native_bodies.contains(&(index as u32))) =>
            {
                vec![index as u32]
            }
            PartType::Multipart(children) => match part
                .content_type()
                .and_then(|content_type| content_type.subtype())
            {
                Some("alternative") => children
                    .iter()
                    .rev()
                    .find(|child| !selected[**child as usize].is_empty())
                    .map(|child| std::mem::take(&mut selected[*child as usize]))
                    .unwrap_or_default(),
                Some("related") => related_root_part(parsed, index as u32)
                    .map(|child| std::mem::take(&mut selected[child as usize]))
                    .unwrap_or_default(),
                _ => children
                    .iter()
                    .flat_map(|child| std::mem::take(&mut selected[*child as usize]))
                    .collect(),
            },
            _ => Vec::new(),
        };
    }
    selected.into_iter().next().unwrap_or_default()
}

/// Resolve a multipart/related start parameter, defaulting to its first child.
pub(crate) fn related_root_part(parsed: &Message<'_>, index: u32) -> Option<u32> {
    let part = parsed.parts.get(index as usize)?;
    let PartType::Multipart(children) = &part.body else {
        return None;
    };
    if let Some(start) = part
        .content_type()
        .and_then(|content_type| content_type.attribute("start"))
    {
        let start = start.trim().trim_start_matches('<').trim_end_matches('>');
        children
            .iter()
            .copied()
            .find(|child| parsed.parts[*child as usize].content_id() == Some(start))
    } else {
        children.first().copied()
    }
}
