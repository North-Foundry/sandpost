//! Mailbox name encoding (RFC 3501 section 5.1.3 modified UTF-7) and LIST pattern matching.
//!
//! The protocol layer exchanges mailbox names with the backend as Unicode strings. Names sent to
//! clients are encoded in modified UTF-7; names received from clients are decoded from it. Raw
//! UTF-8 from clients that ignore the encoding is accepted as well, because it cannot be confused
//! with valid modified UTF-7, which is pure ASCII.

/// The hierarchy delimiter announced in LIST responses.
pub const HIERARCHY_DELIMITER: char = '/';

/// The modified base64 alphabet: standard base64 with `,` instead of `/`.
const MODIFIED_BASE64: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

/// Encode a Unicode mailbox name in modified UTF-7.
pub fn encode(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    for character in name.chars() {
        if (' '..='~').contains(&character) {
            flush_shifted(&mut encoded, &mut pending);
            if character == '&' {
                encoded.push_str("&-");
            } else {
                encoded.push(character);
            }
        } else {
            let mut units = [0u16; 2];
            pending.extend_from_slice(character.encode_utf16(&mut units));
        }
    }
    flush_shifted(&mut encoded, &mut pending);
    encoded
}

/// Write pending UTF-16 code units as one `&...-` shifted run.
fn flush_shifted(encoded: &mut String, pending: &mut Vec<u16>) {
    if pending.is_empty() {
        return;
    }
    let bytes: Vec<u8> = pending.iter().flat_map(|unit| unit.to_be_bytes()).collect();
    encoded.push('&');
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        let symbols = chunk.len() + 1;
        for index in 0..symbols {
            encoded.push(char::from(
                MODIFIED_BASE64[((value >> (18 - 6 * index)) & 63) as usize],
            ));
        }
    }
    encoded.push('-');
    pending.clear();
}

/// Decode a client-supplied mailbox name.
///
/// Pure ASCII input must be valid modified UTF-7; other input must be valid UTF-8 and is taken
/// literally. Returns `None` for malformed names.
pub fn decode(raw: &[u8]) -> Option<String> {
    if !raw.is_ascii() {
        return std::str::from_utf8(raw).ok().map(str::to_owned);
    }
    let mut decoded = String::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        let byte = raw[index];
        if byte != b'&' {
            if !(0x20..=0x7e).contains(&byte) {
                return None;
            }
            decoded.push(char::from(byte));
            index += 1;
            continue;
        }
        let end = index + 1 + raw[index + 1..].iter().position(|byte| *byte == b'-')?;
        let shifted = &raw[index + 1..end];
        if shifted.is_empty() {
            decoded.push('&');
        } else {
            decoded.push_str(&decode_shifted(shifted)?);
        }
        index = end + 1;
    }
    (encode(&decoded).as_bytes() == raw).then_some(decoded)
}

