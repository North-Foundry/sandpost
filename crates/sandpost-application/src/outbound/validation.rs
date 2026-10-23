//! Validation of SMTP Outbound and routing action requests.
use crate::{ApplicationError, validated_name};
use sandpost_core::{
    RoutingActionKind, SmtpOutboundAuthentication, SmtpOutboundIdentifier, SmtpOutboundSecurity,
    normalize_inbox_address,
};
use sandpost_storage::SmtpOutboundSettings;
use serde::Deserialize;
use std::net::IpAddr;

/// Most recipient patterns in one SMTP Outbound's allowlist.
pub const MAXIMUM_ALLOWED_RECIPIENT_PATTERNS: usize = 100;

/// Most recipients of one forward action.
pub const MAXIMUM_FORWARD_RECIPIENTS: usize = 50;

/// Longest accepted password, in bytes.
const MAXIMUM_PASSWORD_LENGTH: usize = 1024;

/// An SMTP Outbound definition supplied by an Owner.
///
/// Intentionally lacks Debug because it may carry a password.
#[derive(Clone, Deserialize)]
pub struct SmtpOutboundRequest {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub security: SmtpOutboundSecurity,
    #[serde(rename = "auth_type")]
    pub authentication: SmtpOutboundAuthentication,
    #[serde(default)]
    pub username: Option<String>,
    /// A new password; absent keeps the stored one when updating.
    #[serde(default)]
    pub password: Option<String>,
    /// Absent: disabled when creating, unchanged when updating.
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub override_from: Option<String>,
    #[serde(default)]
    pub envelope_from: Option<String>,
    #[serde(default, rename = "connect_timeout")]
    pub connect_timeout_seconds: Option<u32>,
    #[serde(default, rename = "smtp_timeout")]
    pub smtp_timeout_seconds: Option<u32>,
    #[serde(default = "verify_by_default", rename = "tls_verify")]
    pub verify_certificates: bool,
    #[serde(default)]
    pub helo_name: Option<String>,
    #[serde(default, rename = "max_connections")]
    pub maximum_connections: Option<u32>,
    #[serde(default)]
    pub allowed_recipients: Vec<String>,
}

/// An unsaved SMTP Outbound to test, optionally reusing a saved outbound's stored password.
///
/// Intentionally lacks Debug because it may carry a password.
#[derive(Clone, Deserialize)]
pub struct SmtpOutboundTestRequest {
    #[serde(flatten)]
    pub outbound: SmtpOutboundRequest,
    /// A saved outbound whose password is used when `password` is absent.
    #[serde(default, rename = "id")]
    pub identifier: Option<SmtpOutboundIdentifier>,
}

/// A routing action definition supplied by an Owner.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingActionRequest {
    #[serde(rename = "smtp_outbound_id")]
    pub smtp_outbound_identifier: SmtpOutboundIdentifier,
    #[serde(rename = "action_type")]
    pub kind: RoutingActionKind,
    #[serde(default)]
    pub forward_recipients: Vec<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

/**
 * Verify TLS certificates unless a request explicitly disables it.
 */
fn verify_by_default() -> bool {
    true
}

/**
 * Enable routing actions created without an explicit choice.
 */
fn enabled_by_default() -> bool {
    true
}

/**
 * Build an invalid-input error.
 */
fn invalid(message: impl Into<String>) -> ApplicationError {
    ApplicationError::InvalidInput(message.into())
}

/**
 * Report whether a label is a DNS label of letters, digits and internal hyphens.
 */
fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

/**
 * Normalize an SMTP server host: a DNS name (lowercased) or an IP literal, optionally bracketed.
 */
pub(crate) fn validated_host(value: &str) -> Result<String, ApplicationError> {
    let value = value.trim();
    let unbracketed = value
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(value);
    if let Ok(address) = unbracketed.parse::<IpAddr>() {
        return Ok(address.to_string());
    }
    let host = value.to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 || !host.split('.').all(valid_label) {
        return Err(invalid(
            "host must be a DNS name or an IP address, without a scheme or port",
        ));
    }
    Ok(host)
}

/**
 * Normalize an optional `EHLO` name; blank means the default.
 */
fn validated_helo_name(value: Option<String>) -> Result<Option<String>, ApplicationError> {
    let Some(value) = value.map(|value| value.trim().to_owned()) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 253 || !value.split('.').all(valid_label) {
        return Err(invalid("HELO name must be a DNS name"));
    }
    Ok(Some(value.to_ascii_lowercase()))
}

/**
 * Normalize an optional email address setting; blank means unset.
 */
fn validated_optional_address(
    value: Option<String>,
    field: &str,
) -> Result<Option<String>, ApplicationError> {
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(address) => normalize_inbox_address(address)
            .map(Some)
            .ok_or_else(|| invalid(format!("{field} must be a valid email address"))),
    }
}

