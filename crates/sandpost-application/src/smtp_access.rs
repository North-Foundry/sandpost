//! Central administration of the global SMTP server and its SMTP access credentials, plus the
//! credential verification the SMTP transport uses.
//!
//! Every management operation requires a global administrator (owner or admin). Generated
//! passwords exist in clear text only inside the creation or regeneration response; storage
//! receives an independently salted Argon2id hash and no read operation returns either.
use crate::credentials::{
    GENERATED_USERNAME_ATTEMPTS, MAXIMUM_CONCURRENT_VERIFICATIONS, MAXIMUM_CREDENTIAL_LENGTH,
    current_unix_seconds, dummy_password_hash, generate_password, generated_username,
    hash_generated_password, validated_username,
};
use crate::{ApplicationError, AuthorizedApplication, MailListenerStatus, instance};
use sandpost_core::{SmtpAccess, SmtpAccessIdentifier, SmtpAuthenticationMechanism};
use sandpost_storage::{NewSmtpAccess, Storage, StorageError};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Administrator input for a new SMTP access; the password is always generated.
#[derive(Deserialize)]
pub struct NewSmtpAccessRequest {
    pub name: String,
    /// Optional explicit username; a unique one is derived from the name when omitted.
    #[serde(default)]
    pub username: Option<String>,
    /// Accept the access only on TLS-protected sessions; off when omitted.
    #[serde(default)]
    pub requires_encryption: bool,
    /// The AUTH mechanisms the access may use; every mechanism when omitted, never empty.
    #[serde(default = "every_mechanism")]
    pub allowed_mechanisms: Vec<SmtpAuthenticationMechanism>,
}

/// Every AUTH mechanism, the default for new accesses.
fn every_mechanism() -> Vec<SmtpAuthenticationMechanism> {
    SmtpAuthenticationMechanism::ALL.to_vec()
}

/// Partial change to an existing SMTP access; omitted fields keep their value.
#[derive(Debug, Default, Deserialize)]
pub struct SmtpAccessUpdate {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub requires_encryption: Option<bool>,
    #[serde(default)]
    pub allowed_mechanisms: Option<Vec<SmtpAuthenticationMechanism>>,
}

/// The verdict on SMTP AUTH credentials, including per-access rules.
pub enum SmtpAuthentication {
    /// Valid credentials allowed on this session.
    Authenticated(SmtpPrincipal),
    /// Unknown, wrong, or disabled credentials.
    Invalid,
    /// Valid credentials of an access that only accepts TLS-protected sessions.
    EncryptionRequired,
    /// Valid credentials presented with a mechanism the access does not allow.
    MechanismNotAllowed,
}

/// The global SMTP server: its configured address and runtime listener status.
///
/// `listener` is absent when no runtime owns the socket (for example, in offline tools).
#[derive(Clone, Debug, Serialize)]
pub struct SmtpServerDetails {
    pub listen_address: Option<SocketAddr>,
    pub listener: Option<MailListenerStatus>,
}

/// Newly issued SMTP credentials: the only representation that ever carries the password.
///
/// Intentionally lacks Debug so the password is never formatted into logs.
#[derive(Serialize)]
pub struct IssuedSmtpCredentials {
    /// The access as stored; it never includes the password or its hash.
    pub access: SmtpAccess,
    pub password: String,
    pub server: SmtpServerDetails,
}

/// The identity bound to an authenticated SMTP session.
///
/// It retains the hash verified at authentication and how the session authenticated, so
/// regenerating the password, disabling the access, or tightening its rules stops deliveries of
/// sessions that authenticated earlier. Intentionally lacks Debug.
#[derive(Clone)]
pub struct SmtpPrincipal {
    access_identifier: SmtpAccessIdentifier,
    verified_password_hash: String,
    mechanism: SmtpAuthenticationMechanism,
    encrypted: bool,
}

impl SmtpAuthentication {
    /// Return the principal of a successful authentication, or `None` for any refusal.
    pub fn principal(self) -> Option<SmtpPrincipal> {
        match self {
            Self::Authenticated(principal) => Some(principal),
            Self::Invalid | Self::EncryptionRequired | Self::MechanismNotAllowed => None,
        }
    }
}

impl SmtpPrincipal {
    /// Return the SMTP access the session authenticated as.
    pub fn access_identifier(&self) -> SmtpAccessIdentifier {
        self.access_identifier
    }
}

/// Verifies SMTP AUTH credentials against stored access hashes for the SMTP transport.
///
/// It is independent of the SMTP protocol: the runtime adapts it to the mail server's handler.
#[derive(Clone)]
pub struct SmtpAuthenticator {
    storage: Arc<dyn Storage>,
    verifications: Arc<Semaphore>,
}