/// Decode the modified base64 body of one shifted run into Unicode text.
fn decode_shifted(shifted: &[u8]) -> Option<String> {
    let mut bits: u32 = 0;
    let mut bit_count = 0;
    let mut bytes = Vec::new();
    for symbol in shifted {
        let value = MODIFIED_BASE64
            .iter()
            .position(|candidate| candidate == symbol)? as u32;
        bits = (bits << 6) | value;
        bit_count += 6;
        if bit_count >= 8 {
            bit_count -= 8;
            bytes.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    // Leftover bits must be zero padding shorter than one base64 symbol.
    if bit_count >= 6 || bits != 0 || bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16(&units).ok()?;
    // Printable ASCII must be represented directly, never inside a shifted run.
    if text
        .chars()
        .any(|character| (' '..='~').contains(&character))
    {
        return None;
    }
    Some(text)
}

/// Report whether a name refers to the INBOX, which is case-insensitive (RFC 3501 section 5.1).
pub fn is_inbox(name: &str) -> bool {
    name.eq_ignore_ascii_case("INBOX")
}

/// Match a mailbox name against a LIST pattern.
///
/// `*` matches any characters including the hierarchy delimiter; `%` matches any characters
/// except the delimiter (RFC 3501 section 6.3.8). Matching is case-sensitive except that a
/// pattern spelling INBOX in any case matches the INBOX.
pub fn matches_pattern(name: &str, pattern: &str) -> bool {
    if is_inbox(name) && is_inbox(pattern) {
        return true;
    }
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    wildcard_match(&name, &pattern)
}

/// Match LIST wildcards in O(name length × pattern length) time and O(name length) memory.
fn wildcard_match(name: &[char], pattern: &[char]) -> bool {
    let mut matches = vec![false; name.len() + 1];
    matches[0] = true;
    for symbol in pattern {
        if matches!(symbol, '*' | '%') {
            for position in 1..=name.len() {
                matches[position] = matches[position]
                    || (matches[position - 1]
                        && (*symbol == '*' || name[position - 1] != HIERARCHY_DELIMITER));
            }
        } else {
            for position in (1..=name.len()).rev() {
                matches[position] = matches[position - 1] && name[position - 1] == *symbol;
            }
            matches[0] = false;
        }
    }
    matches[name.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 3501 section 5.1.3 example and ampersands round-trip.
    #[test]
    fn modified_utf7_round_trips_rfc_example() {
        let name = "~peter/mail/\u{53f0}\u{5317}/\u{65e5}\u{672c}\u{8a9e}";
        let encoded = encode(name);
        assert_eq!(encoded, "~peter/mail/&U,BTFw-/&ZeVnLIqe-");
        assert_eq!(decode(encoded.as_bytes()).unwrap(), name);
        assert_eq!(encode("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(decode(b"Tom &- Jerry").unwrap(), "Tom & Jerry");
        assert_eq!(
            decode(encode("Caf\u{e9} \u{1f600}").as_bytes()).unwrap(),
            "Caf\u{e9} \u{1f600}"
        );
    }

    /// Malformed shifted runs and encoded printable ASCII are rejected; raw UTF-8 is accepted.
    #[test]
    fn malformed_modified_utf7_is_rejected() {
        assert!(decode(b"&Jjo").is_none(), "unterminated");
        assert!(decode(b"&AGE-").is_none(), "encodes printable ASCII 'a'");
        assert!(decode(b"&U,B-").is_none(), "odd length");
        assert!(decode(b"tab\there").is_none());
        assert!(
            decode(b"&U,BTFw-&ZeVnLIqe-").is_none(),
            "superfluous shifts"
        );
        assert_eq!(decode("Caf\u{e9}".as_bytes()).unwrap(), "Caf\u{e9}");
    }

    /// Repeated wildcard alternatives remain tractable instead of exploring exponentially many paths.
    #[test]
    fn adversarial_list_patterns_are_bounded() {
        let name = "a".repeat(256);
        assert!(!matches_pattern(&name, &("*a".repeat(256) + "b")));
        assert!(matches_pattern(&name, &"%a".repeat(256)));
        assert!(matches_pattern("a/b", "*%b"));
        assert!(!matches_pattern("a/b", "%a%b"));
    }

    /// `*` crosses the delimiter, `%` does not, and INBOX matches case-insensitively.
    #[test]
    fn list_patterns_follow_rfc_wildcards() {
        assert!(matches_pattern("Projects/Laravel", "*"));
        assert!(!matches_pattern("Projects/Laravel", "%"));
        assert!(matches_pattern("Projects/Laravel", "Projects/%"));
        assert!(matches_pattern("Laravel", "La*l"));
        assert!(matches_pattern("Laravel", "%ravel"));
        assert!(!matches_pattern("Laravel", "laravel"));
        assert!(matches_pattern("INBOX", "inbox"));
        assert!(matches_pattern("", ""));
        assert!(!matches_pattern("Trash", ""));
        assert!(matches_pattern("a/b/c", "a*c"));
        assert!(!matches_pattern("a/b/c", "a%c"));
    }
}
