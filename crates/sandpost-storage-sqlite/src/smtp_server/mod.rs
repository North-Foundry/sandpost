//! Async adapter for SMTP listener configuration and submission credentials.
mod accesses;
mod listener;

use crate::SqliteStorage;
use crate::error::StorageResult;
use accesses::{AccessUpdateValue, mechanism_flags};
use async_trait::async_trait;
use sandpost_core::{SmtpAccess, SmtpAccessIdentifier, SmtpAuthenticationMechanism};
use sandpost_storage::{NewSmtpAccess, SmtpAccessCredential, SmtpServerStorage, StorageError};
use std::net::SocketAddr;

#[async_trait]
impl SmtpServerStorage for SqliteStorage {
    /// Read the global SMTP listener address, or None when disabled.
    async fn smtp_listen_address(&self) -> Result<Option<SocketAddr>, StorageError> {
        listener::listen_address(self).await
    }

    /// Replace or disable the global SMTP listener address.
    async fn save_smtp_listen_address(
        &self,
        address: Option<SocketAddr>,
    ) -> Result<(), StorageError> {
        listener::save_listen_address(self, address).await
    }

    /// List SMTP accesses without exposing password hashes.
    async fn list_smtp_accesses(&self) -> Result<Vec<SmtpAccess>, StorageError> {
        accesses::list_accesses(self).await
    }

    /// Read one SMTP access without its password hash.
    async fn get_smtp_access(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<Option<SmtpAccess>, StorageError> {
        accesses::get_access(self, identifier).await
    }

    /// Read the credential hash and public access record by identifier.
    async fn get_smtp_access_credential(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<Option<SmtpAccessCredential>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        accesses::get_credential(&mut connection, "identifier", &identifier.to_string()).await
    }

    /// Read the credential hash and public access record by username.
    async fn get_smtp_access_credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<SmtpAccessCredential>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        accesses::get_credential(&mut connection, "username", username).await
    }

    /// Create an enabled SMTP credential with a unique username and validated mechanism policy.
    async fn create_smtp_access(&self, access: &NewSmtpAccess) -> Result<SmtpAccess, StorageError> {
        accesses::create_access(self, access).await
    }

    /// Replace the credential hash and return the updated public record.
    async fn replace_smtp_access_password(
        &self,
        identifier: SmtpAccessIdentifier,
        password_hash: &str,
    ) -> Result<Option<SmtpAccess>, StorageError> {
        accesses::update_access(
            self,
            identifier,
            "UPDATE smtp_accesses SET password_hash = ?2, updated_at = ?3 WHERE identifier = ?1",
            AccessUpdateValue::Password(password_hash),
        )
        .await
    }

    /// Set whether the SMTP access may authenticate and return its updated record.
    async fn set_smtp_access_enabled(
        &self,
        identifier: SmtpAccessIdentifier,
        enabled: bool,
    ) -> Result<Option<SmtpAccess>, StorageError> {
        accesses::update_access(
            self,
            identifier,
            "UPDATE smtp_accesses SET enabled = ?2, updated_at = ?3 WHERE identifier = ?1",
            AccessUpdateValue::Enabled(enabled),
        )
        .await
    }

    /// Replace the TLS requirement and nonempty set of allowed authentication mechanisms.
    async fn set_smtp_access_authentication_rules(
        &self,
        identifier: SmtpAccessIdentifier,
        requires_encryption: bool,
        allowed_mechanisms: &[SmtpAuthenticationMechanism],
    ) -> Result<Option<SmtpAccess>, StorageError> {
        let (plain_allowed, login_allowed) = mechanism_flags(allowed_mechanisms)?;
        accesses::set_authentication_rules(
            self,
            identifier,
            requires_encryption,
            plain_allowed,
            login_allowed,
        )
        .await
    }

    /// Delete a credential without deleting its captured messages.
    async fn delete_smtp_access(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<bool, StorageError> {
        accesses::delete_access(self, identifier).await
    }

    /// Record successful authentication while limiting timestamp writes to once a minute.
    async fn record_smtp_access_use(
        &self,
        identifier: SmtpAccessIdentifier,
        used_at: i64,
    ) -> Result<(), StorageError> {
        accesses::record_access_use(self, identifier, used_at).await
    }
}
