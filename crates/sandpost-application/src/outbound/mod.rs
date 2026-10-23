//! External delivery of captured mail: SMTP Outbounds, scope routing actions and deliveries.
//!
//! Sandpost is a mail sandbox, so external delivery is opt-in and Owner-only. Nothing is
//! delivered until an Owner creates an SMTP Outbound, permits recipients on it and attaches an
//! enabled routing action to a scope. Every management operation rereads the caller's role;
//! delivery history is additionally bounded by the reader's own mail visibility, so it never
//! reveals a message the Owner cannot read.
//!
//! The transport (`sandpost-smtp-outbound`) only knows how to reach a server; routing policy
//! (which scope's actions apply and to which recipients) lives in [`routing`], and the durable
//! dispatcher in [`runtime`].
mod routing;
mod runtime;
mod secrets;
mod validation;

#[cfg(test)]
mod tests;

pub(crate) use runtime::OutboundRuntime;
pub use secrets::OutboundSecretKey;
pub use validation::{
    MAXIMUM_ALLOWED_RECIPIENT_PATTERNS, MAXIMUM_FORWARD_RECIPIENTS, RoutingActionRequest,
    SmtpOutboundRequest, SmtpOutboundTestRequest,
};

use crate::credentials::current_unix_seconds;
use crate::{Application, ApplicationError, AuthorizedApplication, QueryOptions, authorization};
use runtime::{DEFAULT_CONNECT_TIMEOUT, DEFAULT_SMTP_TIMEOUT};
use sandpost_core::{
    MessageIdentifier, OutboundDelivery, OutboundDeliveryAttempt, OutboundDeliveryIdentifier,
    OutboundDeliveryStatus, OutboundFailure, OutboundFailureKind, RoutingAction,
    RoutingActionIdentifier, ScopeIdentifier, SmtpOutbound, SmtpOutboundAuthentication,
    SmtpOutboundIdentifier,
};
use sandpost_smtp_outbound::{
    DEFAULT_HELO_NAME, OutboundConnection, OutboundCredentials, test_connection,
};
use sandpost_storage::{
    NewRoutingAction, NewSmtpOutbound, OutboundDeliveryQuery, OutboundDeliveryReader,
    RoutingActionUpdate, SmtpOutboundPassword, StorageError,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use validation::{validated_forward_recipients, validated_outbound};

/// Limits, retry policy, network policy and credential key of outbound delivery.
#[derive(Debug, Clone)]
pub struct OutboundSettings {
    /// Permit SMTP Outbounds on loopback, private, link-local and other non-public addresses.
    pub allow_internal_addresses: bool,
    /// Delivery attempts running at once across every SMTP Outbound.
    pub concurrent_deliveries: usize,
    /// Attempts before transiently failing recipients are given up.
    pub maximum_attempts: u32,
    /// Delay before the second attempt; it doubles for every later one.
    pub retry_delay: Duration,
    /// Longest delay between two attempts.
    pub maximum_retry_delay: Duration,
    /// Fallback interval at which the dispatcher looks for queued messages and due deliveries.
    pub poll_interval: Duration,
    /// The key sealing SMTP Outbound passwords; without one, passwords cannot be stored or used.
    pub secret_key: Option<OutboundSecretKey>,
}

impl Default for OutboundSettings {
    /**
     * Refuse internal destinations, run four attempts at once, and retry eight times starting one
     * minute apart, doubling up to one hour. No key is configured.
     */
    fn default() -> Self {
        Self {
            allow_internal_addresses: false,
            concurrent_deliveries: 4,
            maximum_attempts: 8,
            retry_delay: Duration::from_secs(60),
            maximum_retry_delay: Duration::from_secs(60 * 60),
            poll_interval: Duration::from_secs(1),
            secret_key: None,
        }
    }
}

/// The instance policy shown next to SMTP Outbound administration.
#[derive(Debug, Clone, Serialize)]
pub struct OutboundPolicy {
    pub allow_internal_addresses: bool,
    pub maximum_attempts: u32,
    pub retry_delay_seconds: u64,
    /// Whether a secret key is available, so passwords can be stored and used.
    pub credentials_available: bool,
}

/// An SMTP Outbound with how many routing actions use it.
#[derive(Debug, Clone, Serialize)]
pub struct SmtpOutboundDetails {
    #[serde(flatten)]
    pub outbound: SmtpOutbound,
    pub routing_action_count: u64,
}

/// The result of a connection test.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionTestResult {
    pub success: bool,
    /// The server address reached, as `address:port`.
    pub address: Option<String>,
    pub encrypted: bool,
    pub authenticated: bool,
    pub extensions: Vec<String>,
    pub duration_milliseconds: u64,
    pub error: Option<OutboundFailure>,
}

