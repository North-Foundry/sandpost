//! Generated passwords, credential hashing, and login-name rules shared by SMTP accesses and
//! IMAP accounts.
//!
//! Generated passwords exist in clear text only in the response that issues them; storage
//! receives an independently salted Argon2id hash.
use crate::{ApplicationError, instance};
use std::sync::OnceLock;

/// Characters of a generated password: letters and digits without the look-alikes 0/O/o and
/// 1/l/I, so a password read from the screen can be typed back reliably.
pub(crate) const PASSWORD_ALPHABET: &[u8; 56] =
    b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
/// Generated password length; 32 characters from 56 symbols carry about 185 bits of entropy.
pub(crate) const PASSWORD_LENGTH: usize = 32;
/// Longest accepted login name.
const MAXIMUM_USERNAME_LENGTH: usize = 64;
/// Shortest accepted login name.
const MINIMUM_USERNAME_LENGTH: usize = 3;
/// Longest credential an authenticator verifies; longer input is refused without hashing.
pub(crate) const MAXIMUM_CREDENTIAL_LENGTH: usize = 1024;
/// Concurrent Argon2 verifications; each uses about 19 MiB, so this bounds memory under bursts.
pub(crate) const MAXIMUM_CONCURRENT_VERIFICATIONS: usize = 4;
/// Attempts to find a free generated login name before giving up.
pub(crate) const GENERATED_USERNAME_ATTEMPTS: usize = 5;

/// Validate a chosen login name.
///
/// Login names are 3 to 64 ASCII letters, digits, and `.`, `_`, `-`, `+`, `@`, starting with a
/// letter or digit, so they paste safely into connection strings and configuration files.
pub(crate) fn validated_username(value: &str) -> Result<String, ApplicationError> {
    let value = value.trim();
    let valid = (MINIMUM_USERNAME_LENGTH..=MAXIMUM_USERNAME_LENGTH).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'@')
        });
    if valid {
        Ok(value.to_owned())
    } else {
        Err(ApplicationError::InvalidInput(
            "username must contain 3 to 64 letters, digits, or . _ - + @ and start with a letter or digit".into(),
        ))
    }
}

/// Derive a readable unique login name from a display name plus a random suffix, falling back
/// to `fallback` when the name has no usable characters.
pub(crate) fn generated_username(name: &str, fallback: &str) -> Result<String, ApplicationError> {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 32 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { fallback } else { slug };
    let suffix = instance::random_bytes::<3>()?;
    Ok(format!(
        "{slug}-{:02x}{:02x}{:02x}",
        suffix[0], suffix[1], suffix[2]
    ))
}

/// Generate a password from the operating system's cryptographically secure random source.
///
/// Rejection sampling keeps every alphabet character equally likely.
pub(crate) fn generate_password() -> Result<String, ApplicationError> {
    let mut password = String::with_capacity(PASSWORD_LENGTH);
    while password.len() < PASSWORD_LENGTH {
        for byte in instance::random_bytes::<64>()? {
            // 224 is the largest multiple of 56 not above 256.
            if byte < 224 && password.len() < PASSWORD_LENGTH {
                password.push(char::from(
                    PASSWORD_ALPHABET[usize::from(byte) % PASSWORD_ALPHABET.len()],
                ));
            }
        }
    }
    Ok(password)
}

/// Hash a password with Argon2id and a fresh random salt off the async executor.
pub(crate) async fn hash_generated_password(password: &str) -> Result<String, ApplicationError> {
    let password = password.to_owned();
    Ok(tokio::task::spawn_blocking(move || instance::hash_password(&password)).await??)
}

/// Return a fixed Argon2id hash used to verify unknown login names in constant work.
pub(crate) fn dummy_password_hash() -> Result<String, ApplicationError> {
    static DUMMY_HASH: OnceLock<String> = OnceLock::new();
    if let Some(hash) = DUMMY_HASH.get() {
        return Ok(hash.clone());
    }
    let hash = instance::hash_password(&generate_password()?)?;
    Ok(DUMMY_HASH.get_or_init(|| hash).clone())
}

/// Current Unix time in whole seconds.
pub(crate) fn current_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generated passwords have the documented length and alphabet and do not repeat.
    #[test]
    fn generated_passwords_are_long_alphanumeric_and_unique() {
        let first = generate_password().unwrap();
        let second = generate_password().unwrap();
        assert_eq!(first.len(), PASSWORD_LENGTH);
        assert!(first.bytes().all(|byte| PASSWORD_ALPHABET.contains(&byte)));
        assert!(!first.contains(['0', 'O', 'o', '1', 'l', 'I']));
        assert_ne!(first, second);
    }

    /// Generated usernames keep a readable slug of the name and a random suffix.
    #[test]
    fn generated_usernames_are_valid_slugs() {
        let username = generated_username("Laravel Development (EU)", "smtp").unwrap();
        assert!(
            username.starts_with("laravel-development-eu-"),
            "{username}"
        );
        assert!(validated_username(&username).is_ok());
        assert!(
            generated_username("!!!", "smtp")
                .unwrap()
                .starts_with("smtp-")
        );
        assert!(
            generated_username("???", "imap")
                .unwrap()
                .starts_with("imap-")
        );
    }

    /// Explicit usernames are trimmed and restricted to the documented characters.
    #[test]
    fn explicit_usernames_are_validated() {
        assert_eq!(validated_username("  ci.pipeline ").unwrap(), "ci.pipeline");
        assert!(validated_username("staging@example.test").is_ok());
        for invalid in [
            "ab",
            "-leading",
            "with space",
            "colon:name",
            &"x".repeat(65),
        ] {
            assert!(validated_username(invalid).is_err(), "{invalid}");
        }
    }
}
