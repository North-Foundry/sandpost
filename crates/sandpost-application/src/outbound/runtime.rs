//! Durable, bounded background routing and delivery of captured mail.
//!
//! Message insertion queues a routing evaluation in the same storage transaction as the message,
//! so SMTP never waits for an external server and no stored message is forgotten. The dispatcher
//! wakes on message notifications, configuration changes, finished attempts and a periodic poll.
//! It first turns queued messages into delivery jobs, then claims due jobs while fewer than the
//! configured number of attempts run, never opening more connections to one SMTP Outbound than
//! it allows. Every attempt is an independent task: its failure, panic or slowness affects no
//! other delivery and no transport.
//!
//! A job claimed by an attempt is recorded as `processing` before connecting. If the process
//! stops, the next dispatcher start moves such jobs back to `retrying`: the external server may
//! already have accepted the message, so the retry can deliver a duplicate. SMTP offers no way to
//! rule that out, so delivery is at least once for transient failures, never exactly once.
use super::OutboundSettings;
use super::routing::RoutingConfiguration;
use crate::credentials::current_unix_seconds;
use crate::{Application, ApplicationError};
use sandpost_core::{
    OutboundAttemptOutcome, OutboundDelivery, OutboundDeliveryStatus, OutboundFailure,
    OutboundFailureKind, OutboundRecipientStatus, SmtpOutboundAuthentication,
    SmtpOutboundIdentifier,
};
use sandpost_smtp_outbound::{
    DestinationPolicy, OutboundConnection, OutboundCredentials, OutboundEnvelope, RecipientOutcome,
    deliver, prepare_message,
};
use sandpost_storage::{OutboundAttemptResult, OutboundRecipientUpdate};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore, broadcast, watch};
use tokio::task::JoinSet;

/// Default bound on connecting, TLS negotiation, the greeting and `EHLO`.
pub(crate) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default bound on each SMTP command after connecting.
pub(crate) const DEFAULT_SMTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Default simultaneous connections to one SMTP Outbound.
const DEFAULT_MAXIMUM_CONNECTIONS: u32 = 2;

/// Queued messages evaluated per dispatcher pass.
const EVALUATION_BATCH: usize = 64;

/// Everything one attempt needs to contact the SMTP server.
struct PreparedAttempt {
    connection: OutboundConnection,
    envelope: OutboundEnvelope,
    message: Vec<u8>,
    outbound: SmtpOutboundIdentifier,
    maximum_connections: u32,
}

/// Background routing and delivery shared by every transport of one application.
pub(crate) struct OutboundRuntime {
    pub(crate) settings: OutboundSettings,
    active: Mutex<HashSet<sandpost_core::OutboundDeliveryIdentifier>>,
    connections: Mutex<HashMap<SmtpOutboundIdentifier, (u32, Arc<Semaphore>)>>,
    wake: Notify,
}

/**
 * Build a configuration failure.
 */
fn configuration_failure(message: impl Into<String>) -> OutboundFailure {
    OutboundFailure {
        kind: OutboundFailureKind::Configuration,
        message: message.into(),
        code: None,
    }
}

/**
 * Wait for the next message notification, or forever once the channel has closed.
 */