/// A delivery with its attempt history.
#[derive(Debug, Clone, Serialize)]
pub struct OutboundDeliveryDetails {
    #[serde(flatten)]
    pub delivery: OutboundDelivery,
    pub attempts: Vec<OutboundDeliveryAttempt>,
}

/// A request to enable or disable an SMTP Outbound.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmtpOutboundEnabledRequest {
    pub enabled: bool,
}

/// A page of deliveries, newest first.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OutboundDeliveryListRequest {
    #[serde(default)]
    pub status: Option<OutboundDeliveryStatus>,
    /// Exclusive `seq` cursor.
    #[serde(default)]
    pub before: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl Application {
    /**
     * Replace the settings used by outbound routing and delivery.
     */
    pub fn with_outbound_settings(mut self, settings: OutboundSettings) -> Self {
        self.outbound = Arc::new(OutboundRuntime::new(settings));
        self
    }

    /**
     * Route queued messages and deliver due jobs until `shutdown` turns true.
     *
     * Hosts run this once per installation alongside their listeners, outside transport
     * supervision. Attempts interrupted by shutdown are retried when the dispatcher runs again.
     */
    pub async fn run_outbound(&self, shutdown: watch::Receiver<bool>) {
        self.outbound.clone().run(self.clone(), shutdown).await;
    }

    /**
     * Return the settings of outbound delivery.
     */
    pub fn outbound_settings(&self) -> &OutboundSettings {
        &self.outbound.settings
    }
}

/**
 * Translate storage conflicts of SMTP Outbound operations into application errors.
 */
fn outbound_storage_error(error: StorageError) -> ApplicationError {
    match error {
        StorageError::DuplicateSmtpOutboundName => {
            ApplicationError::InUse("an SMTP Outbound with this name already exists".into())
        }
        StorageError::SmtpOutboundInUse(count) => ApplicationError::InUse(format!(
            "the SMTP Outbound is used by {count} routing action(s); delete or change them first"
        )),
        StorageError::NotFound => ApplicationError::NotFound,
        error => ApplicationError::Storage(error),
    }
}

/**
 * Return the editable settings of a stored SMTP Outbound.
 */
fn settings_of(outbound: SmtpOutbound) -> sandpost_storage::SmtpOutboundSettings {
    sandpost_storage::SmtpOutboundSettings {
        name: outbound.name,
        host: outbound.host,
        port: outbound.port,
        security: outbound.security,
        authentication: outbound.authentication,
        username: outbound.username,
        enabled: outbound.enabled,
        override_from: outbound.override_from,
        envelope_from: outbound.envelope_from,
        connect_timeout_seconds: outbound.connect_timeout_seconds,
        smtp_timeout_seconds: outbound.smtp_timeout_seconds,
        verify_certificates: outbound.verify_certificates,
        helo_name: outbound.helo_name,
        maximum_connections: outbound.maximum_connections,
        allowed_recipients: outbound.allowed_recipients,
    }
}

/**
 * Describe how to reach the server of an SMTP Outbound, applying the default `EHLO` name and
 * timeouts where the settings leave them unset.
 */
pub(crate) fn outbound_connection(
    settings: &sandpost_storage::SmtpOutboundSettings,
    credentials: Option<OutboundCredentials>,
) -> OutboundConnection {
    let seconds = |value: Option<u32>, default: Duration| {
        value.map_or(default, |seconds| Duration::from_secs(u64::from(seconds)))
    };
    OutboundConnection {
        host: settings.host.clone(),
        port: settings.port,
        security: settings.security,
        credentials,
        verify_certificates: settings.verify_certificates,
        helo_name: settings
            .helo_name
            .clone()
            .unwrap_or_else(|| DEFAULT_HELO_NAME.to_owned()),
        connect_timeout: seconds(settings.connect_timeout_seconds, DEFAULT_CONNECT_TIMEOUT),
        command_timeout: seconds(settings.smtp_timeout_seconds, DEFAULT_SMTP_TIMEOUT),
    }
}