impl SmtpAuthenticator {
    /// Create an authenticator over the shared storage with bounded concurrent verification.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            verifications: Arc::new(Semaphore::new(MAXIMUM_CONCURRENT_VERIFICATIONS)),
        }
    }

    /// Verify a username and password and apply the access's rules for this session.
    ///
    /// `mechanism` is how the credentials were presented and `encrypted` whether the session is
    /// protected by TLS. Rules are checked only after the password verifies, so they never reveal
    /// whether a username exists. Unknown usernames are verified against a fixed dummy hash so
    /// response timing does not reveal which usernames exist either. A successful authentication
    /// records the access's last use.
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
        mechanism: SmtpAuthenticationMechanism,
        encrypted: bool,
    ) -> Result<SmtpAuthentication, ApplicationError> {
        if username.len() > MAXIMUM_CREDENTIAL_LENGTH || password.len() > MAXIMUM_CREDENTIAL_LENGTH
        {
            return Ok(SmtpAuthentication::Invalid);
        }
        let credential = self
            .storage
            .get_smtp_access_credential_by_username(username)
            .await?;
        let stored_hash = match &credential {
            Some(credential) => credential.password_hash.clone(),
            None => dummy_password_hash()?,
        };
        let verified = {
            let _permit =
                self.verifications.acquire().await.map_err(|_| {
                    ApplicationError::Unavailable("SMTP authentication closed".into())
                })?;
            let password = password.to_owned();
            let hash = stored_hash.clone();
            tokio::task::spawn_blocking(move || instance::verify_password(&password, &hash)).await?
        };
        let Some(credential) =
            credential.filter(|credential| verified && credential.access.enabled)
        else {
            return Ok(SmtpAuthentication::Invalid);
        };
        if credential.access.requires_encryption && !encrypted {
            return Ok(SmtpAuthentication::EncryptionRequired);
        }
        if !credential.access.allowed_mechanisms.contains(&mechanism) {
            return Ok(SmtpAuthentication::MechanismNotAllowed);
        }
        if let Err(error) = self
            .storage
            .record_smtp_access_use(credential.access.identifier, current_unix_seconds())
            .await
        {
            tracing::warn!(access = %credential.access.identifier, %error, "SMTP access use was not recorded");
        }
        Ok(SmtpAuthentication::Authenticated(SmtpPrincipal {
            access_identifier: credential.access.identifier,
            verified_password_hash: stored_hash,
            mechanism,
            encrypted,
        }))
    }

    /// Report whether an authenticated session may still deliver mail.
    ///
    /// Returns `false` when the access was deleted, disabled, had its password regenerated, or
    /// now forbids how the session authenticated (in clear text, or with that mechanism), so such
    /// mail is refused instead of captured.
    pub async fn may_deliver(&self, principal: &SmtpPrincipal) -> Result<bool, ApplicationError> {
        Ok(self
            .storage
            .get_smtp_access_credential(principal.access_identifier)
            .await?
            .is_some_and(|credential| {
                credential.access.enabled
                    && credential.password_hash == principal.verified_password_hash
                    && (principal.encrypted || !credential.access.requires_encryption)
                    && credential
                        .access
                        .allowed_mechanisms
                        .contains(&principal.mechanism)
            }))
    }
}

impl AuthorizedApplication {
    /// Describe the global SMTP server and its runtime listener to a global administrator.
    pub async fn smtp_server(&self) -> Result<SmtpServerDetails, ApplicationError> {
        self.require_global_admin().await?;
        self.smtp_server_details().await
    }

    /// List every SMTP access without credential material.
    pub async fn list_smtp_accesses(&self) -> Result<Vec<SmtpAccess>, ApplicationError> {
        self.require_global_admin().await?;
        Ok(self.application.storage.list_smtp_accesses().await?)
    }

