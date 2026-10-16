//! Shared validation and normalization policy for recipient-inbox addresses.

/// Normalize an inbox address and accept only safe ASCII dot-atom mailboxes.
///
/// Surrounding whitespace is trimmed and ASCII letters are lowercased. The address is limited
/// to 254 bytes, its local part to 1–64 bytes, and its domain to 253 bytes. Domain labels may be
/// single-label names for local development, but each label must be 1–63 ASCII alphanumerics
/// with optional internal hyphens. `None` means the input is outside this inbox address policy.
pub fn normalize_inbox_address(address: &str) -> Option<String> {
    if !address.is_ascii() || address.bytes().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    let address = address.trim().to_ascii_lowercase();
    if address.len() > 254 {
        return None;
    }
    let (local_part, domain) = address.split_once('@')?;
    if local_part.len() > 64 || !valid_inbox_local_part(local_part) || domain.len() > 253 {
        return None;
    }
    if !domain.split('.').all(valid_inbox_domain_label) {
        return None;
    }
    Some(address)
}

/// Check a nonempty dot-separated ASCII atext local part.
fn valid_inbox_local_part(local_part: &str) -> bool {
    !local_part.is_empty()
        && local_part.split('.').all(|atom| {
            !atom.is_empty()
                && atom.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(
                            byte,
                            b'!' | b'#'
                                | b'$'
                                | b'%'
                                | b'&'
                                | b'\''
                                | b'*'
                                | b'+'
                                | b'-'
                                | b'/'
                                | b'='
                                | b'?'
                                | b'^'
                                | b'_'
                                | b'`'
                                | b'{'
                                | b'|'
                                | b'}'
                                | b'~'
                        )
                })
        })
}

/// Check one DNS-style label with optional internal hyphens.
fn valid_inbox_domain_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label.as_bytes()[0].is_ascii_alphanumeric()
        && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}