/**
 * Check an optional bound against an inclusive range.
 */
fn validated_bound(
    value: Option<u32>,
    range: std::ops::RangeInclusive<u32>,
    field: &str,
) -> Result<Option<u32>, ApplicationError> {
    match value {
        Some(value) if !range.contains(&value) => Err(invalid(format!(
            "{field} must be between {} and {}",
            range.start(),
            range.end()
        ))),
        value => Ok(value),
    }
}

/**
 * Normalize a recipient allowlist pattern: an address glob where `*` matches any run and `?` one
 * character, compared ignoring ASCII case.
 */
fn validated_pattern(pattern: &str) -> Result<String, ApplicationError> {
    let pattern = pattern.trim().to_ascii_lowercase();
    let allowed = |byte: u8| {
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
                    | b'.'
                    | b'@'
            )
    };
    if pattern.is_empty() || pattern.len() > 254 || !pattern.bytes().all(allowed) {
        return Err(invalid(format!(
            "\"{pattern}\" is not a valid recipient pattern; use addresses with * and ? wildcards"
        )));
    }
    Ok(pattern)
}

/**
 * Normalize, deduplicate and bound a recipient allowlist, keeping its order.
 */
pub(crate) fn validated_patterns(patterns: Vec<String>) -> Result<Vec<String>, ApplicationError> {
    let mut validated: Vec<String> = Vec::new();
    for pattern in patterns {
        if pattern.trim().is_empty() {
            continue;
        }
        let pattern = validated_pattern(&pattern)?;
        if !validated.contains(&pattern) {
            validated.push(pattern);
        }
    }
    if validated.len() > MAXIMUM_ALLOWED_RECIPIENT_PATTERNS {
        return Err(invalid(format!(
            "at most {MAXIMUM_ALLOWED_RECIPIENT_PATTERNS} recipient patterns are allowed"
        )));
    }
    Ok(validated)
}

/**
 * Report whether a recipient address is permitted by an allowlist; an empty list permits nobody.
 */
pub(crate) fn recipient_allowed(address: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| sandpost_query::glob_matches(address, pattern, true))
}

/**
 * Validate a password supplied in a request.
 */
fn validated_password(password: &str) -> Result<(), ApplicationError> {
    if password.is_empty()
        || password.len() > MAXIMUM_PASSWORD_LENGTH
        || password
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
    {
        return Err(invalid(
            "password must contain 1 to 1024 bytes without line breaks or NUL",
        ));
    }
    Ok(())
}

/**
 * Validate an SMTP Outbound request into storage settings and the password it supplies.
 *
 * `password_stored` tells whether an existing outbound already has a password, so an update may
 * keep it; `enabled` is the value used when the request does not choose one.
 */
pub(crate) fn validated_outbound(
    request: SmtpOutboundRequest,
    password_stored: bool,
    enabled: bool,
) -> Result<(SmtpOutboundSettings, Option<String>), ApplicationError> {
    let name = validated_name(&request.name)?;
    let host = validated_host(&request.host)?;
    if request.port == 0 {
        return Err(invalid("port must be between 1 and 65535"));
    }
    let authenticates = request.authentication != SmtpOutboundAuthentication::None;
    let username = request
        .username
        .map(|username| username.trim().to_owned())
        .filter(|username| !username.is_empty());
    if let Some(username) = &username
        && (username.len() > 255 || username.chars().any(char::is_control))
    {
        return Err(invalid(
            "username must contain 1 to 255 printable characters",
        ));
    }
    let password = request.password.filter(|password| !password.is_empty());
    if let Some(password) = &password {
        validated_password(password)?;
    }
    if authenticates {
        if username.is_none() {
            return Err(invalid("a username is required for authentication"));
        }
        if password.is_none() && !password_stored {
            return Err(invalid("a password is required for authentication"));
        }
    }
    Ok((
        SmtpOutboundSettings {
            name,
            host,
            port: request.port,
            security: request.security,
            authentication: request.authentication,
            username: if authenticates { username } else { None },
            enabled: request.enabled.unwrap_or(enabled),
            override_from: validated_optional_address(request.override_from, "override From")?,
            envelope_from: validated_optional_address(request.envelope_from, "envelope From")?,
            connect_timeout_seconds: validated_bound(
                request.connect_timeout_seconds,
                1..=300,
                "connect timeout",
            )?,
            smtp_timeout_seconds: validated_bound(
                request.smtp_timeout_seconds,
                1..=600,
                "SMTP timeout",
            )?,
            verify_certificates: request.verify_certificates,
            helo_name: validated_helo_name(request.helo_name)?,
            maximum_connections: validated_bound(
                request.maximum_connections,
                1..=16,
                "max connections",
            )?,
            allowed_recipients: validated_patterns(request.allowed_recipients)?,
        },
        if authenticates { password } else { None },
    ))
}

