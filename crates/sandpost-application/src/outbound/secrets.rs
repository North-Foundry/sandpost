//! Encryption of SMTP Outbound passwords with a key kept outside the database.
//!
//! An SMTP Outbound password must be recoverable, because Sandpost presents it to the external
//! server; it is therefore encrypted rather than hashed. The 256-bit key lives in its own file
//! (by default `outbound-secret.key` in the data directory), never in SQLite, so a copy of the
//! database alone does not reveal any password. Each value is sealed with ChaCha20-Poly1305 and a
//! fresh random nonce, and authenticated together with the SMTP Outbound identifier, so a stored
//! value copied onto another outbound fails to decrypt instead of leaking.
use crate::{ApplicationError, instance};
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use sandpost_core::SmtpOutboundIdentifier;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// The format version written as the first byte of every sealed value.
const SEALED_VERSION: u8 = 1;

/// Key length in bytes.
const KEY_LENGTH: usize = 32;

/// Domain separation prefix of the associated data.
const ASSOCIATED_DATA_PREFIX: &[u8] = b"sandpost smtp outbound password:";

/// The key that seals SMTP Outbound passwords.
///
/// Its Debug output never shows the key material.
#[derive(Clone)]
pub struct OutboundSecretKey {
    material: Arc<[u8; KEY_LENGTH]>,
}

impl fmt::Debug for OutboundSecretKey {
    /**
     * Describe the key without revealing it.
     */
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OutboundSecretKey(<redacted>)")
    }
}

impl OutboundSecretKey {
    /**
     * Build a key from raw bytes, for embedding hosts and tests.
     */
    pub fn from_bytes(material: [u8; KEY_LENGTH]) -> Self {
        Self {
            material: Arc::new(material),
        }
    }

    /**
     * Generate a fresh random key from the operating system's secure random source.
     */
    pub fn generate() -> Result<Self, ApplicationError> {
        Ok(Self::from_bytes(instance::random_bytes::<KEY_LENGTH>()?))
    }

    /**
     * Read the key stored at `path` as 64 hexadecimal characters, creating the file with a new
     * random key when it does not exist.
     *
     * A new file is created exclusively and, on Unix, readable only by its owner. An existing file
     * that is malformed is an error rather than replaced, because replacing it would make every
     * stored password unrecoverable.
     */
    pub fn load_or_create(path: &Path) -> Result<Self, ApplicationError> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::parse(contents.trim()).ok_or_else(|| {
                ApplicationError::InvalidInput(format!(
                    "{} does not contain a 64-character hexadecimal key",
                    path.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let key = Self::generate()?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                let mut file = options.open(path)?;
                file.write_all(key.hexadecimal().as_bytes())?;
                file.write_all(b"\n")?;
                file.sync_all()?;
                Ok(key)
            }
            Err(error) => Err(error.into()),
        }
    }

    /**
     * Parse 64 hexadecimal characters into a key.
     */
    fn parse(text: &str) -> Option<Self> {
        if text.len() != KEY_LENGTH * 2 || !text.is_ascii() {
            return None;
        }
        let mut material = [0_u8; KEY_LENGTH];
        for (index, byte) in material.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
        }
        Some(Self::from_bytes(material))
    }

    /**
     * Return the key as lowercase hexadecimal, the format of the key file.
     */
    fn hexadecimal(&self) -> String {
        self.material
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /**
     * Build the AEAD key.
     */
    fn aead(&self) -> Result<LessSafeKey, ApplicationError> {
        UnboundKey::new(&CHACHA20_POLY1305, self.material.as_slice())
            .map(LessSafeKey::new)
            .map_err(|_| ApplicationError::Unavailable("the secret key is unusable".into()))
    }

    /**
     * Encrypt a password for one SMTP Outbound: version byte, random nonce, ciphertext and tag.
     */
    pub(crate) fn seal(
        &self,
        outbound: SmtpOutboundIdentifier,
        password: &str,
    ) -> Result<Vec<u8>, ApplicationError> {
        let nonce_bytes = instance::random_bytes::<NONCE_LEN>()?;
        let mut sealed = password.as_bytes().to_vec();
        self.aead()?
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(associated_data(outbound)),
                &mut sealed,
            )
            .map_err(|_| {
                ApplicationError::Unavailable("the password could not be encrypted".into())
            })?;
        let mut stored = Vec::with_capacity(1 + NONCE_LEN + sealed.len());
        stored.push(SEALED_VERSION);
        stored.extend_from_slice(&nonce_bytes);
        stored.extend_from_slice(&sealed);
        Ok(stored)
    }

    /**
     * Decrypt a password sealed for this SMTP Outbound, returning `None` when the value was
     * sealed with another key, for another outbound, or was altered.
     */
    pub(crate) fn open(&self, outbound: SmtpOutboundIdentifier, stored: &[u8]) -> Option<String> {
        let (&version, rest) = stored.split_first()?;
        if version != SEALED_VERSION || rest.len() < NONCE_LEN {
            return None;
        }
        let (nonce, sealed) = rest.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
        let mut buffer = sealed.to_vec();
        let plaintext = self
            .aead()
            .ok()?
            .open_in_place(nonce, Aad::from(associated_data(outbound)), &mut buffer)
            .ok()?;
        String::from_utf8(plaintext.to_vec()).ok()
    }
}

/**
 * Bind a sealed value to the SMTP Outbound it belongs to.
 */
fn associated_data(outbound: SmtpOutboundIdentifier) -> Vec<u8> {
    let mut data = ASSOCIATED_DATA_PREFIX.to_vec();
    data.extend_from_slice(outbound.to_string().as_bytes());
    data
}

#[cfg(test)]
mod tests {
    use super::OutboundSecretKey;
    use sandpost_core::SmtpOutboundIdentifier;

    /**
     * Sealed passwords round-trip only with the same key and outbound and never contain the
     * clear text.
     */
    #[test]
    fn sealed_passwords_are_bound_to_their_key_and_outbound() {
        let key = OutboundSecretKey::generate().unwrap();
        let outbound = SmtpOutboundIdentifier::new();
        let sealed = key.seal(outbound, "correct horse battery").unwrap();
        assert!(!sealed.windows(7).any(|window| window == b"correct"));
        assert_ne!(sealed, key.seal(outbound, "correct horse battery").unwrap());
        assert_eq!(
            key.open(outbound, &sealed).as_deref(),
            Some("correct horse battery")
        );
        assert_eq!(key.open(SmtpOutboundIdentifier::new(), &sealed), None);
        assert_eq!(
            OutboundSecretKey::generate()
                .unwrap()
                .open(outbound, &sealed),
            None
        );
        let mut altered = sealed.clone();
        *altered.last_mut().unwrap() ^= 1;
        assert_eq!(key.open(outbound, &altered), None);
        assert!(!format!("{key:?}").contains(&key.hexadecimal()));
    }

    /**
     * The key file is created once, reused afterwards, and malformed files are refused.
     */
    #[test]
    fn key_files_are_created_once_and_validated() {
        let directory =
            std::env::temp_dir().join(format!("sandpost-secret-{}", uuid::Uuid::new_v4()));
        let path = directory.join("outbound-secret.key");
        let created = OutboundSecretKey::load_or_create(&path).unwrap();
        let reloaded = OutboundSecretKey::load_or_create(&path).unwrap();
        assert_eq!(created.hexadecimal(), reloaded.hexadecimal());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "only the owner may read the key");
        }
        std::fs::write(&path, "not a key").unwrap();
        assert!(OutboundSecretKey::load_or_create(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
