//! RFC 5321 mailbox and domain syntax used by SMTP commands.
use sandpost_core::Mailbox;
use std::net::Ipv6Addr;

/// Parse an ASCII SMTP mailbox without source routes or the unqualified Postmaster exception.
///
/// The returned address preserves the local-part spelling and lowercases only the domain.
/// Transport paths and unqualified Postmaster recipients are handled separately.
pub(crate) fn parse_mailbox(address: &str) -> Result<Mailbox, ()> {
    if !address.is_ascii() || address.len() + 2 > 256 {
        return Err(());
    }
    let separator = mailbox_separator(address).ok_or(())?;
    let (local_part, domain_with_separator) = address.split_at(separator);
    let domain = &domain_with_separator[1..];
    if local_part.len() > 64
        || domain.len() > 255
        || !valid_local_part(local_part)
        || !is_valid_domain(domain)
    {
        return Err(());
    }
    let domain = domain.to_ascii_lowercase();
    Ok(Mailbox {
        address: format!("{local_part}@{domain}"),
        domain,
    })
}

/// Validate the complete path length, then discard a validated obsolete source route.
pub(crate) fn parse_path(address: &str) -> Result<Mailbox, ()> {
    if !address.is_ascii() || address.len() + 2 > 256 {
        return Err(());
    }
    parse_mailbox(strip_source_route(address)?)
}

/// Validate an RFC 5321 domain or address literal, including single-label domains.
pub(crate) fn is_valid_domain(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > 255 || !domain.is_ascii() {
        return false;
    }
    if domain.starts_with('[') && domain.ends_with(']') {
        return valid_address_literal(&domain[1..domain.len() - 1]);
    }
    domain.split('.').all(valid_subdomain)
}

/// Remove and validate an obsolete source route, if present.
fn strip_source_route(address: &str) -> Result<&str, ()> {
    if !address.starts_with('@') {
        return Ok(address);
    }
    let mut in_literal = false;
    let Some(separator) = address.char_indices().find_map(|(index, character)| {
        match character {
            '[' if !in_literal => in_literal = true,
            ']' if in_literal => in_literal = false,
            ':' if !in_literal => return Some(index),
            _ => {}
        }
        None
    }) else {
        return Err(());
    };
    let route = &address[..separator];
    if route.is_empty() {
        return Err(());
    }
    for routed_domain in route.split(',') {
        let Some(domain) = routed_domain.strip_prefix('@') else {
            return Err(());
        };
        if !is_valid_domain(domain) || domain.starts_with('[') {
            return Err(());
        }
    }
    let mailbox = &address[separator + 1..];
    if mailbox.is_empty() || mailbox.starts_with('@') {
        return Err(());
    }
    Ok(mailbox)
}

/// Find the one `@` separating a local-part from its domain, ignoring quoted text.
fn mailbox_separator(address: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    let mut separator = None;
    for (index, character) in address.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if !quoted && character == '@' && separator.replace(index).is_some() {
            return None;
        }
    }
    if quoted || escaped { None } else { separator }
}

/// Validate an RFC 5321 dot-string or quoted-string local-part.
fn valid_local_part(local_part: &str) -> bool {
    if local_part.is_empty() {
        return false;
    }
    if local_part.starts_with('"') {
        return valid_quoted_local_part(local_part);
    }
    local_part
        .split('.')
        .all(|atom| !atom.is_empty() && atom.bytes().all(is_atext))
}

/// Validate quoted-string bytes, including quoted-pairs, with no controls or non-ASCII bytes.
fn valid_quoted_local_part(local_part: &str) -> bool {
    let bytes = local_part.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || *bytes.last().unwrap() != b'"' {
        return false;
    }
    let mut index = 1;
    while index + 1 < bytes.len() {
        match bytes[index] {
            b'\\' if index + 2 < bytes.len() && (0x20..=0x7e).contains(&bytes[index + 1]) => {
                index += 2;
            }
            b' '..=b'!' | b'#'..=b'[' | b']'..=b'~' => index += 1,
            _ => return false,
        }
    }
    index + 1 == bytes.len() && bytes[index] == b'"'
}

/// Return whether a byte is an RFC 5321 `atext` character.
fn is_atext(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&byte)
}

/// Validate a domain label with alphanumeric ends and optional internal hyphens.
fn valid_subdomain(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && label
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Validate an IPv4, IPv6, or tagged RFC 5321 address literal.
fn valid_address_literal(literal: &str) -> bool {
    if literal
        .get(..5)
        .is_some_and(|tag| tag.eq_ignore_ascii_case("IPv6:"))
    {
        return literal[5..].parse::<Ipv6Addr>().is_ok();
    }
    // IPv6 is the only registered tagged literal; do not accept invented address types.
    let octets: Vec<_> = literal.split('.').collect();
    octets.len() == 4
        && octets.iter().all(|octet| {
            !octet.is_empty()
                && octet.len() <= 3
                && octet.bytes().all(|byte| byte.is_ascii_digit())
                && octet.parse::<u8>().is_ok()
        })
}