/**
 * Validate a routing action's recipients for its type and the SMTP Outbound's allowlist.
 *
 * Relay actions take no recipients; forward actions need 1 to 50 valid addresses, normalized and
 * deduplicated, each permitted by the outbound's allowlist.
 */
pub(crate) fn validated_forward_recipients(
    kind: RoutingActionKind,
    recipients: Vec<String>,
    allowed_recipients: &[String],
) -> Result<Vec<String>, ApplicationError> {
    match kind {
        RoutingActionKind::Relay => {
            if recipients
                .iter()
                .any(|recipient| !recipient.trim().is_empty())
            {
                return Err(invalid(
                    "relay actions deliver to the original recipients and take no forward recipients",
                ));
            }
            Ok(Vec::new())
        }
        RoutingActionKind::Forward => {
            let mut normalized: Vec<String> = Vec::new();
            for recipient in recipients {
                if recipient.trim().is_empty() {
                    continue;
                }
                let address = normalize_inbox_address(&recipient).ok_or_else(|| {
                    invalid(format!(
                        "\"{}\" is not a valid email address",
                        recipient.trim()
                    ))
                })?;
                if !normalized.contains(&address) {
                    normalized.push(address);
                }
            }
            if normalized.is_empty() || normalized.len() > MAXIMUM_FORWARD_RECIPIENTS {
                return Err(invalid(format!(
                    "forward actions need 1 to {MAXIMUM_FORWARD_RECIPIENTS} recipients"
                )));
            }
            let refused: Vec<&str> = normalized
                .iter()
                .filter(|address| !recipient_allowed(address, allowed_recipients))
                .map(String::as_str)
                .collect();
            if !refused.is_empty() {
                return Err(invalid(format!(
                    "the SMTP Outbound's recipient allowlist does not permit {}",
                    refused.join(", ")
                )));
            }
            Ok(normalized)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        recipient_allowed, validated_forward_recipients, validated_host, validated_patterns,
    };
    use sandpost_core::RoutingActionKind;

    /**
     * Hosts accept DNS names and IP literals and refuse URLs and ports.
     */
    #[test]
    fn hosts_are_names_or_addresses() {
        assert_eq!(
            validated_host(" SMTP.Example.com ").unwrap(),
            "smtp.example.com"
        );
        assert_eq!(validated_host("[::1]").unwrap(), "::1");
        assert_eq!(validated_host("192.0.2.1").unwrap(), "192.0.2.1");
        for invalid in [
            "smtp://example.com",
            "example.com:587",
            "",
            "-bad.example",
            "a b",
        ] {
            assert!(validated_host(invalid).is_err(), "{invalid}");
        }
    }

    /**
     * Allowlists are normalized globs and an empty allowlist permits nobody.
     */
    #[test]
    fn allowlists_match_whole_addresses_ignoring_case() {
        let patterns = validated_patterns(vec![
            "*@Example.TEST".into(),
            " ".into(),
            "qa+?@corp.test".into(),
        ])
        .unwrap();
        assert_eq!(patterns, ["*@example.test", "qa+?@corp.test"]);
        assert!(recipient_allowed("Someone@example.test", &patterns));
        assert!(recipient_allowed("qa+1@corp.test", &patterns));
        assert!(!recipient_allowed("qa+12@corp.test", &patterns));
        assert!(!recipient_allowed("someone@example.test.evil", &patterns));
        assert!(!recipient_allowed("anyone@example.test", &[]));
        assert!(recipient_allowed("anyone@anywhere.test", &["*".to_owned()]));
        assert!(validated_patterns(vec!["bad pattern".into()]).is_err());
    }

    /**
     * Forward recipients are normalized, deduplicated and checked against the allowlist, and
     * relay actions take none.
     */
    #[test]
    fn forward_recipients_follow_the_allowlist() {
        let allowlist = vec!["*@example.test".to_owned()];
        assert_eq!(
            validated_forward_recipients(
                RoutingActionKind::Forward,
                vec!["QA@example.test".into(), "qa@example.test".into()],
                &allowlist
            )
            .unwrap(),
            ["qa@example.test"]
        );
        assert!(
            validated_forward_recipients(
                RoutingActionKind::Forward,
                vec!["qa@elsewhere.test".into()],
                &allowlist
            )
            .is_err()
        );
        assert!(
            validated_forward_recipients(RoutingActionKind::Forward, Vec::new(), &allowlist)
                .is_err()
        );
        assert!(
            validated_forward_recipients(
                RoutingActionKind::Relay,
                vec!["qa@example.test".into()],
                &allowlist
            )
            .is_err()
        );
    }
}