    /// Create an SMTP access with a generated password returned only in this response.
    pub async fn create_smtp_access(
        &self,
        request: NewSmtpAccessRequest,
    ) -> Result<IssuedSmtpCredentials, ApplicationError> {
        self.require_global_admin().await?;
        let storage = &self.application.storage;
        let name = crate::validated_name(&request.name)?;
        let explicit_username = request
            .username
            .map(|username| validated_username(&username))
            .transpose()?;
        let allowed_mechanisms = validated_mechanisms(request.allowed_mechanisms)?;
        let password = generate_password()?;
        let password_hash = hash_generated_password(&password).await?;
        let mut attempts = 0;
        let access = loop {
            attempts += 1;
            let username = match &explicit_username {
                Some(username) => username.clone(),
                None => generated_username(&name, "smtp")?,
            };
            match storage
                .create_smtp_access(&NewSmtpAccess {
                    name: name.clone(),
                    username,
                    password_hash: password_hash.clone(),
                    created_at: current_unix_seconds(),
                    requires_encryption: request.requires_encryption,
                    allowed_mechanisms: allowed_mechanisms.clone(),
                })
                .await
            {
                Ok(access) => break access,
                Err(StorageError::DuplicateSmtpUsername)
                    if explicit_username.is_none() && attempts < GENERATED_USERNAME_ATTEMPTS => {}
                Err(StorageError::DuplicateSmtpUsername) => return Err(ApplicationError::Conflict),
                Err(error) => return Err(error.into()),
            }
        };
        tracing::info!(access = %access.identifier, username = %access.username, "SMTP access created");
        Ok(IssuedSmtpCredentials {
            access,
            password,
            server: self.smtp_server_details().await?,
        })
    }

    /// Replace an access's password with a new generated one returned only in this response.
    ///
    /// The previous password stops authenticating as soon as the replacement commits, and
    /// sessions authenticated with it can no longer deliver mail.
    pub async fn regenerate_smtp_access_password(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<IssuedSmtpCredentials, ApplicationError> {
        self.require_global_admin().await?;
        let storage = &self.application.storage;
        let password = generate_password()?;
        let password_hash = hash_generated_password(&password).await?;
        let access = storage
            .replace_smtp_access_password(identifier, &password_hash)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        tracing::info!(access = %access.identifier, "SMTP access password regenerated");
        Ok(IssuedSmtpCredentials {
            access,
            password,
            server: self.smtp_server_details().await?,
        })
    }

    /// Change an access's status or authentication rules; omitted fields keep their value.
    ///
    /// Disabled accesses neither authenticate nor deliver. Tightened rules apply to the next
    /// authentication and to deliveries of sessions that no longer satisfy them.
    pub async fn update_smtp_access(
        &self,
        identifier: SmtpAccessIdentifier,
        update: SmtpAccessUpdate,
    ) -> Result<SmtpAccess, ApplicationError> {
        self.require_global_admin().await?;
        let storage = &self.application.storage;
        let mechanisms = update
            .allowed_mechanisms
            .map(validated_mechanisms)
            .transpose()?;
        let mut access = storage
            .get_smtp_access(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        if let Some(enabled) = update.enabled {
            access = storage
                .set_smtp_access_enabled(identifier, enabled)
                .await?
                .ok_or(ApplicationError::NotFound)?;
        }
        if update.requires_encryption.is_some() || mechanisms.is_some() {
            let requires_encryption = update
                .requires_encryption
                .unwrap_or(access.requires_encryption);
            let mechanisms = mechanisms.unwrap_or_else(|| access.allowed_mechanisms.clone());
            access = storage
                .set_smtp_access_authentication_rules(identifier, requires_encryption, &mechanisms)
                .await?
                .ok_or(ApplicationError::NotFound)?;
        }
        tracing::info!(
            access = %access.identifier,
            enabled = access.enabled,
            requires_encryption = access.requires_encryption,
            mechanisms = ?access.allowed_mechanisms,
            "SMTP access updated"
        );
        Ok(access)
    }

    /// Permanently delete an access; its credentials stop working immediately.
    pub async fn delete_smtp_access(
        &self,
        identifier: SmtpAccessIdentifier,
    ) -> Result<(), ApplicationError> {
        self.require_global_admin().await?;
        if !self
            .application
            .storage
            .delete_smtp_access(identifier)
            .await?
        {
            return Err(ApplicationError::NotFound);
        }
        tracing::info!(access = %identifier, "SMTP access deleted");
        Ok(())
    }

    /// Read the configured server address and runtime listener status, when a runtime is attached.
    async fn smtp_server_details(&self) -> Result<SmtpServerDetails, ApplicationError> {
        let listener = match &self.application.mail_listeners {
            Some(control) => Some(control.status().await),
            None => None,
        };
        Ok(SmtpServerDetails {
            listen_address: listener
                .as_ref()
                .and_then(|status| status.configured_address),
            listener,
        })
    }
}

/// Sort and deduplicate allowed mechanisms, refusing an empty set.
fn validated_mechanisms(
    mut mechanisms: Vec<SmtpAuthenticationMechanism>,
) -> Result<Vec<SmtpAuthenticationMechanism>, ApplicationError> {
    mechanisms.sort();
    mechanisms.dedup();
    if mechanisms.is_empty() {
        return Err(ApplicationError::InvalidInput(
            "allow at least one SMTP AUTH mechanism".into(),
        ));
    }
    Ok(mechanisms)
}
