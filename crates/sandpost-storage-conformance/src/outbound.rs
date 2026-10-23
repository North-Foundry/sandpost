//! SMTP Outbound, routing action, routing queue and outbound delivery lifecycle checks.
use sandpost_core::{
    MessageIdentifier, OutboundAttemptOutcome, OutboundDeliveryIdentifier, OutboundDeliveryStatus,
    OutboundFailure, OutboundFailureKind, OutboundRecipientStatus, RoutingAction,
    RoutingActionIdentifier, RoutingActionKind, ScopeIdentifier, SmtpOutbound,
    SmtpOutboundAuthentication, SmtpOutboundIdentifier, SmtpOutboundSecurity, UserIdentifier,
};
use sandpost_query::Expression;
use sandpost_storage::{
    NewOutboundDelivery, NewOutboundRecipient, NewRoutingAction, NewSmtpOutbound,
    OutboundAttemptResult, OutboundDeliveryQuery, OutboundDeliveryReader, OutboundRecipientUpdate,
    RoutingActionUpdate, SmtpOutboundPassword, SmtpOutboundSettings, Storage, StorageError,
};

use crate::ConformanceFailure;
use crate::support::{
    failure, mailbox, save_scope, simple_message, unique_token, unwrap_storage, verify,
    verify_equal,
};

/**
 * Build enabled SMTP Outbound settings with a unique name.
 */
fn settings(prefix: &str) -> SmtpOutboundSettings {
    SmtpOutboundSettings {
        name: format!("{prefix} {} {}", std::process::id(), unique_token()),
        host: "smtp.example.test".into(),
        port: 587,
        security: SmtpOutboundSecurity::StartTls,
        authentication: SmtpOutboundAuthentication::Plain,
        username: Some("relay-user".into()),
        enabled: true,
        override_from: None,
        envelope_from: Some("bounces@example.test".into()),
        connect_timeout_seconds: Some(5),
        smtp_timeout_seconds: None,
        verify_certificates: true,
        helo_name: None,
        maximum_connections: Some(2),
        allowed_recipients: vec!["*@example.test".into()],
    }
}

/**
 * Create an SMTP Outbound with the supplied settings and encrypted password bytes.
 */
async fn create_outbound(
    storage: &dyn Storage,
    check: &'static str,
    settings: SmtpOutboundSettings,
    encrypted_password: Option<Vec<u8>>,
) -> Result<SmtpOutbound, ConformanceFailure> {
    unwrap_storage(
        check,
        "create_smtp_outbound",
        storage
            .create_smtp_outbound(&NewSmtpOutbound {
                identifier: SmtpOutboundIdentifier::new(),
                settings,
                encrypted_password,
                created_at: 100,
            })
            .await,
    )
}

/**
 * Create a routing action of a scope.
 */
async fn create_action(
    storage: &dyn Storage,
    check: &'static str,
    scope: ScopeIdentifier,
    outbound: SmtpOutboundIdentifier,
    kind: RoutingActionKind,
    enabled: bool,
) -> Result<RoutingAction, ConformanceFailure> {
    unwrap_storage(
        check,
        "create_routing_action",
        storage
            .create_routing_action(&NewRoutingAction {
                identifier: RoutingActionIdentifier::new(),
                scope_identifier: scope,
                smtp_outbound_identifier: outbound,
                kind,
                forward_recipients: match kind {
                    RoutingActionKind::Relay => Vec::new(),
                    RoutingActionKind::Forward => vec!["qa@example.test".into()],
                },
                enabled,
                created_at: 200,
            })
            .await,
    )
}

/**
 * Store a message addressed to the supplied envelope recipients and return its identifier.
 */
async fn deliver(
    storage: &dyn Storage,
    check: &'static str,
    recipients: &[&str],
) -> Result<MessageIdentifier, ConformanceFailure> {
    let mut message = simple_message("outbound delivery");
    message.facts.envelope_to = recipients.iter().map(|address| mailbox(address)).collect();
    message.facts.received_at = 300;
    unwrap_storage(
        check,
        "insert_message",
        storage.insert_message(&message).await,
    )?;
    Ok(message.identifier)
}

