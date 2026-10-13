//! Raw header fields: locating fields in a header block, unfolding, and structured values.
//!
//! IMAP returns header data byte-for-byte (RFC 3501 section 6.4.5), so fields keep references to
//! their original octets; unfolded values are produced only where the protocol interprets them.
use std::ops::Range;

/// One header field: its name and the byte range of the complete field including line breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderField {
    /// Field name exactly as written, without the colon.
    pub name: String,
    /// The whole field, from the name through its final line break.
    pub range: Range<usize>,
    /// The value octets after the colon, still folded, without the final line break.
    pub value: Range<usize>,
}

/// Split a header block into fields. Lines that are neither fields nor continuations are
/// ignored, as are leading continuation lines that have no field to continue.
pub fn parse_fields(message: &[u8], header: Range<usize>) -> Vec<HeaderField> {
    let mut fields: Vec<HeaderField> = Vec::new();
    let mut position = header.start;
    while position < header.end {
        let line_end = message[position..header.end]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(header.end, |offset| position + offset + 1);
        let line = &message[position..line_end];
        let content_end = line_end - trailing_line_break(line);
        if line
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            if let Some(field) = fields.last_mut() {
                field.range.end = line_end;
                field.value.end = content_end;
            }
        } else if let Some(colon) = line.iter().position(|byte| *byte == b':') {
            let name = &line[..colon];
            if !name.is_empty() && name.iter().all(|byte| (0x21..=0x7e).contains(byte)) {
                fields.push(HeaderField {
                    name: String::from_utf8_lossy(name).into_owned(),
                    range: position..line_end,
                    value: position + colon + 1..content_end,
                });
            }
        }
        position = line_end;
    }
    fields
}

/// Number of line-break octets (CRLF, LF, or none) ending a line.
fn trailing_line_break(line: &[u8]) -> usize {
    if line.ends_with(b"\r\n") {
        2
    } else if line.ends_with(b"\n") {
        1
    } else {
        0
    }
}

/// Unfold a field value: remove line breaks before whitespace and trim surrounding whitespace.
pub fn unfold(value: &[u8]) -> Vec<u8> {
    let mut unfolded = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        match value[index] {
            b'\r' if value.get(index + 1) == Some(&b'\n') => index += 2,
            b'\n' => index += 1,
            byte => {
                unfolded.push(byte);
                index += 1;
            }
        }
    }
    let start = unfolded
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(unfolded.len());
    let end = unfolded
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |position| position + 1);
    unfolded[start..end].to_vec()
}

/// The unfolded value of the first field with this name (ASCII-case-insensitive).
pub fn first_value(message: &[u8], fields: &[HeaderField], name: &str) -> Option<Vec<u8>> {
    fields
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case(name))
        .map(|field| unfold(&message[field.value.clone()]))
}

/// Read an interpreted metadata value; NUL-bearing values are unavailable rather than rewritten.
/// This does not alter raw header sections or decode RFC 2047 words for FETCH metadata.
pub fn metadata_value(message: &[u8], fields: &[HeaderField], name: &str) -> Option<Vec<u8>> {
    first_value(message, fields, name).filter(|value| !value.contains(&0))
}

/// A parsed MIME structured value such as `Content-Type` or `Content-Disposition`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StructuredValue {
    /// The leading token (for example `text/plain` or `attachment`), lowercase.
    pub value: String,
    /// Parameters in order, with lowercase names and unquoted raw values.
    pub parameters: Vec<(String, Vec<u8>)>,
}

impl StructuredValue {
    /// Parse `token *(";" attribute "=" value)`, skipping comments and malformed parameters.
    pub fn parse(raw: &[u8]) -> Self {
        let raw = strip_comments(raw);
        let mut parts = split_unquoted(&raw, b';').into_iter();
        let value = parts
            .next()
            .and_then(
                |leading| match leading.iter().position(|byte| *byte == b'/') {
                    Some(slash) => Some(format!(
                        "{}/{}",
                        mime_token(&leading[..slash])?,
                        mime_token(&leading[slash + 1..])?
                    )),
                    None => mime_token(leading),
                },
            )
            .unwrap_or_default();
        let parameters = parts
            .filter_map(|parameter| {
                let parameter = parameter.trim_ascii();
                let equals = parameter.iter().position(|byte| *byte == b'=')?;
                let name = mime_token(&parameter[..equals])?;
                let value = parameter[equals + 1..].trim_ascii();
                if value.contains(&0) {
                    return None;
                }
                Some((name, unquote(value)))
            })
            .collect();
        Self { value, parameters }
    }

    /// The value of a parameter by lowercase name.
    #[cfg(test)]
    pub fn parameter(&self, name: &str) -> Option<&[u8]> {
        self.parameters
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_slice())
    }
}

/// Parse one MIME token, ignoring surrounding whitespace but rejecting embedded separators.
fn mime_token(raw: &[u8]) -> Option<String> {
    let token = raw.trim_ascii();
    (!token.is_empty()
        && token
            .iter()
            .all(|byte| (0x21..=0x7e).contains(byte) && !b"()<>@,;:\\\"/[]?=".contains(byte)))
    .then(|| String::from_utf8_lossy(token).to_ascii_lowercase())
}

/// Interpret Content-Transfer-Encoding as a lowercase MIME token, ignoring comments.
/// Missing, empty, or malformed values use the MIME default of 7bit; raw fields stay unchanged.
pub fn transfer_encoding(message: &[u8], fields: &[HeaderField]) -> String {
    first_value(message, fields, "content-transfer-encoding")
        .and_then(|raw| mime_token(&strip_comments(&raw)))
        .unwrap_or_else(|| "7bit".to_owned())
}

