//! Parser-only recovery for missing nested MIME multipart closing delimiters.

use std::borrow::Cow;

use mail_parser::{MessageParser, MimeHeaders};

/// Insert missing nested multipart closing delimiters before enclosing delimiters.
///
/// Correct input is returned borrowed. Recovered input is an owned parser view with inserted
/// byte ranges; callers map those offsets back to the original source for attachment reads.
pub(crate) fn recover_outer_boundaries(raw: &[u8]) -> (Cow<'_, [u8]>, Vec<(usize, usize)>) {
    let Some(root_headers_end) = header_block_end(raw, 0) else {
        return (Cow::Borrowed(raw), Vec::new());
    };
    let Some(root_boundary) = multipart_boundary(&raw[..root_headers_end]) else {
        return (Cow::Borrowed(raw), Vec::new());
    };

    let mut boundary_stack = vec![root_boundary];
    let mut insertions = Vec::<(usize, Vec<u8>)>::new();
    let mut cursor = root_headers_end;

    while let Some((line_start, line_end, next_line)) = physical_line(raw, cursor) {
        let matched_boundary = boundary_stack
            .iter()
            .rposition(|boundary| is_boundary_line(&raw[line_start..line_end], boundary));

        if let Some(matched_index) = matched_boundary {
            for nested_boundary in boundary_stack[matched_index + 1..].iter().rev() {
                insertions.push((line_start, closing_delimiter(nested_boundary)));
            }
            boundary_stack.truncate(matched_index + 1);

            let delimiter_is_closing = is_closing_boundary_line(
                &raw[line_start..line_end],
                &boundary_stack[matched_index],
            );
            if delimiter_is_closing {
                boundary_stack.pop();
            } else if let Some(part_headers_end) = header_block_end(raw, next_line) {
                if let Some(nested_boundary) =
                    embedded_multipart_boundary(raw, next_line, part_headers_end)
                {
                    boundary_stack.push(nested_boundary);
                }
                cursor = part_headers_end;
                continue;
            }
        }

        cursor = next_line;
    }

    if insertions.is_empty() {
        return (Cow::Borrowed(raw), Vec::new());
    }

    let inserted_length = insertions
        .iter()
        .map(|(_, bytes)| bytes.len())
        .sum::<usize>();
    let mut recovered = Vec::with_capacity(raw.len() + inserted_length);
    let mut inserted_ranges = Vec::with_capacity(insertions.len());
    let mut copied_until = 0;
    for (offset, delimiter) in insertions {
        recovered.extend_from_slice(&raw[copied_until..offset]);
        inserted_ranges.push((recovered.len(), delimiter.len()));
        recovered.extend_from_slice(&delimiter);
        copied_until = offset;
    }
    recovered.extend_from_slice(&raw[copied_until..]);
    (Cow::Owned(recovered), inserted_ranges)
}

/// Find the byte immediately after a MIME header block beginning at `start`.
fn header_block_end(raw: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    let mut has_header_field = false;
    while let Some((line_start, line_end, next_line)) = physical_line(raw, cursor) {
        if line_start == line_end {
            return has_header_field.then_some(next_line);
        }
        let line = &raw[line_start..line_end];
        if matches!(line.first(), Some(b' ' | b'\t')) {
            if !has_header_field {
                return None;
            }
        } else {
            let colon_position = line.iter().position(|byte| *byte == b':')?;
            let field_name = &line[..colon_position];
            if field_name.is_empty()
                || !field_name
                    .iter()
                    .all(|byte| (33..=57).contains(byte) || (59..=126).contains(byte))
            {
                return None;
            }
            has_header_field = true;
        }
        cursor = next_line;
    }
    None
}

/// Parse a header block and return its multipart boundary parameter, if present.
fn multipart_boundary(header_block: &[u8]) -> Option<Vec<u8>> {
    let parsed_headers = MessageParser::default()
        .with_mime_headers()
        .parse_headers(header_block)?;
    if parsed_headers
        .content_transfer_encoding()
        .is_some_and(|encoding| !identity_transfer_encoding(encoding))
    {
        return None;
    }
    let content_type = parsed_headers.content_type()?;
    if content_type.ctype() != "multipart" {
        return None;
    }
    content_type
        .attribute("boundary")
        .filter(|boundary| !boundary.is_empty())
        .map(|boundary| boundary.as_bytes().to_vec())
}

/// Discover multipart headers inside unencoded message entities without scanning ordinary bodies.
fn embedded_multipart_boundary(
    raw: &[u8],
    mut header_start: usize,
    mut header_end: usize,
) -> Option<Vec<u8>> {
    loop {
        let headers = &raw[header_start..header_end];
        if let Some(boundary) = multipart_boundary(headers) {
            return Some(boundary);
        }
        let parsed = MessageParser::default()
            .with_mime_headers()
            .parse_headers(headers)?;
        let content_type = parsed.content_type()?;
        if content_type.ctype() != "message"
            || !matches!(content_type.subtype(), Some("rfc822" | "global"))
        {
            return None;
        }
        if parsed
            .content_transfer_encoding()
            .is_some_and(|encoding| !identity_transfer_encoding(encoding))
        {
            return None;
        }
        header_start = header_end;
        header_end = header_block_end(raw, header_start)?;
    }
}

/// Return the next physical line's start, content end, and byte after its line ending.
fn physical_line(raw: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    let remaining = raw.get(start..)?;
    if remaining.is_empty() {
        return None;
    }
    let relative_end = remaining.iter().position(|byte| *byte == b'\n');
    let next_line = relative_end.map_or(raw.len(), |end| start + end + 1);
    let mut line_end = relative_end.map_or(raw.len(), |end| start + end);
    if line_end > start && raw[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    Some((start, line_end, next_line))
}

/// Check whether a physical line is exactly a delimiter for `boundary`.
fn is_boundary_line(line: &[u8], boundary: &[u8]) -> bool {
    boundary_line_tail(line, boundary).is_some()
}

/// Check whether a matching physical boundary line has the closing marker.
fn is_closing_boundary_line(line: &[u8], boundary: &[u8]) -> bool {
    boundary_line_tail(line, boundary).is_some_and(|tail| tail.starts_with(b"--"))
}

/// Return the legal suffix after a delimiter, allowing only trailing spaces or tabs.
fn boundary_line_tail<'a>(line: &'a [u8], boundary: &[u8]) -> Option<&'a [u8]> {
    let delimiter_length = boundary.len().checked_add(2)?;
    if !line.starts_with(b"--") || line.get(2..delimiter_length)? != boundary {
        return None;
    }
    let mut tail = &line[delimiter_length..];
    if tail.starts_with(b"--") {
        tail = &tail[2..];
    }
    tail.iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then_some(&line[delimiter_length..])
}

/// Build one missing closing delimiter with the canonical CRLF line ending.
fn closing_delimiter(boundary: &[u8]) -> Vec<u8> {
    let mut delimiter = Vec::with_capacity(boundary.len() + 6);
    delimiter.extend_from_slice(b"--");
    delimiter.extend_from_slice(boundary);
    delimiter.extend_from_slice(b"--\r\n");
    delimiter
}

/// Recognize unencoded entity bodies while ignoring MIME comments around the encoding token.
fn identity_transfer_encoding(encoding: &str) -> bool {
    crate::parsing::transfer_encoding_token(encoding).is_some_and(|token| {
        ["7bit", "8bit", "binary"]
            .iter()
            .any(|known| token.eq_ignore_ascii_case(known))
    })
}