/**
 * Remove every queued routing evaluation without creating deliveries.
 */
async fn drain_queue(
    storage: &dyn Storage,
    check: &'static str,
) -> Result<Vec<MessageIdentifier>, ConformanceFailure> {
    let queued = unwrap_storage(
        check,
        "list_routing_evaluations",
        storage.list_routing_evaluations(1000).await,
    )?;
    for message in &queued {
        unwrap_storage(
            check,
            "complete_routing_evaluation",
            storage.complete_routing_evaluation(*message, &[]).await,
        )?;
    }
    Ok(queued)
}

/**
 * Build a pending delivery of an action to the supplied recipients.
 */
fn new_delivery(
    action: &RoutingAction,
    outbound: &SmtpOutbound,
    recipients: &[&str],
) -> NewOutboundDelivery {
    NewOutboundDelivery {
        identifier: OutboundDeliveryIdentifier::new(),
        scope_identifier: action.scope_identifier,
        routing_action_identifier: action.identifier,
        smtp_outbound_identifier: outbound.identifier,
        smtp_outbound_name: outbound.name.clone(),
        kind: action.kind,
        status: OutboundDeliveryStatus::Pending,
        last_error: None,
        recipients: recipients
            .iter()
            .map(|address| NewOutboundRecipient {
                address: (*address).into(),
                status: OutboundRecipientStatus::Pending,
                error: None,
            })
            .collect(),
        created_at: 400,
    }
}

/**
 * Build a failure of the given kind.
 */
fn failure_of(kind: OutboundFailureKind, message: &str, code: Option<u16>) -> OutboundFailure {
    OutboundFailure {
        kind,
        message: message.into(),
        code,
    }
}