/// Remove RFC 5322 comments outside quoted strings, honoring nesting and escapes.
pub fn strip_comments(raw: &[u8]) -> Vec<u8> {
    let mut stripped = Vec::with_capacity(raw.len());
    let mut depth = 0usize;
    let mut quoted = false;
    let mut index = 0;
    while index < raw.len() {
        let byte = raw[index];
        if byte == b'\\' && (quoted || depth > 0) {
            if depth == 0 {
                stripped.push(byte);
                if let Some(next) = raw.get(index + 1) {
                    stripped.push(*next);
                }
            }
            index += 2;
            continue;
        }
        match byte {
            b'"' if depth == 0 => {
                quoted = !quoted;
                stripped.push(byte);
            }
            b'(' if !quoted => depth += 1,
            b')' if !quoted && depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    stripped.push(b' ');
                }
            }
            _ if depth == 0 => stripped.push(byte),
            _ => {}
        }
        index += 1;
    }
    stripped
}

/// Split on a separator byte that is not inside a quoted string.
pub fn split_unquoted(raw: &[u8], separator: u8) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'\\' if quoted => index += 1,
            b'"' => quoted = !quoted,
            byte if byte == separator && !quoted => {
                parts.push(&raw[start..index]);
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    parts.push(&raw[start.min(raw.len())..]);
    parts
}

/// Remove surrounding double quotes and resolve backslash escapes.
pub fn unquote(value: &[u8]) -> Vec<u8> {
    if value.len() >= 2 && value.starts_with(b"\"") && value.ends_with(b"\"") {
        let mut unquoted = Vec::with_capacity(value.len() - 2);
        let mut index = 1;
        while index < value.len() - 1 {
            if value[index] == b'\\' && index + 1 < value.len() - 1 {
                index += 1;
            }
            unquoted.push(value[index]);
            index += 1;
        }
        unquoted
    } else {
        value.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fields keep their exact byte ranges, including continuation lines.
    #[test]
    fn fields_span_continuation_lines() {
        let header = b"Subject: Hello\r\n  world\r\nX-Empty:\r\nbroken line\r\nTo: a@b\r\n\r\n";
        let fields = parse_fields(header, 0..header.len() - 2);
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "Subject");
        assert_eq!(
            &header[fields[0].range.clone()],
            b"Subject: Hello\r\n  world\r\n"
        );
        assert_eq!(unfold(&header[fields[0].value.clone()]), b"Hello  world");
        assert_eq!(unfold(&header[fields[1].value.clone()]), b"");
        assert_eq!(first_value(header, &fields, "to").unwrap(), b"a@b");
    }

    /// Structured values lowercase tokens and names and unquote values.
    #[test]
    fn structured_values_parse_parameters() {
        let value = StructuredValue::parse(
            b"Multipart/Mixed; boundary=\"a;b\" (comment); Charset=UTF-8; broken",
        );
        assert_eq!(value.value, "multipart/mixed");
        assert_eq!(value.parameter("boundary"), Some(&b"a;b"[..]));
        assert_eq!(value.parameter("charset"), Some(&b"UTF-8"[..]));
        assert_eq!(value.parameters.len(), 2);
    }

    /// Comments are removed outside quotes and nested comments are honored.
    #[test]
    fn comments_are_stripped_outside_quotes() {
        assert_eq!(
            strip_comments(b"a (x (y) z) \"(kept)\" b"),
            b"a   \"(kept)\" b".to_vec()
        );
        assert_eq!(unquote(b"\"a\\\"b\""), b"a\"b");
    }

    /// MIME tokens ignore comments and surrounding whitespace without joining invalid atoms.
    #[test]
    fn mime_tokens_normalize_comments_and_whitespace() {
        let value =
            StructuredValue::parse(b"Multipart (nested (comment)) / Mixed; boundary=\"a;b\"");
        assert_eq!(value.value, "multipart/mixed");
        assert_eq!(value.parameter("boundary"), Some(&b"a;b"[..]));
        assert_eq!(StructuredValue::parse(b"text / plain").value, "text/plain");
        for invalid in [
            b"mul tipart/mixed".as_slice(),
            b"text/plain/extra",
            b"text/\0plain",
        ] {
            assert!(StructuredValue::parse(invalid).value.is_empty());
        }
        let raw = b"Content-Transfer-Encoding: BaSe64 (comment)\r\n\r\n";
        assert_eq!(
            transfer_encoding(raw, &parse_fields(raw, 0..raw.len())),
            "base64"
        );
    }

    /// Invalid NUL metadata is unavailable, while original values and encoded ASCII stay intact.
    #[test]
    fn metadata_rejects_nul_without_rewriting_source() {
        let raw = b"Subject: a\0b\r\nX-Encoded: =?utf-8?B?AA==?=\r\n\r\n";
        let fields = parse_fields(raw, 0..raw.len());
        assert_eq!(first_value(raw, &fields, "subject").unwrap(), b"a\0b");
        assert!(metadata_value(raw, &fields, "subject").is_none());
        assert_eq!(
            metadata_value(raw, &fields, "x-encoded").unwrap(),
            b"=?utf-8?B?AA==?="
        );
        let parameters = StructuredValue::parse(
            b"application/octet-stream; name=\"a\0b\"; filename*=utf-8''%00",
        );
        assert!(parameters.parameter("name").is_none());
        assert_eq!(parameters.parameter("filename*"), Some(&b"utf-8''%00"[..]));
    }
}