/**
 * Convert a transport connection report into its client representation.
 */
fn test_result(report: sandpost_smtp_outbound::ConnectionReport) -> ConnectionTestResult {
    ConnectionTestResult {
        success: report.success,
        address: report.address.map(|address| address.to_string()),
        encrypted: report.encrypted,
        authenticated: report.authenticated,
        extensions: report.extensions,
        duration_milliseconds: u64::try_from(report.duration.as_millis()).unwrap_or(u64::MAX),
        error: report.error,
    }
}

impl AuthorizedApplication {
    /**
     * Describe the instance's outbound delivery policy; Owners only.
     */
    pub async fn outbound_policy(&self) -> Result<OutboundPolicy, ApplicationError> {
        self.require_owner().await?;
        let settings = self.application.outbound_settings();
        Ok(OutboundPolicy {
            allow_internal_addresses: settings.allow_internal_addresses,
            maximum_attempts: settings.maximum_attempts,
            retry_delay_seconds: settings.retry_delay.as_secs(),
            credentials_available: settings.secret_key.is_some(),
        })
    }

    /**
     * List every SMTP Outbound without passwords; Owners only.
     */
    pub async fn list_smtp_outbounds(&self) -> Result<Vec<SmtpOutboundDetails>, ApplicationError> {
        self.require_owner().await?;
        let storage = &self.application.storage;
        let actions = storage.list_routing_actions(None).await?;
        Ok(storage
            .list_smtp_outbounds()
            .await?
            .into_iter()
            .map(|outbound| {
                let routing_action_count = actions
                    .iter()
                    .filter(|action| action.smtp_outbound_identifier == outbound.identifier)
                    .count() as u64;
                SmtpOutboundDetails {
                    outbound,
                    routing_action_count,
                }
            })
            .collect())
    }

