//! Smtp server storage capability.
use crate::{
    NewSmtpAccess, SmtpAccess, SmtpAccessCredential, SmtpAccessIdentifier,
    SmtpAuthenticationMechanism, StorageError,
};
use async_trait::async_trait;
use std::net::SocketAddr;

/// The single global SMTP server's settings and its centrally administered access credentials.
///
/// Passwords cross this boundary only as one-way hashes. Listing and single-access reads never
/// return hashes; only the credential reads used by authentication do.
#[async_trait]
pub trait SmtpServerStorage: Send + Sync {
    /// Load the persisted listener address of the global SMTP server; `None` means disabled.
    async fn smtp_listen_address(&self) -> Result<Option<SocketAddr>, StorageError>;

    /// Replace the persisted listener address of the global SMTP server.
    async fn save_smtp_listen_address(
        &self,
        address: Option<SocketAddr>,
    ) -> Result<(), StorageError>;

    /// List every SMTP access ordered by name and identifier, without password hashes.
    async fn list_smtp_accesses(&self) -> Result<Vec<SmtpAccess>, StorageError>;

    /// Load one SMTP access by identifier, without its password hash.
    async fn get_smtp_access(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<Option<SmtpAccess>, StorageError>;

    /// Load an access and its password hash by identifier for delivery revalidation.
    async fn get_smtp_access_credential(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<Option<SmtpAccessCredential>, StorageError>;

    /// Load an access and its password hash by exact username for SMTP authentication.
    async fn get_smtp_access_credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<SmtpAccessCredential>, StorageError>;

    /// Create an enabled access; a taken username fails with DuplicateSmtpUsername.
    async fn create_smtp_access(&self, access: &NewSmtpAccess) -> Result<SmtpAccess, StorageError>;

    /// Atomically replace an access's password hash, returning None for an unknown access.
    ///
    /// The previous hash stops verifying as soon as the operation commits.
    async fn replace_smtp_access_password(
        &self,
        identifier: SmtpAccessIdentifier,
        password_hash: &str,
    ) -> Result<Option<SmtpAccess>, StorageError>;

    /// Enable or disable an access, returning None for an unknown access.
    async fn set_smtp_access_enabled(
        &self,
        identifier: SmtpAccessIdentifier,
        enabled: bool,
    ) -> Result<Option<SmtpAccess>, StorageError>;

    /// Replace how an access may authenticate, returning None for an unknown access.
    ///
    /// `allowed_mechanisms` is never empty; the change applies to the next authentication and to
    /// deliveries of sessions that authenticated earlier.
    async fn set_smtp_access_authentication_rules(
        &self,
        identifier: SmtpAccessIdentifier,
        requires_encryption: bool,
        allowed_mechanisms: &[SmtpAuthenticationMechanism],
    ) -> Result<Option<SmtpAccess>, StorageError>;

    /// Delete an access, reporting whether it existed.
    async fn delete_smtp_access(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<bool, StorageError>;

    /// Record a successful authentication time.
    ///
    /// Backends may skip the write when the stored time is less than a minute older, so bursts
    /// of SMTP sessions do not turn every authentication into a write.
    async fn record_smtp_access_use(
        &self,
        identifier: SmtpAccessIdentifier,
        used_at: i64,
    ) -> Result<(), StorageError>;
}
