//! Mailbox normalization shared by envelopes and address headers.
use crate::MimeError;
use sandpost_core::Mailbox;

/// Normalize an RFC 5322 addr-spec into a [`Mailbox`].
///
/// Surrounding whitespace is removed, while whitespace inside the addr-spec must follow the
/// quoted-string grammar. ASCII letters in the entire address are lowercased for header
/// comparison; non-ASCII characters and quoted-pair escapes are preserved. SMTP envelope
/// semantics are handled by their caller, and this function applies no SMTP byte limits.
///
/// ```
/// let mailbox = sandpost_mime::normalize_mailbox(" Alice@Example.COM ")?;
/// assert_eq!(mailbox.address, "alice@example.com");
/// assert_eq!(mailbox.domain, "example.com");
/// # Ok::<(), sandpost_mime::MimeError>(())
/// ```
pub fn normalize_mailbox(address: &str) -> Result<Mailbox, MimeError> {
    let invalid_mailbox = || MimeError::InvalidMailbox(address.to_owned());
    if address
        .chars()
        .any(|character| character.is_ascii_control() && character != '\t')
    {
        return Err(invalid_mailbox());
    }
    let trimmed_address = address.trim_matches([' ', '\t']);
    if trimmed_address.is_empty() {
        return Err(invalid_mailbox());
    }

    let (local_part, domain) =
        split_address_specification(trimmed_address).ok_or_else(invalid_mailbox)?;
    let local_part = local_part.trim_matches([' ', '\t']);
    let domain = domain.trim_matches([' ', '\t']);
    if !valid_local_part(local_part) || !valid_domain(domain) {
        return Err(invalid_mailbox());
    }

    let normalized_address = format!("{local_part}@{domain}").to_ascii_lowercase();
    let normalized_domain = domain.to_ascii_lowercase();
    Ok(Mailbox {
        address: normalized_address,
        domain: normalized_domain,
    })
}

/// Split an addr-spec at its one unquoted `@` separator.
fn split_address_specification(address: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    let mut escaped = false;

    for (byte_index, character) in address.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && character == '\\' {
            escaped = true;
            continue;
        }
        match character {
            '"' => quoted = !quoted,
            '@' if !quoted => {
                return Some((&address[..byte_index], &address[byte_index + 1..]));
            }
            _ => {}
        }
    }

    None
}

/// Check a local-part as either a dot-atom or a quoted string.
fn valid_local_part(local_part: &str) -> bool {
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in local_part.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '.' if !quoted => {
                if !valid_local_word(&local_part[start..index]) {
                    return false;
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    !quoted && !escaped && valid_local_word(&local_part[start..])
}

/// Recognize a word in modern or obsolete local-part syntax, including empty quoted words.
fn valid_local_word(word: &str) -> bool {
    if word.starts_with('"') {
        valid_quoted_string(word)
    } else {
        !word.is_empty() && word.chars().all(is_address_text_character)
    }
}

/// Check a nonempty dot-separated sequence of RFC atext characters.
fn valid_dot_atom(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|atom| !atom.is_empty() && atom.chars().all(is_address_text_character))
}

/// Check the content and closing quote of an RFC quoted string.
fn valid_quoted_string(value: &str) -> bool {
    let Some(content) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return false;
    };
    let mut escaped = false;

    for character in content.chars() {
        if escaped {
            if !is_quoted_pair_character(character) {
                return false;
            }
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if !is_quoted_text_character(character) {
            return false;
        }
    }

    !escaped
}

/// Check the domain as a dot-atom or a domain literal.
fn valid_domain(domain: &str) -> bool {
    if domain.starts_with('[') {
        valid_domain_literal(domain)
    } else {
        valid_dot_atom(domain)
    }
}

/// Check RFC 5322 domain-literal syntax; SMTP address-literal semantics belong to SMTP.
fn valid_domain_literal(domain: &str) -> bool {
    let Some(content) = domain
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };

    let mut escaped = false;
    for character in content.chars() {
        if escaped {
            if !is_quoted_pair_character(character) {
                return false;
            }
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if !is_domain_text_character(character) {
            return false;
        }
    }
    if escaped {
        return false;
    }

    true
}

/// Check whether a character is permitted in an RFC atext atom.
fn is_address_text_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(
            character,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '/'
                | '='
                | '?'
                | '^'
                | '_'
                | '`'
                | '{'
                | '|'
                | '}'
                | '~'
        )
        || is_utf8_non_ascii(character)
}

/// Check whether a character is permitted unescaped in a quoted string.
fn is_quoted_text_character(character: char) -> bool {
    matches!(character, ' ' | '\t')
        || matches!(character as u32, 33 | 35..=91 | 93..=126)
        || is_utf8_non_ascii(character)
}

/// Check whether a character is permitted after a quoted-pair backslash.
fn is_quoted_pair_character(character: char) -> bool {
    matches!(character, ' ' | '\t')
        || matches!(character as u32, 33..=126)
        || is_utf8_non_ascii(character)
}

/// Check whether a character is permitted unescaped in a domain literal.
fn is_domain_text_character(character: char) -> bool {
    matches!(character, ' ' | '\t')
        || matches!(character as u32, 33..=90 | 94..=126)
        || is_utf8_non_ascii(character)
}

/// Check for visible non-ASCII UTF-8 characters allowed by RFC 6532.
fn is_utf8_non_ascii(character: char) -> bool {
    !character.is_ascii()
}