    /**
     * Read one SMTP Outbound without its password; Owners only.
     */
    pub async fn get_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<SmtpOutboundDetails, ApplicationError> {
        self.require_owner().await?;
        let outbound = self
            .application
            .storage
            .get_smtp_outbound(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        self.describe_outbound(outbound).await
    }

    /**
     * Create an SMTP Outbound, disabled unless the request enables it; Owners only.
     *
     * The password is encrypted before it reaches storage and never returned.
     */
    pub async fn create_smtp_outbound(
        &self,
        request: SmtpOutboundRequest,
    ) -> Result<SmtpOutboundDetails, ApplicationError> {
        self.require_owner().await?;
        let (settings, password) = validated_outbound(request, false, false)?;
        let identifier = SmtpOutboundIdentifier::new();
        let encrypted_password = password
            .map(|password| self.seal_password(identifier, &password))
            .transpose()?;
        let outbound = self
            .application
            .storage
            .create_smtp_outbound(&NewSmtpOutbound {
                identifier,
                settings,
                encrypted_password,
                created_at: current_unix_seconds(),
            })
            .await
            .map_err(outbound_storage_error)?;
        self.application.outbound.wake();
        self.describe_outbound(outbound).await
    }

    /**
     * Replace an SMTP Outbound's settings; Owners only.
     *
     * An absent password keeps the stored one without revealing it; switching to no
     * authentication removes it. Disabling the outbound fails its waiting deliveries.
     */
    pub async fn update_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
        request: SmtpOutboundRequest,
    ) -> Result<SmtpOutboundDetails, ApplicationError> {
        self.require_owner().await?;
        let storage = &self.application.storage;
        let previous = storage
            .get_smtp_outbound(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        let (settings, password) =
            validated_outbound(request, previous.password_configured, previous.enabled)?;
        let password = match password {
            Some(password) => {
                SmtpOutboundPassword::Replace(self.seal_password(identifier, &password)?)
            }
            None if settings.authentication == SmtpOutboundAuthentication::None => {
                SmtpOutboundPassword::Clear
            }
            None => SmtpOutboundPassword::Keep,
        };
        let outbound = storage
            .update_smtp_outbound(identifier, &settings, &password, current_unix_seconds())
            .await
            .map_err(outbound_storage_error)?
            .ok_or(ApplicationError::NotFound)?;
        self.application.outbound.wake();
        self.describe_outbound(outbound).await
    }

    /**
     * Enable or disable an SMTP Outbound; Owners only. Disabling fails its waiting deliveries.
     */
    pub async fn set_smtp_outbound_enabled(
        &self,
        identifier: SmtpOutboundIdentifier,
        enabled: bool,
    ) -> Result<SmtpOutboundDetails, ApplicationError> {
        self.require_owner().await?;
        let storage = &self.application.storage;
        let previous = storage
            .get_smtp_outbound(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        let settings = sandpost_storage::SmtpOutboundSettings {
            enabled,
            ..settings_of(previous)
        };
        let outbound = storage
            .update_smtp_outbound(
                identifier,
                &settings,
                &SmtpOutboundPassword::Keep,
                current_unix_seconds(),
            )
            .await
            .map_err(outbound_storage_error)?
            .ok_or(ApplicationError::NotFound)?;
        self.application.outbound.wake();
        self.describe_outbound(outbound).await
    }

    /**
     * Delete an SMTP Outbound that no routing action references; Owners only. Its waiting
     * deliveries fail and delivery history keeps the outbound's name.
     */
    pub async fn delete_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<(), ApplicationError> {
        self.require_owner().await?;
        if !self
            .application
            .storage
            .delete_smtp_outbound(identifier)
            .await
            .map_err(outbound_storage_error)?
        {
            return Err(ApplicationError::NotFound);
        }
        Ok(())
    }

    /**
     * Connect, negotiate TLS and authenticate with a saved SMTP Outbound without sending mail;
     * Owners only. A disabled outbound can be tested.
     */
    pub async fn test_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<ConnectionTestResult, ApplicationError> {
        self.require_owner().await?;
        let credential = self
            .application
            .storage
            .get_smtp_outbound_credential(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        let outbound = credential.outbound;
        let password = match outbound.authentication {
            SmtpOutboundAuthentication::None => None,
            _ => Some(self.open_password(identifier, credential.encrypted_password.as_deref())?),
        };
        let settings = settings_of(outbound);
        Ok(self.run_connection_test(&settings, password).await)
    }

    /**
     * Test an unsaved SMTP Outbound definition without sending mail; Owners only. When the
     * request names a saved outbound and supplies no password, the stored password is used.
     */
    pub async fn test_smtp_outbound_draft(
        &self,
        request: SmtpOutboundTestRequest,
    ) -> Result<ConnectionTestResult, ApplicationError> {
        self.require_owner().await?;
        let stored = match request.identifier {
            Some(identifier) => Some(
                self.application
                    .storage
                    .get_smtp_outbound_credential(identifier)
                    .await?
                    .ok_or(ApplicationError::NotFound)?,
            ),
            None => None,
        };
        let password_stored = stored
            .as_ref()
            .is_some_and(|credential| credential.encrypted_password.is_some());
        let (settings, password) = validated_outbound(request.outbound, password_stored, true)?;
        let password = match (password, &stored) {
            (Some(password), _) => Some(password),
            (None, _) if settings.authentication == SmtpOutboundAuthentication::None => None,
            (None, Some(credential)) => Some(self.open_password(
                credential.outbound.identifier,
                credential.encrypted_password.as_deref(),
            )?),
            (None, None) => None,
        };
        Ok(self.run_connection_test(&settings, password).await)
    }

    /**
     * List a scope's routing actions; Owners only.
     */
    pub async fn list_routing_actions(
        &self,
        scope: ScopeIdentifier,
    ) -> Result<Vec<RoutingAction>, ApplicationError> {
        self.require_owner().await?;
        let storage = &self.application.storage;
        storage
            .get_scope(scope)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        Ok(storage.list_routing_actions(Some(scope)).await?)
    }

    /**
     * Attach a relay or forward action to a scope; Owners only.
     *
     * Forward recipients must be permitted by the referenced SMTP Outbound's allowlist.
     */
    pub async fn create_routing_action(
        &self,
        scope: ScopeIdentifier,
        request: RoutingActionRequest,
    ) -> Result<RoutingAction, ApplicationError> {
        self.require_owner().await?;
        let storage = &self.application.storage;
        storage
            .get_scope(scope)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        let outbound = self
            .request_outbound(request.smtp_outbound_identifier)
            .await?;
        let forward_recipients = validated_forward_recipients(
            request.kind,
            request.forward_recipients,
            &outbound.allowed_recipients,
        )?;
        let action = storage
            .create_routing_action(&NewRoutingAction {
                identifier: RoutingActionIdentifier::new(),
                scope_identifier: scope,
                smtp_outbound_identifier: outbound.identifier,
                kind: request.kind,
                forward_recipients,
                enabled: request.enabled,
                created_at: current_unix_seconds(),
            })
            .await
            .map_err(outbound_storage_error)?;
        self.application.outbound.wake();
        Ok(action)
    }

    /**
     * Replace a routing action's type, SMTP Outbound, recipients and enabled state; Owners only.
     * Disabling it or changing its type or outbound fails its waiting deliveries.
     */
    pub async fn update_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
        request: RoutingActionRequest,
    ) -> Result<RoutingAction, ApplicationError> {
        self.require_owner().await?;
        let outbound = self
            .request_outbound(request.smtp_outbound_identifier)
            .await?;
        let forward_recipients = validated_forward_recipients(
            request.kind,
            request.forward_recipients,
            &outbound.allowed_recipients,
        )?;
        let action = self
            .application
            .storage
            .update_routing_action(
                identifier,
                &RoutingActionUpdate {
                    smtp_outbound_identifier: outbound.identifier,
                    kind: request.kind,
                    forward_recipients,
                    enabled: request.enabled,
                },
                current_unix_seconds(),
            )
            .await
            .map_err(outbound_storage_error)?
            .ok_or(ApplicationError::NotFound)?;
        self.application.outbound.wake();
        Ok(action)
    }

    /**
     * Delete a routing action; Owners only. Its waiting deliveries fail.
     */
    pub async fn delete_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
    ) -> Result<(), ApplicationError> {
        self.require_owner().await?;
        if !self
            .application
            .storage
            .delete_routing_action(identifier)
            .await?
        {
            return Err(ApplicationError::NotFound);
        }
        Ok(())
    }

    /**
     * List deliveries newest first, limited to messages the Owner can read; Owners only.
     */
    pub async fn list_outbound_deliveries(
        &self,
        request: OutboundDeliveryListRequest,
    ) -> Result<Vec<OutboundDelivery>, ApplicationError> {
        self.require_owner().await?;
        let limit = request.limit.unwrap_or(50);
        crate::validate_page_size(limit)?;
        self.read_deliveries(None, request.status, request.before, limit)
            .await
    }

    /**
     * List the deliveries of one message the Owner can read; Owners only. Other messages are
     * not found, so delivery history never confirms that a message exists.
     */
    pub async fn list_message_deliveries(
        &self,
        message: MessageIdentifier,
    ) -> Result<Vec<OutboundDelivery>, ApplicationError> {
        self.require_owner().await?;
        if !self.is_message_visible(message).await? {
            return Err(ApplicationError::NotFound);
        }
        self.read_deliveries(Some(message), None, None, 100).await
    }

    /**
     * Read one delivery with its attempts when its message is readable; Owners only.
     */
    pub async fn get_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
    ) -> Result<OutboundDeliveryDetails, ApplicationError> {
        self.require_owner().await?;
        let (delivery, attempts) = self
            .application
            .storage
            .get_outbound_delivery(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        if !self.is_message_visible(delivery.message_identifier).await? {
            return Err(ApplicationError::NotFound);
        }
        Ok(OutboundDeliveryDetails { delivery, attempts })
    }

    /**
     * Schedule a failed delivery again with its failed recipients; Owners only. Deliveries in any
     * other state are a conflict.
     */
    pub async fn retry_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
    ) -> Result<OutboundDelivery, ApplicationError> {
        let details = self.get_outbound_delivery(identifier).await?;
        if details.delivery.status != OutboundDeliveryStatus::Failed {
            return Err(ApplicationError::InUse(
                "only failed deliveries can be retried".into(),
            ));
        }
        let delivery = self
            .application
            .storage
            .retry_outbound_delivery(identifier, current_unix_seconds())
            .await
            .map_err(outbound_storage_error)?
            .ok_or_else(|| {
                ApplicationError::InUse("only failed deliveries can be retried".into())
            })?;
        self.application.outbound.wake();
        Ok(delivery)
    }

