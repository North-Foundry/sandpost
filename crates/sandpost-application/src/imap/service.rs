//! Authentication for user-owned IMAP accounts.
//!
//! Authenticated mailbox operations live in [`super::session`]. The runtime adapts this service
//! to the protocol crate's backend traits, as it adapts [`SmtpAuthenticator`](crate::SmtpAuthenticator)
//! to SMTP.
use super::session::ImapAccountSession;
use super::throttle::LoginThrottle;
use crate::credentials::{
    MAXIMUM_CONCURRENT_VERIFICATIONS, MAXIMUM_CREDENTIAL_LENGTH, current_unix_seconds,
    dummy_password_hash,
};
use crate::{Application, ApplicationChange, ApplicationError, MessageEvent, instance};
use sandpost_storage::Storage;
use std::{net::IpAddr, sync::Arc};
use tokio::sync::{Semaphore, broadcast};

/// The verdict on IMAP credentials.
pub enum ImapLogin {
    Authenticated(ImapAccountSession),
    Invalid,
    Throttled,
}

/// Verifies IMAP credentials and opens account sessions.
#[derive(Clone)]
pub struct ImapService {
    storage: Arc<dyn Storage>,
    changes: broadcast::Sender<ApplicationChange>,
    messages: broadcast::Sender<MessageEvent>,
    verifications: Arc<Semaphore>,
    throttle: Arc<LoginThrottle>,
}

impl ImapService {
    /// Create the service over the application's storage and change notifications.
    pub fn new(application: &Application) -> Self {
        Self {
            storage: application.storage.clone(),
            changes: application.changes.clone(),
            messages: application.events.clone(),
            verifications: Arc::new(Semaphore::new(MAXIMUM_CONCURRENT_VERIFICATIONS)),
            throttle: Arc::new(LoginThrottle::default()),
        }
    }

    /// Verify a username and password and open a session for the account.
    ///
    /// Unknown usernames are verified against a dummy hash so timing does not reveal which
    /// usernames exist. Disabled accounts and accounts whose owner no longer exists are refused
    /// like wrong passwords. Repeated failures from one address or for one username are
    /// throttled before any verification.
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
        remote_address: Option<IpAddr>,
    ) -> Result<ImapLogin, ApplicationError> {
        if self.throttle.is_blocked(remote_address, username) {
            return Ok(ImapLogin::Throttled);
        }
        if username.len() > MAXIMUM_CREDENTIAL_LENGTH || password.len() > MAXIMUM_CREDENTIAL_LENGTH
        {
            self.throttle.record_failure(remote_address, username);
            return Ok(ImapLogin::Invalid);
        }
        let credential = self
            .storage
            .get_imap_account_credential_by_username(username)
            .await?;
        let stored_hash = match &credential {
            Some(credential) => credential.password_hash.clone(),
            None => dummy_password_hash()?,
        };
        let verified = {
            let _permit =
                self.verifications.acquire().await.map_err(|_| {
                    ApplicationError::Unavailable("IMAP authentication closed".into())
                })?;
            let password = password.to_owned();
            let hash = stored_hash.clone();
            tokio::task::spawn_blocking(move || instance::verify_password(&password, &hash)).await?
        };
        let credential = credential.filter(|credential| verified && credential.account.enabled);
        let owner_exists = match &credential {
            Some(credential) => self
                .storage
                .get_user(credential.account.owner_identifier)
                .await?
                .is_some(),
            None => false,
        };
        let Some(credential) = credential.filter(|_| owner_exists) else {
            self.throttle.record_failure(remote_address, username);
            return Ok(ImapLogin::Invalid);
        };
        if let Err(error) = self
            .storage
            .record_imap_account_use(credential.account.identifier, current_unix_seconds())
            .await
        {
            tracing::warn!(account = %credential.account.identifier, %error, "IMAP account use was not recorded");
        }
        let current = self
            .storage
            .get_imap_account_credential(credential.account.identifier)
            .await?;
        if current.is_none_or(|current| {
            !current.account.enabled
                || current.password_hash != credential.password_hash
                || current.account.owner_identifier != credential.account.owner_identifier
                || current.account.mode_generation != credential.account.mode_generation
        }) {
            self.throttle.record_failure(remote_address, username);
            return Ok(ImapLogin::Invalid);
        }
        self.throttle.record_success(remote_address, username);
        Ok(ImapLogin::Authenticated(ImapAccountSession {
            storage: self.storage.clone(),
            changes: self.changes.clone(),
            messages: self.messages.clone(),
            account_identifier: credential.account.identifier,
            owner_identifier: credential.account.owner_identifier,
            verified_password_hash: credential.password_hash,
            authenticated_mode_generation: credential.account.mode_generation,
        }))
    }
}
