//! Character classes of the RFC 3501 section 9 formal syntax.

/// Report whether a byte is an `ATOM-CHAR`: printable US-ASCII except the atom specials
/// `( ) { SP CTL % * " \ ]`.
pub fn is_atom_byte(byte: u8) -> bool {
    (0x21..=0x7e).contains(&byte)
        && !matches!(byte, b'(' | b')' | b'{' | b'%' | b'*' | b'"' | b'\\' | b']')
}

/// Report whether a byte is an `ASTRING-CHAR`: an atom character or `]`.
pub fn is_astring_byte(byte: u8) -> bool {
    is_atom_byte(byte) || byte == b']'
}

/// Report whether a byte may appear in a tag: an astring character other than `+`.
pub fn is_tag_byte(byte: u8) -> bool {
    is_astring_byte(byte) && byte != b'+'
}

/// Report whether a byte is a `list-char`: an atom character, a list wildcard, or `]`.
pub fn is_list_byte(byte: u8) -> bool {
    is_astring_byte(byte) || matches!(byte, b'%' | b'*')
}

/// Report whether a byte string can be sent as a bare atom.
pub fn is_atom(value: &[u8]) -> bool {
    !value.is_empty() && value.iter().all(|byte| is_atom_byte(*byte))
}

/// Report whether a byte string can be sent as a quoted string: 7-bit text without CR, LF, or
/// NUL. Quotes and backslashes are escaped by the writer.
pub fn is_quotable(value: &[u8]) -> bool {
    value
        .iter()
        .all(|byte| (0x01..=0x7f).contains(byte) && !matches!(byte, b'\r' | b'\n'))
}

/// Validate SASL base64 groups and terminal padding (RFC 3501 section 9, RFC 4959 section 3).
/// The special `=` initial response is handled separately by AUTHENTICATE.
pub(crate) fn is_base64_response(value: &[u8]) -> bool {
    let padding_start = value
        .iter()
        .position(|byte| *byte == b'=')
        .unwrap_or(value.len());
    value.len().is_multiple_of(4)
        && value.len() - padding_start <= 2
        && value[..padding_start]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
        && value[padding_start..].iter().all(|byte| *byte == b'=')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SASL accepts complete groups and terminal padding, but no MIME whitespace or internal pads.
    #[test]
    fn sasl_base64_requires_complete_groups_and_terminal_padding() {
        for valid in [b"".as_slice(), b"YWJj", b"YWI=", b"YQ=="] {
            assert!(is_base64_response(valid));
        }
        for invalid in [
            b"=".as_slice(),
            b"=AAA",
            b"AAA=BBB=",
            b"A===",
            b"YWJj ",
            b"YWJ",
        ] {
            assert!(!is_base64_response(invalid));
        }
    }

    /// Atom specials, controls, and 8-bit bytes are excluded from atoms.
    #[test]
    fn atom_characters_exclude_specials() {
        for byte in b"()\x7b%*\"\\] \x00\x7f\x80".iter() {
            assert!(!is_atom_byte(*byte), "{byte:#x}");
        }
        assert!(is_atom(b"INBOX"));
        assert!(is_atom(b"$Forwarded"));
        assert!(!is_atom(b""));
        assert!(is_astring_byte(b']'));
        assert!(!is_tag_byte(b'+'));
        assert!(is_list_byte(b'%') && is_list_byte(b'*'));
    }

    /// Quoted strings carry 7-bit text but never line breaks or NUL.
    #[test]
    fn quotable_text_is_seven_bit_without_line_breaks() {
        assert!(is_quotable(b"Hello \"world\""));
        assert!(!is_quotable(b"line\r\nbreak"));
        assert!(!is_quotable(b"nul\0"));
        assert!(!is_quotable("caf\u{e9}".as_bytes()));
    }
}