/// SMTP Outbounds are created with unique names, never return passwords from ordinary reads,
/// replace or clear passwords on update, and cannot be deleted while referenced.
pub async fn smtp_outbound_definitions(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "smtp_outbound_definitions";
    let first_settings = settings("Pippo");
    let first =
        create_outbound(storage, CHECK, first_settings.clone(), Some(vec![1, 2, 3])).await?;
    verify(
        CHECK,
        first.password_configured,
        "a stored password is reported",
    )?;
    verify_equal(
        CHECK,
        "stored name",
        first.name.as_str(),
        first_settings.name.as_str(),
    )?;
    verify_equal(
        CHECK,
        "allowed recipients",
        first.allowed_recipients.clone(),
        vec!["*@example.test".to_owned()],
    )?;
    let mut duplicate = settings("other");
    duplicate.name = first_settings.name.to_uppercase();
    match storage
        .create_smtp_outbound(&NewSmtpOutbound {
            identifier: SmtpOutboundIdentifier::new(),
            settings: duplicate,
            encrypted_password: None,
            created_at: 100,
        })
        .await
    {
        Err(StorageError::DuplicateSmtpOutboundName) => {}
        other => {
            return Err(failure(
                CHECK,
                format!("a name differing only in case must conflict, got {other:?}"),
            ));
        }
    }
    let credential = unwrap_storage(
        CHECK,
        "get_smtp_outbound_credential",
        storage.get_smtp_outbound_credential(first.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "the credential read finds the outbound"))?;
    verify_equal(
        CHECK,
        "encrypted password",
        credential.encrypted_password.clone(),
        Some(vec![1, 2, 3]),
    )?;
    verify_equal(CHECK, "credential outbound", &credential.outbound, &first)?;

    let mut changed = first_settings.clone();
    changed.host = "relay.example.test".into();
    let kept = unwrap_storage(
        CHECK,
        "update_smtp_outbound keep",
        storage
            .update_smtp_outbound(first.identifier, &changed, &SmtpOutboundPassword::Keep, 150)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "update finds the outbound"))?;
    verify_equal(
        CHECK,
        "updated host",
        kept.host.as_str(),
        "relay.example.test",
    )?;
    verify_equal(CHECK, "updated time", kept.updated_at, 150)?;
    let replaced = unwrap_storage(
        CHECK,
        "get_smtp_outbound_credential after keep",
        storage.get_smtp_outbound_credential(first.identifier).await,
    )?
    .and_then(|credential| credential.encrypted_password);
    verify_equal(CHECK, "kept password", replaced, Some(vec![1, 2, 3]))?;
    unwrap_storage(
        CHECK,
        "update_smtp_outbound replace",
        storage
            .update_smtp_outbound(
                first.identifier,
                &changed,
                &SmtpOutboundPassword::Replace(vec![9]),
                160,
            )
            .await,
    )?;
    let replaced = unwrap_storage(
        CHECK,
        "get_smtp_outbound_credential after replace",
        storage.get_smtp_outbound_credential(first.identifier).await,
    )?
    .and_then(|credential| credential.encrypted_password);
    verify_equal(CHECK, "replaced password", replaced, Some(vec![9]))?;
    changed.authentication = SmtpOutboundAuthentication::None;
    changed.username = None;
    let cleared = unwrap_storage(
        CHECK,
        "update_smtp_outbound clear",
        storage
            .update_smtp_outbound(
                first.identifier,
                &changed,
                &SmtpOutboundPassword::Clear,
                170,
            )
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "update finds the outbound"))?;
    verify(
        CHECK,
        !cleared.password_configured,
        "the password was cleared",
    )?;
    let missing = unwrap_storage(
        CHECK,
        "update_smtp_outbound unknown",
        storage
            .update_smtp_outbound(
                SmtpOutboundIdentifier::new(),
                &changed,
                &SmtpOutboundPassword::Keep,
                170,
            )
            .await,
    )?;
    verify(
        CHECK,
        missing.is_none(),
        "updating an unknown outbound returns none",
    )?;

    let scope = save_scope(storage, CHECK, "outbound references").await?;
    let action = create_action(
        storage,
        CHECK,
        scope,
        first.identifier,
        RoutingActionKind::Relay,
        false,
    )
    .await?;
    match storage.delete_smtp_outbound(first.identifier).await {
        Err(StorageError::SmtpOutboundInUse(1)) => {}
        other => {
            return Err(failure(
                CHECK,
                format!("a referenced outbound cannot be deleted, got {other:?}"),
            ));
        }
    }
    unwrap_storage(
        CHECK,
        "delete_routing_action",
        storage.delete_routing_action(action.identifier).await,
    )?;
    let deleted = unwrap_storage(
        CHECK,
        "delete_smtp_outbound",
        storage.delete_smtp_outbound(first.identifier).await,
    )?;
    verify(CHECK, deleted, "an unreferenced outbound is deleted")?;
    let deleted_again = unwrap_storage(
        CHECK,
        "delete_smtp_outbound again",
        storage.delete_smtp_outbound(first.identifier).await,
    )?;
    verify(
        CHECK,
        !deleted_again,
        "deleting twice reports a missing outbound",
    )?;
    unwrap_storage(CHECK, "delete_scope", storage.delete_scope(scope).await)?;
    Ok(())
}