    /**
     * Read deliveries bounded by the caller's composed authorization and deletions.
     */
    async fn read_deliveries(
        &self,
        message: Option<MessageIdentifier>,
        status: Option<OutboundDeliveryStatus>,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<OutboundDelivery>, ApplicationError> {
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        Ok(self
            .application
            .storage
            .list_outbound_deliveries(&OutboundDeliveryQuery {
                message,
                status,
                before,
                limit,
                reader: Some(OutboundDeliveryReader {
                    user: self.user_identifier,
                    filter: authorization::storage_filter(&query),
                }),
            })
            .await?)
    }

    /**
     * Load the SMTP Outbound a routing action request references, as invalid input when unknown.
     */
    async fn request_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<SmtpOutbound, ApplicationError> {
        self.application
            .storage
            .get_smtp_outbound(identifier)
            .await?
            .ok_or_else(|| {
                ApplicationError::InvalidInput("the SMTP Outbound does not exist".into())
            })
    }

    /**
     * Attach the routing action count to an SMTP Outbound.
     */
    async fn describe_outbound(
        &self,
        outbound: SmtpOutbound,
    ) -> Result<SmtpOutboundDetails, ApplicationError> {
        let routing_action_count = self
            .application
            .storage
            .list_routing_actions(None)
            .await?
            .iter()
            .filter(|action| action.smtp_outbound_identifier == outbound.identifier)
            .count() as u64;
        Ok(SmtpOutboundDetails {
            outbound,
            routing_action_count,
        })
    }

    /**
     * Encrypt a password for an SMTP Outbound, refusing when no secret key is configured.
     */
    fn seal_password(
        &self,
        identifier: SmtpOutboundIdentifier,
        password: &str,
    ) -> Result<Vec<u8>, ApplicationError> {
        self.application
            .outbound_settings()
            .secret_key
            .as_ref()
            .ok_or_else(|| {
                ApplicationError::InvalidInput(
                    "passwords cannot be stored because no outbound secret key is configured"
                        .into(),
                )
            })?
            .seal(identifier, password)
    }

    /**
     * Decrypt a stored SMTP Outbound password for a connection test.
     */
    fn open_password(
        &self,
        identifier: SmtpOutboundIdentifier,
        sealed: Option<&[u8]>,
    ) -> Result<String, ApplicationError> {
        let key = self
            .application
            .outbound_settings()
            .secret_key
            .as_ref()
            .ok_or_else(|| {
                ApplicationError::InvalidInput(
                    "the stored password cannot be used because no outbound secret key is configured"
                        .into(),
                )
            })?;
        sealed
            .and_then(|sealed| key.open(identifier, sealed))
            .ok_or_else(|| {
                ApplicationError::InvalidInput(
                    "the stored password cannot be decrypted with the current secret key; enter it again"
                        .into(),
                )
            })
    }

    /**
     * Run a connection test with validated settings and an optional clear-text password.
     */
    async fn run_connection_test(
        &self,
        settings: &sandpost_storage::SmtpOutboundSettings,
        password: Option<String>,
    ) -> ConnectionTestResult {
        let credentials = password.map(|password| OutboundCredentials {
            mechanism: settings.authentication,
            username: settings.username.clone().unwrap_or_default(),
            password,
        });
        let connection = outbound_connection(settings, credentials);
        let report =
            test_connection(&connection, &self.application.outbound.destination_policy()).await;
        if let Some(error) = &report.error
            && error.kind == OutboundFailureKind::Policy
        {
            tracing::info!(host = %settings.host, "SMTP Outbound connection test refused by the destination policy");
        }
        test_result(report)
    }
}