async fn next_event(
    events: &mut Option<broadcast::Receiver<crate::MessageEvent>>,
) -> Result<crate::MessageEvent, broadcast::error::RecvError> {
    match events {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

impl OutboundRuntime {
    /**
     * Create a runtime following `settings`.
     */
    pub(crate) fn new(settings: OutboundSettings) -> Self {
        Self {
            settings,
            active: Mutex::new(HashSet::new()),
            connections: Mutex::new(HashMap::new()),
            wake: Notify::new(),
        }
    }

    /**
     * Ask the dispatcher to evaluate queued messages and start due deliveries now.
     */
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    /**
     * Return the destination policy of outbound connections.
     */
    pub(crate) fn destination_policy(&self) -> DestinationPolicy {
        DestinationPolicy {
            allow_internal_addresses: self.settings.allow_internal_addresses,
        }
    }

    /**
     * Return how many delivery attempts this process is running.
     */
    pub(crate) fn active_count(&self) -> usize {
        self.active.lock().map(|active| active.len()).unwrap_or(0)
    }

    /**
     * Return the semaphore bounding connections to one SMTP Outbound, resized when its limit
     * changed.
     */
    fn connection_slots(&self, outbound: SmtpOutboundIdentifier, maximum: u32) -> Arc<Semaphore> {
        let mut connections = self
            .connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match connections.get(&outbound) {
            Some((limit, semaphore)) if *limit == maximum => semaphore.clone(),
            _ => {
                let semaphore = Arc::new(Semaphore::new(maximum.max(1) as usize));
                connections.insert(outbound, (maximum, semaphore.clone()));
                semaphore
            }
        }
    }

    /**
     * Route and deliver until `shutdown` turns true.
     *
     * Deliveries left `processing` by a previous process become `retrying` first. Shutdown stops
     * running attempts; their jobs stay `processing` and are retried after the next start.
     */
    pub(crate) async fn run(
        self: Arc<Self>,
        application: Application,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let storage = application.storage.clone();
        match storage
            .interrupt_outbound_deliveries(current_unix_seconds())
            .await
        {
            Ok(0) => {}
            Ok(interrupted) => {
                tracing::warn!(
                    interrupted,
                    "outbound deliveries interrupted by a previous stop will be retried; a server may receive a duplicate"
                )
            }
            Err(error) => {
                tracing::warn!(%error, "interrupted outbound deliveries could not be recovered")
            }
        }
        let mut events = Some(application.subscribe());
        let mut tasks = JoinSet::new();
        tracing::debug!("outbound dispatcher started");
        while !*shutdown.borrow() {
            if let Err(error) = self.evaluate(&application).await {
                tracing::warn!(%error, "outbound routing could not evaluate queued messages");
            }
            if let Err(error) = self.fill(&application, &mut tasks).await {
                tracing::warn!(%error, "outbound dispatcher could not read due deliveries");
            }
            tokio::select! {
                () = self.wake.notified() => {}
                received = next_event(&mut events) => {
                    if matches!(received, Err(broadcast::error::RecvError::Closed)) {
                        events = None;
                    }
                }
                () = tokio::time::sleep(self.settings.poll_interval) => {}
                Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                    if let Err(error) = joined
                        && error.is_panic()
                    {
                        tracing::error!(%error, "outbound delivery task panicked");
                    }
                }
                _ = shutdown.changed() => {}
            }
        }
        tasks.shutdown().await;
        if let Ok(mut active) = self.active.lock() {
            active.clear();
        }
        tracing::debug!("outbound dispatcher stopped");
    }

    /**
     * Turn queued messages into delivery jobs, one batch per pass, until the queue is empty.
     */
    pub(crate) async fn evaluate(
        &self,
        application: &Application,
    ) -> Result<usize, ApplicationError> {
        let storage = &application.storage;
        let mut evaluated = 0;
        loop {
            let queued = storage.list_routing_evaluations(EVALUATION_BATCH).await?;
            if queued.is_empty() {
                return Ok(evaluated);
            }
            let configuration = RoutingConfiguration::load(storage).await?;
            let mut created = 0;
            for message in &queued {
                let deliveries = if configuration.is_empty() {
                    Vec::new()
                } else {
                    match storage.get_message_metadata(*message).await? {
                        Some((facts, _)) => {
                            configuration.deliveries(&facts, current_unix_seconds())
                        }
                        None => Vec::new(),
                    }
                };
                created += storage
                    .complete_routing_evaluation(*message, &deliveries)
                    .await?;
            }
            evaluated += queued.len();
            if created > 0 {
                tracing::debug!(created, "outbound deliveries created");
            }
            if queued.len() < EVALUATION_BATCH {
                return Ok(evaluated);
            }
        }
    }

    /**
     * Claim due deliveries while capacity remains and start an attempt task for each.
     */
    pub(crate) async fn fill(
        self: &Arc<Self>,
        application: &Application,
        tasks: &mut JoinSet<()>,
    ) -> Result<(), ApplicationError> {
        let maximum = self.settings.concurrent_deliveries.max(1);
        let active = self.active_count();
        if active >= maximum {
            return Ok(());
        }
        let now = current_unix_seconds();
        let due = application
            .storage
            .list_due_outbound_deliveries(now, (maximum - active) * 2)
            .await?;
        for delivery in due {
            if self.active_count() >= maximum {
                break;
            }
            let Some(claimed) = application
                .storage
                .claim_outbound_delivery(delivery.identifier, now)
                .await?
            else {
                continue;
            };
            if let Ok(mut active) = self.active.lock() {
                active.insert(claimed.identifier);
            }
            let runtime = self.clone();
            let application = application.clone();
            tasks.spawn(async move {
                let identifier = claimed.identifier;
                if let Err(error) = runtime.attempt(&application, claimed).await {
                    tracing::warn!(delivery = %identifier, %error, "outbound delivery attempt could not be recorded; it is retried after the next start");
                }
                if let Ok(mut active) = runtime.active.lock() {
                    active.remove(&identifier);
                }
                runtime.wake();
            });
        }
        Ok(())
    }

    /**
     * Run one claimed attempt and record its outcome.
     *
     * Returns an error only when storage failed; the job then stays `processing` until the next
     * start retries it.
     */
    pub(crate) async fn attempt(
        &self,
        application: &Application,
        delivery: OutboundDelivery,
    ) -> Result<(), ApplicationError> {
        let identifier = delivery.identifier;
        let result = match self.prepare(application, &delivery).await? {
            Ok(PreparedAttempt {
                connection,
                envelope,
                message,
                outbound,
                maximum_connections,
            }) => {
                let slots = self.connection_slots(outbound, maximum_connections);
                let _permit = slots.acquire_owned().await.map_err(|_| {
                    ApplicationError::Unavailable("outbound connections closed".into())
                })?;
                let report =
                    deliver(&connection, &self.destination_policy(), &envelope, &message).await;
                self.outcome(&delivery, envelope.sender, report.recipients, report.error)
            }
            Err(failure) => {
                let refused: Vec<_> = delivery
                    .recipients
                    .iter()
                    .filter(|recipient| recipient.status == OutboundRecipientStatus::Pending)
                    .map(|recipient| {
                        (
                            recipient.address.clone(),
                            RecipientOutcome::Failed(failure.clone()),
                        )
                    })
                    .collect();
                self.outcome(&delivery, None, refused, Some(failure))
            }
        };
        match result.status {
            OutboundDeliveryStatus::Delivered => {
                tracing::info!(delivery = %identifier, outbound = %delivery.smtp_outbound_name, "outbound delivery delivered")
            }
            OutboundDeliveryStatus::Failed => {
                tracing::warn!(delivery = %identifier, outbound = %delivery.smtp_outbound_name, error = ?result.error.as_ref().map(|error| &error.message), "outbound delivery failed")
            }
            _ => {
                tracing::info!(delivery = %identifier, outbound = %delivery.smtp_outbound_name, next_attempt_at = ?result.next_attempt_at, "outbound delivery deferred")
            }
        }
        application
            .storage
            .finish_outbound_delivery_attempt(identifier, &result)
            .await?;
        Ok(())
    }

    /**
     * Gather what an attempt needs: the current routing action, SMTP Outbound and password,
     * the message, and the recipients still permitted. A configuration or policy problem is
     * returned as the failure of every pending recipient.
     */
    async fn prepare(
        &self,
        application: &Application,
        delivery: &OutboundDelivery,
    ) -> Result<Result<PreparedAttempt, OutboundFailure>, ApplicationError> {
        let storage = &application.storage;
        let Some(action_identifier) = delivery.routing_action_identifier else {
            return Ok(Err(configuration_failure("the routing action was deleted")));
        };
        let Some(action) = storage.get_routing_action(action_identifier).await? else {
            return Ok(Err(configuration_failure("the routing action was deleted")));
        };
        if !action.enabled {
            return Ok(Err(configuration_failure("the routing action is disabled")));
        }
        let Some(outbound_identifier) = delivery.smtp_outbound_identifier else {
            return Ok(Err(configuration_failure("the SMTP Outbound was deleted")));
        };
        let Some(credential) = storage
            .get_smtp_outbound_credential(outbound_identifier)
            .await?
        else {
            return Ok(Err(configuration_failure("the SMTP Outbound was deleted")));
        };
        let outbound = credential.outbound;
        if !outbound.enabled {
            return Ok(Err(configuration_failure("the SMTP Outbound is disabled")));
        }
        let credentials = match outbound.authentication {
            SmtpOutboundAuthentication::None => None,
            mechanism => {
                let Some(key) = &self.settings.secret_key else {
                    return Ok(Err(configuration_failure(
                        "no secret key is available to decrypt the SMTP Outbound password",
                    )));
                };
                let Some(password) = credential
                    .encrypted_password
                    .as_deref()
                    .and_then(|sealed| key.open(outbound.identifier, sealed))
                else {
                    return Ok(Err(configuration_failure(
                        "the SMTP Outbound password cannot be decrypted with the current secret key; enter it again",
                    )));
                };
                Some(OutboundCredentials {
                    mechanism,
                    username: outbound.username.clone().unwrap_or_default(),
                    password,
                })
            }
        };
        let Some(message) = storage.get_message(delivery.message_identifier).await? else {
            return Ok(Err(configuration_failure("the message no longer exists")));
        };
        let recipients: Vec<String> = delivery
            .recipients
            .iter()
            .filter(|recipient| recipient.status == OutboundRecipientStatus::Pending)
            .map(|recipient| recipient.address.clone())
            .collect();
        let sender = outbound.envelope_from.clone().or_else(|| {
            message
                .facts
                .envelope_from
                .as_ref()
                .map(|mailbox| mailbox.address.clone())
        });
        let override_from = outbound.override_from.clone();
        let raw = message.raw_message;
        let prepared =
            tokio::task::spawn_blocking(move || prepare_message(&raw, override_from.as_deref()))
                .await?;
        let maximum_connections = outbound
            .maximum_connections
            .unwrap_or(DEFAULT_MAXIMUM_CONNECTIONS);
        let outbound_identifier = outbound.identifier;
        let allowed_recipients = outbound.allowed_recipients.clone();
        let connection = super::outbound_connection(&super::settings_of(outbound), credentials);
        // The allowlist may have been narrowed since the job was created.
        let (permitted, refused): (Vec<String>, Vec<String>) =
            recipients.into_iter().partition(|address| {
                super::validation::recipient_allowed(address, &allowed_recipients)
            });
        if permitted.is_empty() {
            return Ok(Err(OutboundFailure {
                kind: OutboundFailureKind::Policy,
                message: if refused.is_empty() {
                    "no recipient is left to deliver to".into()
                } else {
                    "the SMTP Outbound's recipient allowlist permits none of the remaining recipients".into()
                },
                code: None,
            }));
        }
        Ok(Ok(PreparedAttempt {
            connection,
            envelope: OutboundEnvelope {
                sender,
                recipients: permitted,
            },
            message: prepared,
            outbound: outbound_identifier,
            maximum_connections,
        }))
    }

    /**
     * Turn per-recipient outcomes into the attempt result and the job's next state.
     *
     * Deferred recipients stay pending for another attempt with exponential backoff, unless this
     * was the last allowed attempt, which fails them. Pending recipients that this attempt did not
     * include (refused by a narrowed allowlist) fail with a policy failure. The job is delivered
     * when every recipient is delivered, retrying while any is pending, and failed otherwise.
     */
    fn outcome(
        &self,
        delivery: &OutboundDelivery,
        sender: Option<String>,
        outcomes: Vec<(String, RecipientOutcome)>,
        session_error: Option<OutboundFailure>,
    ) -> OutboundAttemptResult {
        let now = current_unix_seconds();
        let last_attempt = delivery.attempt_count >= self.settings.maximum_attempts;
        let mut outcomes: HashMap<String, RecipientOutcome> = outcomes.into_iter().collect();
        let mut updates = Vec::new();
        for recipient in &delivery.recipients {
            if recipient.status != OutboundRecipientStatus::Pending {
                continue;
            }
            let outcome = outcomes.remove(&recipient.address).unwrap_or_else(|| {
                RecipientOutcome::Failed(OutboundFailure {
                    kind: OutboundFailureKind::Policy,
                    message:
                        "the SMTP Outbound's recipient allowlist no longer permits this recipient"
                            .into(),
                    code: None,
                })
            });
            updates.push(match outcome {
                RecipientOutcome::Delivered { response } => OutboundRecipientUpdate {
                    address: recipient.address.clone(),
                    status: OutboundRecipientStatus::Delivered,
                    error: None,
                    response: Some(response),
                    delivered_at: Some(now),
                },
                RecipientOutcome::Failed(failure) => OutboundRecipientUpdate {
                    address: recipient.address.clone(),
                    status: OutboundRecipientStatus::Failed,
                    error: Some(failure),
                    response: None,
                    delivered_at: None,
                },
                RecipientOutcome::Deferred(mut failure) => {
                    if last_attempt {
                        failure.message = format!(
                            "{} (gave up after {} attempts)",
                            failure.message, delivery.attempt_count
                        );
                    }
                    OutboundRecipientUpdate {
                        address: recipient.address.clone(),
                        status: if last_attempt {
                            OutboundRecipientStatus::Failed
                        } else {
                            OutboundRecipientStatus::Pending
                        },
                        error: Some(failure),
                        response: None,
                        delivered_at: None,
                    }
                }
            });
        }
        let delivered_now = updates
            .iter()
            .filter(|update| update.status == OutboundRecipientStatus::Delivered)
            .count();
        let pending_now = updates
            .iter()
            .filter(|update| update.status == OutboundRecipientStatus::Pending)
            .count();
        let outcome = if delivered_now == updates.len() && !updates.is_empty() {
            OutboundAttemptOutcome::Delivered
        } else if delivered_now > 0 {
            OutboundAttemptOutcome::Partial
        } else if pending_now > 0 {
            OutboundAttemptOutcome::Deferred
        } else {
            OutboundAttemptOutcome::Failed
        };
        let previously_failed = delivery
            .recipients
            .iter()
            .any(|recipient| recipient.status == OutboundRecipientStatus::Failed);
        let failed_now = updates
            .iter()
            .any(|update| update.status == OutboundRecipientStatus::Failed);
        let status = if pending_now > 0 {
            OutboundDeliveryStatus::Retrying
        } else if previously_failed || failed_now {
            OutboundDeliveryStatus::Failed
        } else {
            OutboundDeliveryStatus::Delivered
        };
        let error = session_error.or_else(|| {
            updates
                .iter()
                .find(|update| update.status != OutboundRecipientStatus::Delivered)
                .and_then(|update| update.error.clone())
        });
        OutboundAttemptResult {
            finished_at: now,
            envelope_from: sender,
            outcome,
            error: if status == OutboundDeliveryStatus::Delivered {
                None
            } else {
                error
            },
            recipients: updates,
            status,
            next_attempt_at: (status == OutboundDeliveryStatus::Retrying)
                .then(|| now + self.backoff(delivery.attempt_count)),
        }
    }

    /**
     * Return the delay in seconds before the attempt that follows attempt number `attempt`:
     * the retry delay doubled for every earlier attempt, capped at the maximum delay.
     */
    pub(crate) fn backoff(&self, attempt: u32) -> i64 {
        let base = self.settings.retry_delay.as_secs().max(1);
        let exponent = attempt.saturating_sub(1).min(20);
        let delay = base
            .saturating_mul(1_u64 << exponent)
            .min(self.settings.maximum_retry_delay.as_secs().max(base));
        i64::try_from(delay).unwrap_or(i64::MAX)
    }
}