/// Routing actions require their scope and outbound, several may share a scope and an outbound,
/// they disappear with their scope, and message insertion queues a routing evaluation only while
/// an enabled action uses an enabled outbound.
pub async fn routing_actions_and_queue(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "routing_actions_and_queue";
    drain_queue(storage, CHECK).await?;
    let unrouted = deliver(storage, CHECK, &["nobody@example.test"]).await?;
    let queued = drain_queue(storage, CHECK).await?;
    verify(
        CHECK,
        !queued.contains(&unrouted),
        "mail stored without routing actions is not queued",
    )?;

    let outbound = create_outbound(storage, CHECK, settings("Pluto"), None).await?;
    let first_scope = save_scope(storage, CHECK, "routing first").await?;
    let second_scope = save_scope(storage, CHECK, "routing second").await?;
    match storage
        .create_routing_action(&NewRoutingAction {
            identifier: RoutingActionIdentifier::new(),
            scope_identifier: ScopeIdentifier::new(),
            smtp_outbound_identifier: outbound.identifier,
            kind: RoutingActionKind::Relay,
            forward_recipients: Vec::new(),
            enabled: true,
            created_at: 1,
        })
        .await
    {
        Err(StorageError::NotFound) => {}
        other => {
            return Err(failure(
                CHECK,
                format!("an unknown scope is not found, got {other:?}"),
            ));
        }
    }
    match storage
        .create_routing_action(&NewRoutingAction {
            identifier: RoutingActionIdentifier::new(),
            scope_identifier: first_scope,
            smtp_outbound_identifier: SmtpOutboundIdentifier::new(),
            kind: RoutingActionKind::Relay,
            forward_recipients: Vec::new(),
            enabled: true,
            created_at: 1,
        })
        .await
    {
        Err(StorageError::NotFound) => {}
        other => {
            return Err(failure(
                CHECK,
                format!("an unknown outbound is not found, got {other:?}"),
            ));
        }
    }

    let disabled = create_action(
        storage,
        CHECK,
        first_scope,
        outbound.identifier,
        RoutingActionKind::Forward,
        false,
    )
    .await?;
    let message = deliver(storage, CHECK, &["someone@example.test"]).await?;
    let queued = drain_queue(storage, CHECK).await?;
    verify(
        CHECK,
        !queued.contains(&message),
        "a disabled action queues nothing",
    )?;

    let forward = create_action(
        storage,
        CHECK,
        first_scope,
        outbound.identifier,
        RoutingActionKind::Forward,
        true,
    )
    .await?;
    let relay = create_action(
        storage,
        CHECK,
        first_scope,
        outbound.identifier,
        RoutingActionKind::Relay,
        true,
    )
    .await?;
    let other_scope_action = create_action(
        storage,
        CHECK,
        second_scope,
        outbound.identifier,
        RoutingActionKind::Relay,
        true,
    )
    .await?;
    let listed = unwrap_storage(
        CHECK,
        "list_routing_actions",
        storage.list_routing_actions(Some(first_scope)).await,
    )?;
    verify_equal(
        CHECK,
        "actions of the first scope",
        listed
            .iter()
            .map(|action| action.identifier)
            .collect::<Vec<_>>(),
        vec![disabled.identifier, forward.identifier, relay.identifier],
    )?;
    let every = unwrap_storage(
        CHECK,
        "list_routing_actions all",
        storage.list_routing_actions(None).await,
    )?;
    verify(
        CHECK,
        every
            .iter()
            .any(|action| action.identifier == other_scope_action.identifier),
        "listing every action includes other scopes",
    )?;

    let message = deliver(storage, CHECK, &["someone@example.test"]).await?;
    let queued = drain_queue(storage, CHECK).await?;
    verify_equal(CHECK, "queued messages", queued, vec![message])?;

    let mut disabled_outbound = settings("disabled");
    disabled_outbound.name = outbound.name.clone();
    disabled_outbound.enabled = false;
    unwrap_storage(
        CHECK,
        "disable outbound",
        storage
            .update_smtp_outbound(
                outbound.identifier,
                &disabled_outbound,
                &SmtpOutboundPassword::Keep,
                500,
            )
            .await,
    )?;
    let message = deliver(storage, CHECK, &["someone@example.test"]).await?;
    let queued = drain_queue(storage, CHECK).await?;
    verify(
        CHECK,
        !queued.contains(&message),
        "a disabled outbound queues nothing",
    )?;

    let updated = unwrap_storage(
        CHECK,
        "update_routing_action",
        storage
            .update_routing_action(
                forward.identifier,
                &RoutingActionUpdate {
                    smtp_outbound_identifier: outbound.identifier,
                    kind: RoutingActionKind::Forward,
                    forward_recipients: vec!["audit@example.test".into()],
                    enabled: true,
                },
                600,
            )
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the action is updated"))?;
    verify_equal(
        CHECK,
        "updated recipients",
        updated.forward_recipients,
        vec!["audit@example.test".to_owned()],
    )?;
    verify_equal(
        CHECK,
        "update keeps the scope",
        updated.scope_identifier,
        first_scope,
    )?;

    unwrap_storage(
        CHECK,
        "delete_scope",
        storage.delete_scope(first_scope).await,
    )?;
    let remaining = unwrap_storage(
        CHECK,
        "list_routing_actions after scope deletion",
        storage.list_routing_actions(Some(first_scope)).await,
    )?;
    verify(
        CHECK,
        remaining.is_empty(),
        "scope deletion removes its actions",
    )?;
    unwrap_storage(
        CHECK,
        "delete_scope second",
        storage.delete_scope(second_scope).await,
    )?;
    let deleted = unwrap_storage(
        CHECK,
        "delete_smtp_outbound",
        storage.delete_smtp_outbound(outbound.identifier).await,
    )?;
    verify(
        CHECK,
        deleted,
        "the outbound is free once its scopes are gone",
    )?;
    Ok(())
}

/// Delivery jobs are created idempotently from the routing queue, claimed once, finish with
/// per-recipient outcomes, survive interruption, can be retried after failing, fail when their
/// outbound or action stops applying, and disappear with their message.
pub async fn outbound_delivery_lifecycle(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "outbound_delivery_lifecycle";
    drain_queue(storage, CHECK).await?;
    let outbound = create_outbound(storage, CHECK, settings("Lifecycle"), None).await?;
    let scope = save_scope(storage, CHECK, "delivery lifecycle").await?;
    let relay = create_action(
        storage,
        CHECK,
        scope,
        outbound.identifier,
        RoutingActionKind::Relay,
        true,
    )
    .await?;
    let forward = create_action(
        storage,
        CHECK,
        scope,
        outbound.identifier,
        RoutingActionKind::Forward,
        true,
    )
    .await?;
    let message = deliver(
        storage,
        CHECK,
        &["first@example.test", "second@example.test"],
    )
    .await?;
    let relay_delivery = new_delivery(
        &relay,
        &outbound,
        &["first@example.test", "second@example.test"],
    );
    let forward_delivery = new_delivery(&forward, &outbound, &["qa@example.test"]);
    let created = unwrap_storage(
        CHECK,
        "complete_routing_evaluation",
        storage
            .complete_routing_evaluation(
                message,
                &[relay_delivery.clone(), forward_delivery.clone()],
            )
            .await,
    )?;
    verify_equal(CHECK, "created deliveries", created, 2)?;
    let repeated = unwrap_storage(
        CHECK,
        "complete_routing_evaluation again",
        storage
            .complete_routing_evaluation(message, std::slice::from_ref(&relay_delivery))
            .await,
    )?;
    verify_equal(CHECK, "a dequeued message creates nothing", repeated, 0)?;

    let due = unwrap_storage(
        CHECK,
        "list_due_outbound_deliveries",
        storage.list_due_outbound_deliveries(400, 10).await,
    )?;
    verify_equal(
        CHECK,
        "due deliveries",
        due.iter()
            .map(|delivery| delivery.identifier)
            .collect::<Vec<_>>(),
        vec![relay_delivery.identifier, forward_delivery.identifier],
    )?;
    verify_equal(
        CHECK,
        "relay recipients",
        due[0]
            .recipients
            .iter()
            .map(|recipient| recipient.address.as_str())
            .collect::<Vec<_>>(),
        vec!["first@example.test", "second@example.test"],
    )?;
    let early = unwrap_storage(
        CHECK,
        "list_due_outbound_deliveries early",
        storage.list_due_outbound_deliveries(399, 10).await,
    )?;
    verify(
        CHECK,
        early.is_empty(),
        "deliveries are not due before creation",
    )?;

    let claimed = unwrap_storage(
        CHECK,
        "claim_outbound_delivery",
        storage
            .claim_outbound_delivery(relay_delivery.identifier, 410)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "a due delivery is claimed"))?;
    verify_equal(
        CHECK,
        "claimed status",
        claimed.status,
        OutboundDeliveryStatus::Processing,
    )?;
    verify_equal(CHECK, "claimed attempts", claimed.attempt_count, 1)?;
    let claimed_twice = unwrap_storage(
        CHECK,
        "claim_outbound_delivery twice",
        storage
            .claim_outbound_delivery(relay_delivery.identifier, 410)
            .await,
    )?;
    verify(
        CHECK,
        claimed_twice.is_none(),
        "a processing delivery is not claimed again",
    )?;

    let finished = unwrap_storage(
        CHECK,
        "finish_outbound_delivery_attempt partial",
        storage
            .finish_outbound_delivery_attempt(
                relay_delivery.identifier,
                &OutboundAttemptResult {
                    finished_at: 420,
                    envelope_from: Some("sender@example.test".into()),
                    outcome: OutboundAttemptOutcome::Partial,
                    error: None,
                    recipients: vec![
                        OutboundRecipientUpdate {
                            address: "first@example.test".into(),
                            status: OutboundRecipientStatus::Delivered,
                            error: None,
                            response: Some("250 queued".into()),
                            delivered_at: Some(420),
                        },
                        OutboundRecipientUpdate {
                            address: "second@example.test".into(),
                            status: OutboundRecipientStatus::Pending,
                            error: Some(failure_of(
                                OutboundFailureKind::Temporary,
                                "mailbox busy",
                                Some(450),
                            )),
                            response: None,
                            delivered_at: None,
                        },
                    ],
                    status: OutboundDeliveryStatus::Retrying,
                    next_attempt_at: Some(480),
                },
            )
            .await,
    )?;
    verify(
        CHECK,
        finished,
        "a processing delivery finishes its attempt",
    )?;
    let (stored, attempts) = unwrap_storage(
        CHECK,
        "get_outbound_delivery",
        storage
            .get_outbound_delivery(relay_delivery.identifier)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the delivery exists"))?;
    verify_equal(
        CHECK,
        "retrying status",
        stored.status,
        OutboundDeliveryStatus::Retrying,
    )?;
    verify_equal(CHECK, "next attempt", stored.next_attempt_at, Some(480))?;
    verify_equal(
        CHECK,
        "recipient states",
        stored
            .recipients
            .iter()
            .map(|recipient| recipient.status)
            .collect::<Vec<_>>(),
        vec![
            OutboundRecipientStatus::Delivered,
            OutboundRecipientStatus::Pending,
        ],
    )?;
    verify_equal(
        CHECK,
        "deferred recipient error code",
        stored.recipients[1]
            .error
            .as_ref()
            .and_then(|error| error.code),
        Some(450),
    )?;
    verify_equal(CHECK, "attempt history length", attempts.len(), 1)?;
    verify_equal(
        CHECK,
        "attempt outcome",
        attempts[0].outcome,
        Some(OutboundAttemptOutcome::Partial),
    )?;
    verify_equal(
        CHECK,
        "attempt counts",
        (
            attempts[0].delivered_recipients,
            attempts[0].deferred_recipients,
        ),
        (1, 1),
    )?;
    let not_due = unwrap_storage(
        CHECK,
        "claim before next attempt",
        storage
            .claim_outbound_delivery(relay_delivery.identifier, 470)
            .await,
    )?;
    verify(
        CHECK,
        not_due.is_none(),
        "a retrying delivery waits for its next attempt",
    )?;

    unwrap_storage(
        CHECK,
        "claim for interruption",
        storage
            .claim_outbound_delivery(relay_delivery.identifier, 480)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the retry is claimed"))?;
    let interrupted = unwrap_storage(
        CHECK,
        "interrupt_outbound_deliveries",
        storage.interrupt_outbound_deliveries(490).await,
    )?;
    verify_equal(CHECK, "interrupted deliveries", interrupted, 1)?;
    let (stored, attempts) = unwrap_storage(
        CHECK,
        "get_outbound_delivery after interruption",
        storage
            .get_outbound_delivery(relay_delivery.identifier)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the delivery exists"))?;
    verify_equal(
        CHECK,
        "interrupted status",
        stored.status,
        OutboundDeliveryStatus::Retrying,
    )?;
    verify_equal(
        CHECK,
        "interrupted attempt",
        attempts.last().and_then(|attempt| attempt.outcome),
        Some(OutboundAttemptOutcome::Interrupted),
    )?;

    unwrap_storage(
        CHECK,
        "claim after interruption",
        storage
            .claim_outbound_delivery(relay_delivery.identifier, 500)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the interrupted delivery is claimed again"))?;
    unwrap_storage(
        CHECK,
        "finish failed",
        storage
            .finish_outbound_delivery_attempt(
                relay_delivery.identifier,
                &OutboundAttemptResult {
                    finished_at: 510,
                    envelope_from: None,
                    outcome: OutboundAttemptOutcome::Failed,
                    error: None,
                    recipients: vec![OutboundRecipientUpdate {
                        address: "second@example.test".into(),
                        status: OutboundRecipientStatus::Failed,
                        error: Some(failure_of(
                            OutboundFailureKind::Permanent,
                            "no such user",
                            Some(550),
                        )),
                        response: None,
                        delivered_at: None,
                    }],
                    status: OutboundDeliveryStatus::Failed,
                    next_attempt_at: None,
                },
            )
            .await,
    )?;
    let not_retryable = unwrap_storage(
        CHECK,
        "retry_outbound_delivery pending",
        storage
            .retry_outbound_delivery(forward_delivery.identifier, 520)
            .await,
    )?;
    verify(
        CHECK,
        not_retryable.is_none(),
        "only failed deliveries are retried",
    )?;
    match storage
        .retry_outbound_delivery(OutboundDeliveryIdentifier::new(), 520)
        .await
    {
        Err(StorageError::NotFound) => {}
        other => {
            return Err(failure(
                CHECK,
                format!("retrying an unknown delivery is not found, got {other:?}"),
            ));
        }
    }
    let retried = unwrap_storage(
        CHECK,
        "retry_outbound_delivery",
        storage
            .retry_outbound_delivery(relay_delivery.identifier, 530)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "a failed delivery is retried"))?;
    verify_equal(
        CHECK,
        "retried status",
        retried.status,
        OutboundDeliveryStatus::Retrying,
    )?;
    verify_equal(CHECK, "retried attempt budget", retried.attempt_count, 0)?;
    verify_equal(
        CHECK,
        "retried recipients",
        retried
            .recipients
            .iter()
            .map(|recipient| recipient.status)
            .collect::<Vec<_>>(),
        vec![
            OutboundRecipientStatus::Delivered,
            OutboundRecipientStatus::Pending,
        ],
    )?;

    let by_message = unwrap_storage(
        CHECK,
        "list_outbound_deliveries by message",
        storage
            .list_outbound_deliveries(&OutboundDeliveryQuery {
                message: Some(message),
                status: None,
                before: None,
                limit: 10,
                reader: None,
            })
            .await,
    )?;
    verify_equal(
        CHECK,
        "deliveries of the message newest first",
        by_message
            .iter()
            .map(|delivery| delivery.identifier)
            .collect::<Vec<_>>(),
        vec![forward_delivery.identifier, relay_delivery.identifier],
    )?;
    verify_equal(
        CHECK,
        "delivery subject",
        by_message[0].subject.as_str(),
        "outbound delivery",
    )?;
    let older = unwrap_storage(
        CHECK,
        "list_outbound_deliveries before",
        storage
            .list_outbound_deliveries(&OutboundDeliveryQuery {
                message: Some(message),
                status: Some(OutboundDeliveryStatus::Retrying),
                before: Some(by_message[0].sequence),
                limit: 10,
                reader: None,
            })
            .await,
    )?;
    verify_equal(CHECK, "keyset and status filter", older.len(), 1)?;
    for (filter, expected) in [(Expression::True, 2), (Expression::False, 0)] {
        let readable = unwrap_storage(
            CHECK,
            "list_outbound_deliveries with a reader",
            storage
                .list_outbound_deliveries(&OutboundDeliveryQuery {
                    message: Some(message),
                    status: None,
                    before: None,
                    limit: 10,
                    reader: Some(OutboundDeliveryReader {
                        user: UserIdentifier::new(),
                        filter,
                    }),
                })
                .await,
        )?;
        verify_equal(
            CHECK,
            "deliveries visible to the reader",
            readable.len(),
            expected,
        )?;
    }

    let deleted = unwrap_storage(
        CHECK,
        "delete_routing_action forward",
        storage.delete_routing_action(forward.identifier).await,
    )?;
    verify(CHECK, deleted, "the forward action is deleted")?;
    let (orphaned, _) = unwrap_storage(
        CHECK,
        "get_outbound_delivery after action deletion",
        storage
            .get_outbound_delivery(forward_delivery.identifier)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the delivery history survives its action"))?;
    verify_equal(
        CHECK,
        "orphaned status",
        orphaned.status,
        OutboundDeliveryStatus::Failed,
    )?;
    verify(
        CHECK,
        orphaned.routing_action_identifier.is_none(),
        "the deleted action reference is cleared",
    )?;
    verify_equal(
        CHECK,
        "orphaned failure kind",
        orphaned.last_error.map(|error| error.kind),
        Some(OutboundFailureKind::Configuration),
    )?;
    verify_equal(
        CHECK,
        "recorded outbound name",
        orphaned.smtp_outbound_name,
        outbound.name.clone(),
    )?;

    let mut disabled = settings("disabled lifecycle");
    disabled.name = outbound.name.clone();
    disabled.enabled = false;
    unwrap_storage(
        CHECK,
        "disable outbound",
        storage
            .update_smtp_outbound(
                outbound.identifier,
                &disabled,
                &SmtpOutboundPassword::Keep,
                540,
            )
            .await,
    )?;
    let (stopped, _) = unwrap_storage(
        CHECK,
        "get_outbound_delivery after disabling",
        storage
            .get_outbound_delivery(relay_delivery.identifier)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the delivery exists"))?;
    verify_equal(
        CHECK,
        "disabled outbound fails waiting deliveries",
        stopped.status,
        OutboundDeliveryStatus::Failed,
    )?;
    verify_equal(
        CHECK,
        "pending recipient fails with the delivery",
        stopped.recipients[1].status,
        OutboundRecipientStatus::Failed,
    )?;

    unwrap_storage(
        CHECK,
        "delete_message",
        storage.delete_message(message).await,
    )?;
    let gone = unwrap_storage(
        CHECK,
        "get_outbound_delivery after message deletion",
        storage
            .get_outbound_delivery(relay_delivery.identifier)
            .await,
    )?;
    verify(
        CHECK,
        gone.is_none(),
        "message deletion removes its deliveries",
    )?;
    unwrap_storage(CHECK, "delete_scope", storage.delete_scope(scope).await)?;
    unwrap_storage(
        CHECK,
        "delete_smtp_outbound",
        storage.delete_smtp_outbound(outbound.identifier).await,
    )?;
    Ok(())
}
