//! RFC 2392 Content-ID and Message-ID URL resolution within one captured message.
use crate::{
    MimeError, ParsedMessage, attachments::decoded_attachment_bytes, parsing::CapturedMessage,
};
use mail_parser::MimeHeaders;

/// Resolve a cid: URL or a mid: URL within the supplied message and return original decoded octets.
///
/// Both mid:message-id and mid:message-id/content-id are supported. Percent decoding preserves
/// identifier case and literal plus signs. Other messages are never fetched. Where an alternative
/// repeats a Content-ID, the final representation is preferred as required by MIME ordering.
/// For several reads from the same message, retain a [`ParsedMessage`] instead.
pub fn content_reference_bytes(raw_message: &[u8], reference: &str) -> Result<Vec<u8>, MimeError> {
    // Keep invalid references without a scheme separator ahead of message parsing errors.
    if !reference.contains(':') {
        return Err(MimeError::InvalidContentReference(reference.to_owned()));
    }
    ParsedMessage::parse(raw_message)?.content_reference_bytes(reference)
}

/// Resolve one local content reference without reparsing the supplied MIME view.
pub(crate) fn content_reference_bytes_from_parsed(
    parsed: &CapturedMessage<'_>,
    reference: &str,
) -> Result<Vec<u8>, MimeError> {
    let invalid = || MimeError::InvalidContentReference(reference.to_owned());
    let missing = || MimeError::ContentReferenceNotFound(reference.to_owned());
    let (scheme, value) = reference.split_once(':').ok_or_else(invalid)?;
    let content_id = if scheme.eq_ignore_ascii_case("cid") {
        percent_decoded_identifier(value).ok_or_else(invalid)?
    } else if scheme.eq_ignore_ascii_case("mid") {
        let (message_id, content_id) = value
            .split_once('/')
            .map_or((value, None), |(message, part)| (message, Some(part)));
        let message_id = percent_decoded_identifier(message_id).ok_or_else(invalid)?;
        if parsed.message_id() != Some(message_id.as_str()) {
            return Err(missing());
        }
        let Some(content_id) = content_id else {
            return Ok(parsed.original_message().to_vec());
        };
        percent_decoded_identifier(content_id).ok_or_else(invalid)?
    } else {
        return Err(invalid());
    };
    let reachable = crate::parsing::reachable_parts(parsed);
    let part = parsed
        .parts
        .iter()
        .enumerate()
        .rev()
        .find(|(index, part)| {
            reachable.contains(&(*index as u32)) && part.content_id() == Some(content_id.as_str())
        })
        .map(|(_, part)| part)
        .ok_or_else(missing)?;
    Ok(decoded_attachment_bytes(part, parsed.original_body(part)))
}

/// Decode URI escapes without form semantics and reject empty, bracketed or control-containing IDs.
fn percent_decoded_identifier(value: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let identifier = String::from_utf8(decoded).ok()?;
    if identifier.is_empty()
        || identifier.chars().any(|character| {
            character.is_ascii_whitespace()
                || character.is_ascii_control()
                || matches!(character, '<' | '>')
        })
    {
        None
    } else {
        Some(identifier)
    }
}
