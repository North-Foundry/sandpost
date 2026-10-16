//! Authenticate SMTP sessions and persist their mail before acknowledgement and notification.
//!
//! This adapter connects the protocol-level `SessionHandler` to the application's SMTP access
//! authenticator and storage. All captured mail joins one pool; authenticated sessions are
//! revalidated before each delivery, and unauthenticated mail is accepted only where the
//! listener's policy allows it.
use sandpost_application::{MessageEvent, SmtpAuthentication, SmtpAuthenticator, SmtpPrincipal};
use sandpost_core::Message;
use sandpost_smtp::{
    AuthenticationError, AuthenticationOutcome, Credentials, DeliveryError, SessionHandler,
};
use sandpost_storage::Storage;
use std::{future::Future, sync::Arc};
use tokio::sync::broadcast;

/// Shared state every SMTP session uses to authenticate and capture mail.
#[derive(Clone)]
pub(crate) struct MailDelivery {
    pub authenticator: SmtpAuthenticator,
    pub storage: Arc<dyn Storage>,
    pub events: broadcast::Sender<MessageEvent>,
}

impl SessionHandler for MailDelivery {
    type Principal = SmtpPrincipal;

    /// Verify credentials against the SMTP accesses and their rules, never logging the password.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<SmtpPrincipal>, AuthenticationError>> + Send
    {
        let authenticator = self.authenticator.clone();
        async move {
            let authentication = authenticator
                .authenticate(
                    &credentials.username,
                    &credentials.password,
                    credentials.mechanism,
                    credentials.encrypted,
                )
                .await
                .map_err(|error| AuthenticationError::Unavailable(error.to_string()))?;
            let username = &credentials.username;
            Ok(match authentication {
                SmtpAuthentication::Authenticated(principal) => {
                    tracing::debug!(access = %principal.access_identifier(), "SMTP session authenticated");
                    AuthenticationOutcome::Authenticated(principal)
                }
                SmtpAuthentication::Invalid => {
                    tracing::info!(%username, "SMTP authentication failed");
                    AuthenticationOutcome::InvalidCredentials
                }
                SmtpAuthentication::EncryptionRequired => {
                    tracing::info!(%username, "SMTP access refused: it requires an encrypted connection");
                    AuthenticationOutcome::EncryptionRequired
                }
                SmtpAuthentication::MechanismNotAllowed => {
                    tracing::info!(%username, mechanism = ?credentials.mechanism, "SMTP access refused: mechanism not allowed");
                    AuthenticationOutcome::MechanismNotAllowed
                }
            })
        }
    }

    /// Refuse mail from revoked credentials, then commit the message and notify subscribers.
    fn deliver(
        &self,
        message: Message,
        principal: Option<SmtpPrincipal>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        let delivery = self.clone();
        async move {
            if let Some(principal) = principal
                && !delivery
                    .authenticator
                    .may_deliver(&principal)
                    .await
                    .map_err(|error| DeliveryError::Temporary(error.to_string()))?
            {
                return Err(DeliveryError::Rejected(
                    "SMTP access credentials are no longer valid".into(),
                ));
            }
            capture_message(message, delivery.storage, delivery.events)
                .await
                .map_err(DeliveryError::Temporary)
        }
    }
}

/// Commit one message and its transactional index outbox operation.
pub(crate) async fn capture_message(
    message: Message,
    storage: Arc<dyn Storage>,
    event_broadcast: broadcast::Sender<MessageEvent>,
) -> Result<(), String> {
    let identifier = message.identifier;
    let sequence = storage
        .insert_message(&message)
        .await
        .map_err(|error| error.to_string())?;
    let _ = event_broadcast.send(MessageEvent {
        identifier,
        sequence,
    });
    Ok(())
}
