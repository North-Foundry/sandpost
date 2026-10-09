//! Header extraction: every header line, decoded where possible and unfolded.
use crate::{MimeError, normalize_mailbox};
use mail_parser::{HeaderName, Message as ParsedMessage, parsers::MessageStream};
use sandpost_core::Mailbox;
use std::{borrow::Cow, collections::BTreeMap};

/// Collect the root part's headers by lowercase name, keeping repeated values in order.
///
/// The parsed header list is walked directly: `headers_raw()` skips headers whose raw bytes are
/// not UTF-8, so pairing it with `headers()` by index would shift later values onto the wrong
/// names. The decoded text is preferred; otherwise the raw value is kept with invalid UTF-8
/// replaced, so no header line is dropped.
pub(crate) fn collect_headers(
    parsed: &ParsedMessage<'_>,
    raw_message: &[u8],
) -> BTreeMap<String, Vec<String>> {
    let mut headers = BTreeMap::<String, Vec<String>>::new();
    for header in parsed.headers() {
        let value = match header.value().as_text() {
            Some(text) => Cow::Borrowed(text),
            None => String::from_utf8_lossy(
                raw_message
                    .get(header.offset_start as usize..header.offset_end as usize)
                    .unwrap_or_default(),
            ),
        };
        headers
            .entry(header.name.as_str().to_ascii_lowercase())
            .or_default()
            .push(unfold_header(&value));
    }
    headers
}

/// Replace folded header line breaks with spaces and trim surrounding whitespace.
fn unfold_header(header_value: &str) -> String {
    let mut unfolded_value = String::with_capacity(header_value.len());
    let mut characters = header_value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' && characters.peek() == Some(&'\n') {
            characters.next();
            while matches!(characters.peek(), Some(' ' | '\t')) {
                characters.next();
            }
            unfolded_value.push(' ');
        } else {
            unfolded_value.push(character);
        }
    }
    unfolded_value.trim().to_owned()
}

/// Extract all instances of an address header, retaining quoted local parts and flattening groups.
pub(crate) fn header_mailboxes(
    parsed: &ParsedMessage<'_>,
    header_name: HeaderName<'_>,
) -> Result<Vec<Mailbox>, MimeError> {
    let mut mailboxes = Vec::new();
    for header in parsed
        .headers()
        .iter()
        .filter(|header| header.name == header_name)
    {
        let value = String::from_utf8_lossy(
            parsed
                .raw_message
                .get(header.offset_start as usize..header.offset_end as usize)
                .unwrap_or_default(),
        );
        // The dependency treats an unbracketed quoted local part as a display name.
        // Supplying equivalent angle-address syntax fixes extraction without changing stored bytes.
        let value = format!("{}\r\n", bracket_quoted_addresses(&unfold_header(&value)));
        let addresses = MessageStream::new(value.as_bytes()).parse_address();
        if let Some(addresses) = addresses.as_address() {
            for address in addresses.iter().filter_map(|address| address.address()) {
                mailboxes.push(normalize_mailbox(address)?);
            }
        }
    }
    Ok(mailboxes)
}

/// Wrap bare quoted addr-specs within lists and groups while leaving display-name syntax intact.
fn bracket_quoted_addresses(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut comment_depth = 0usize;
    let mut brackets = 0usize;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted || comment_depth > 0 => escaped = true,
            '"' if comment_depth == 0 => quoted = !quoted,
            '(' if !quoted => comment_depth += 1,
            ')' if !quoted && comment_depth > 0 => comment_depth -= 1,
            '<' | '[' if !quoted && comment_depth == 0 => brackets += 1,
            '>' | ']' if !quoted && comment_depth == 0 => brackets = brackets.saturating_sub(1),
            ',' | ':' | ';' if !quoted && comment_depth == 0 && brackets == 0 => {
                append_address_segment(&mut result, &value[start..index]);
                result.push(character);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    append_address_segment(&mut result, &value[start..]);
    result
}

/// Use the normalizer to distinguish a quoted mailbox from a quoted display name.
fn append_address_segment(result: &mut String, segment: &str) {
    let candidate = without_address_comments(segment);
    let candidate = candidate.trim();
    if candidate.starts_with('"') && normalize_mailbox(candidate).is_ok() {
        result.push('<');
        result.push_str(candidate);
        result.push('>');
    } else {
        result.push_str(segment);
    }
}

/// Remove nested CFWS comments outside quoted strings for addr-spec recognition.
fn without_address_comments(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0usize;
    for character in value.chars() {
        if escaped {
            if depth == 0 {
                result.push(character);
            }
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted || depth > 0 => {
                escaped = true;
                if depth == 0 {
                    result.push(character);
                }
            }
            '"' if depth == 0 => {
                quoted = !quoted;
                result.push(character);
            }
            '(' if !quoted => depth += 1,
            ')' if !quoted && depth > 0 => depth -= 1,
            _ if depth == 0 => result.push(character),
            _ => {}
        }
    }
    result
}
