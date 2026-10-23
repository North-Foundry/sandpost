//! Preparation of captured message bytes for re-submission to another SMTP server.

/**
 * Return the bytes to send for a captured message.
 *
 * The content, attachments, `Bcc` absence and every other header are preserved, with three
 * adaptations required for re-submission:
 *
 * - `Return-Path` fields are removed: capture added one as the final delivery, and RFC 5321
 *   forbids a relay from sending it.
 * - Bare line feeds become CRLF, the only line ending SMTP allows.
 * - When `override_from` is set, the `From` field is replaced by it and the original value is
 *   kept in `X-Original-From`, so the external server sees an authorized sender.
 *
 * Header fields are recognized with their continuation lines, so folded values are moved or
 * removed as a whole. The stored message itself is never changed.
 */
pub fn prepare_message(raw: &[u8], override_from: Option<&str>) -> Vec<u8> {
    let normalized = crlf_line_endings(raw);
    let header_length = header_section(&normalized);
    let (headers, body) = normalized.split_at(header_length);
    let mut prepared = Vec::with_capacity(normalized.len() + 64);
    let mut original_from: Option<&[u8]> = None;
    for field in header_fields(headers) {
        let name = field_name(field);
        if name.eq_ignore_ascii_case(b"return-path") {
            continue;
        }
        if override_from.is_some() && name.eq_ignore_ascii_case(b"from") {
            if original_from.is_none() {
                original_from = Some(field_value(field));
            }
            continue;
        }
        prepared.extend_from_slice(field);
    }
    if let Some(sender) = override_from {
        prepared.extend_from_slice(b"From: ");
        prepared.extend_from_slice(sender.as_bytes());
        prepared.extend_from_slice(b"\r\n");
        if let Some(original) = original_from {
            prepared.extend_from_slice(b"X-Original-From:");
            prepared.extend_from_slice(original);
        }
    }
    prepared.extend_from_slice(body);
    prepared
}

/**
 * Convert every bare line feed into CRLF, leaving existing CRLF pairs untouched.
 */
fn crlf_line_endings(raw: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(raw.len() + raw.len() / 64);
    let mut previous = 0_u8;
    for &byte in raw {
        if byte == b'\n' && previous != b'\r' {
            normalized.push(b'\r');
        }
        normalized.push(byte);
        previous = byte;
    }
    normalized
}

/**
 * Return the length of the header section including its final CRLF; the empty line that
 * separates it from the body, if any, starts right after it.
 */
fn header_section(message: &[u8]) -> usize {
    if message.starts_with(b"\r\n") {
        return 0;
    }
    message
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map_or(message.len(), |position| position + 2)
}

/**
 * Split a header section into complete fields, each with its continuation lines and CRLF.
 */
fn header_fields(headers: &[u8]) -> Vec<&[u8]> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < headers.len() {
        let line_end = headers[index..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map_or(headers.len(), |position| index + position + 2);
        let continues = headers
            .get(line_end)
            .is_some_and(|byte| *byte == b' ' || *byte == b'\t');
        index = line_end;
        if !continues {
            fields.push(&headers[start..line_end]);
            start = line_end;
        }
    }
    if start < headers.len() {
        fields.push(&headers[start..]);
    }
    fields
}

/**
 * Return the name of a header field, without the colon and surrounding whitespace.
 */
fn field_name(field: &[u8]) -> &[u8] {
    let end = field
        .iter()
        .position(|byte| *byte == b':')
        .unwrap_or(field.len());
    field[..end].trim_ascii()
}

/**
 * Return a header field's raw value after its colon, including folding and the final CRLF.
 */
fn field_value(field: &[u8]) -> &[u8] {
    match field.iter().position(|byte| *byte == b':') {
        Some(colon) => &field[colon + 1..],
        None => b"\r\n",
    }
}

#[cfg(test)]
mod tests {
    use super::prepare_message;

    /**
     * Removing the capture's Return-Path keeps every other header, Bcc absence and the body.
     */
    #[test]
    fn return_path_is_removed_and_the_rest_is_preserved() {
        let raw = b"Return-Path: <sender@example.test>\r\nReceived: from client by sandpost\r\nFrom: Sender <sender@example.test>\r\nTo: a@example.test\r\nSubject: Hello\r\n\r\nBody line\r\n.leading dot\r\n";
        let prepared = prepare_message(raw, None);
        assert_eq!(
            prepared,
            b"Received: from client by sandpost\r\nFrom: Sender <sender@example.test>\r\nTo: a@example.test\r\nSubject: Hello\r\n\r\nBody line\r\n.leading dot\r\n"
        );
    }

    /**
     * A folded From field is replaced as a whole and its original value is retained.
     */
    #[test]
    fn from_override_replaces_folded_fields() {
        let raw = b"From: Very Long Name\r\n <sender@example.test>\r\nSubject: Hi\r\n\r\nBody\r\n";
        let prepared = prepare_message(raw, Some("relay@example.test"));
        assert_eq!(
            String::from_utf8(prepared).unwrap(),
            "Subject: Hi\r\nFrom: relay@example.test\r\nX-Original-From: Very Long Name\r\n <sender@example.test>\r\n\r\nBody\r\n"
        );
    }

    /**
     * Bare line feeds become CRLF while binary-safe bytes and existing CRLF are unchanged.
     */
    #[test]
    fn bare_line_feeds_become_crlf() {
        let raw = b"Subject: x\n\nline one\nline two\r\n\xc3\xa9\n";
        let prepared = prepare_message(raw, None);
        assert_eq!(
            prepared,
            b"Subject: x\r\n\r\nline one\r\nline two\r\n\xc3\xa9\r\n"
        );
    }

    /**
     * A message without a body separator keeps its headers and gains no body.
     */
    #[test]
    fn header_only_messages_are_preserved() {
        let raw = b"Return-Path: <>\r\nSubject: only headers\r\n";
        assert_eq!(prepare_message(raw, None), b"Subject: only headers\r\n");
    }
}
