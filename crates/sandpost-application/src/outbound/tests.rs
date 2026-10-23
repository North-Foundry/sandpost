//! SMTP Outbound management, routing policy, delivery, retry and recovery tests.
//!
//! Deliveries go to Sandpost's own SMTP capture server (`sandpost-smtp`) on a loopback port, so
//! the suite needs no external SMTP server, credentials or network access.
use super::{OutboundSecretKey, OutboundSettings, RoutingActionRequest, SmtpOutboundRequest};
use crate::{
    Application, ApplicationError, AuthorizedApplication, MessageEvent,
    OutboundDeliveryListRequest, SessionUser, SmtpOutboundTestRequest,
};
use sandpost_core::{
    GlobalRole, MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier,
    OutboundAttemptOutcome, OutboundDelivery, OutboundDeliveryStatus, OutboundFailureKind,
    OutboundRecipientStatus, RoutingActionKind, Scope, ScopeIdentifier, SmtpOutboundAuthentication,
    SmtpOutboundIdentifier, SmtpOutboundSecurity, User, UserIdentifier,
};
use sandpost_smtp::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    SessionHandler, SmtpServer, TransportSecurity,
};
use sandpost_storage::{NewUser, Storage};
use sandpost_storage_sqlite::SqliteStorage;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot, watch};

/// Capture handler accepting `relay` / `secret` and recording delivered messages.
struct RecordingHandler(Arc<Mutex<Vec<Message>>>);

impl SessionHandler for RecordingHandler {
    type Principal = ();

    /**
     * Accept only `relay` / `secret`.
     */
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<()>, AuthenticationError>> + Send {
        let valid = credentials.username == "relay" && credentials.password == "secret";
        std::future::ready(Ok(if valid {
            AuthenticationOutcome::Authenticated(())
        } else {
            AuthenticationOutcome::InvalidCredentials
        }))
    }

    /**
     * Record the delivered message.
     */
    fn deliver(
        &self,
        message: Message,
        _principal: Option<()>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        self.0.lock().unwrap().push(message);
        std::future::ready(Ok(()))
    }
}

/// A running external SMTP server stand-in.
struct Capture {
    address: SocketAddr,
    messages: Arc<Mutex<Vec<Message>>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Capture {
    /**
     * Start a plaintext capture server with the given AUTH policy on a loopback port.
     */
    async fn start(authentication: AuthenticationPolicy) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let messages = Arc::new(Mutex::new(Vec::new()));
        let (shutdown, receiver) = oneshot::channel::<()>();
        tokio::spawn(
            SmtpServer::new(RecordingHandler(messages.clone()))
                .authentication(authentication)
                .transport_security(TransportSecurity::Plaintext)
                .serve(listener, async {
                    let _ = receiver.await;
                }),
        );
        Self {
            address,
            messages,
            shutdown: Some(shutdown),
        }
    }

    /**
     * Return the envelope recipients of every received message, in arrival order.
     */
    fn envelopes(&self) -> Vec<Vec<String>> {
        self.messages
            .lock()
            .unwrap()
            .iter()
            .map(|message| {
                message
                    .facts
                    .envelope_to
                    .iter()
                    .map(|mailbox| mailbox.address.clone())
                    .collect()
            })
            .collect()
    }
}

impl Drop for Capture {
    /**
     * Stop the capture server.
     */
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

/// A running outbound dispatcher, stopped explicitly or when dropped.
struct Dispatcher {
    shutdown: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Dispatcher {
    /**
     * Start the outbound dispatcher of an application.
     */
    fn start(application: &Application) -> Self {
        let (shutdown, receiver) = watch::channel(false);
        let application = application.clone();
        let task = tokio::spawn(async move { application.run_outbound(receiver).await });
        Self {
            shutdown,
            task: Some(task),
        }
    }

    /**
     * Stop the dispatcher and wait for it.
     */
    async fn stop(mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.task.take() {
            task.await.unwrap();
        }
    }
}

impl Drop for Dispatcher {
    /**
     * Stop the dispatcher if a test ends without stopping it.
     */
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}

/**
 * Open empty in-memory storage.
 */
async fn storage() -> Arc<dyn Storage> {
    Arc::new(SqliteStorage::memory().await.unwrap())
}

/**
 * Create a user with a role and mail access.
 */
async fn user(storage: &Arc<dyn Storage>, role: GlobalRole, mail_access: MailAccess) -> User {
    storage
        .create_user(&NewUser {
            name: format!("{role:?}"),
            email: format!("{}@example.test", UserIdentifier::new()),
            password_hash: "hash".into(),
            global_role: role,
            mail_access,
            personal_filter: None,
        })
        .await
        .unwrap()
}

/**
 * Build the session identity of a stored user.
 */
fn session(user: &User) -> SessionUser {
    SessionUser {
        id: user.identifier,
        name: user.name.clone(),
        email: user.email.clone(),
        role: user.global_role,
    }
}

/**
 * Settings allowing loopback servers, with a key, fast polling and a short retry delay.
 */
fn settings() -> OutboundSettings {
    OutboundSettings {
        allow_internal_addresses: true,
        poll_interval: Duration::from_millis(25),
        retry_delay: Duration::from_secs(30),
        secret_key: Some(OutboundSecretKey::generate().unwrap()),
        ..OutboundSettings::default()
    }
}

/**
 * Build an application with outbound settings.
 */
async fn application(storage: Arc<dyn Storage>, settings: OutboundSettings) -> Application {
    let (events, _) = broadcast::channel::<MessageEvent>(64);
    Application::new(storage, events)
        .await
        .unwrap()
        .with_outbound_settings(settings)
}

/**
 * Save a scope with a filter under an optional parent.
 */
async fn scope(
    storage: &Arc<dyn Storage>,
    parent: Option<ScopeIdentifier>,
    filter: &str,
) -> ScopeIdentifier {
    let scope = Scope {
        identifier: ScopeIdentifier::new(),
        parent,
        name: format!("scope {filter}"),
        description: None,
        filter: filter.into(),
        position: 0,
        policy_version: 1,
    };
    storage.save_scope(&scope).await.unwrap();
    scope.identifier
}

/**
 * Build an enabled, unauthenticated plain SMTP Outbound request for a loopback port.
 */
fn outbound_request(name: &str, port: u16, allowed_recipients: &[&str]) -> SmtpOutboundRequest {
    SmtpOutboundRequest {
        name: name.into(),
        host: "127.0.0.1".into(),
        port,
        security: SmtpOutboundSecurity::None,
        authentication: SmtpOutboundAuthentication::None,
        username: None,
        password: None,
        enabled: Some(true),
        override_from: None,
        envelope_from: None,
        connect_timeout_seconds: Some(2),
        smtp_timeout_seconds: Some(2),
        verify_certificates: true,
        helo_name: None,
        maximum_connections: None,
        allowed_recipients: allowed_recipients
            .iter()
            .map(|pattern| (*pattern).to_owned())
            .collect(),
    }
}

/**
 * Build a routing action request.
 */
fn action_request(
    outbound: SmtpOutboundIdentifier,
    kind: RoutingActionKind,
    recipients: &[&str],
) -> RoutingActionRequest {
    RoutingActionRequest {
        smtp_outbound_identifier: outbound,
        kind,
        forward_recipients: recipients
            .iter()
            .map(|recipient| (*recipient).to_owned())
            .collect(),
        enabled: true,
    }
}

/**
 * Store a captured message with a subject, envelope sender and recipients, and notify.
 *
 * The raw MIME names only the first recipient in `To`; the others are Bcc recipients present
 * only in the envelope.
 */
async fn capture(
    application: &Application,
    storage: &Arc<dyn Storage>,
    subject: &str,
    recipients: &[&str],
) -> MessageIdentifier {
    let identifier = MessageIdentifier::new();
    let raw = format!(
        "Return-Path: <app@example.test>\r\nReceived: from app by sandpost.localhost\r\nFrom: App <app@example.test>\r\nTo: {}\r\nSubject: {subject}\r\n\r\nHello\r\n",
        recipients[0]
    );
    let mailbox = |address: &str| Mailbox {
        address: address.into(),
        domain: address.rsplit_once('@').unwrap().1.to_ascii_lowercase(),
    };
    let sequence = storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: subject.into(),
                envelope_from: Some(mailbox("app@example.test")),
                envelope_to: recipients.iter().map(|address| mailbox(address)).collect(),
                received_at: 1_700_000_000,
                ..MessageFacts::default()
            },
            raw_message: raw.into_bytes(),
            attachments: Vec::new(),
        })
        .await
        .unwrap();
    let _ = application.publish_message(MessageEvent {
        identifier,
        sequence,
    });
    identifier
}

/**
 * Wait until every delivery of a message is terminal, returning them.
 */
async fn settled(
    owner: &AuthorizedApplication,
    message: MessageIdentifier,
) -> Vec<OutboundDelivery> {
    for _ in 0..400 {
        let deliveries = owner.list_message_deliveries(message).await.unwrap();
        if !deliveries.is_empty()
            && deliveries
                .iter()
                .all(|delivery| delivery.status.is_terminal())
        {
            return deliveries;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("deliveries did not settle");
}

/**
 * Wait until a message has at least one delivery in the given status.
 */
async fn reached(
    owner: &AuthorizedApplication,
    message: MessageIdentifier,
    status: OutboundDeliveryStatus,
) -> OutboundDelivery {
    for _ in 0..400 {
        if let Some(delivery) = owner
            .list_message_deliveries(message)
            .await
            .unwrap()
            .into_iter()
            .find(|delivery| delivery.status == status)
        {
            return delivery;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no delivery reached {status:?}");
}

/**
 * Only Owners manage SMTP Outbounds, routing actions and deliveries, whatever the transport.
 */
#[tokio::test]
async fn only_owners_manage_outbound_delivery() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let created = application
        .authorized(&session(&owner))
        .create_smtp_outbound(outbound_request("Pippo", 25, &["*"]))
        .await
        .unwrap();
    let scope = scope(&storage, None, "").await;
    for role in [GlobalRole::Member, GlobalRole::Viewer] {
        let other = user(&storage, role, MailAccess::All).await;
        let caller = application.authorized(&session(&other));
        let identifier = created.outbound.identifier;
        let results = [
            caller.outbound_policy().await.err(),
            caller.list_smtp_outbounds().await.err(),
            caller.get_smtp_outbound(identifier).await.err(),
            caller
                .create_smtp_outbound(outbound_request("Pluto", 25, &[]))
                .await
                .err(),
            caller
                .update_smtp_outbound(identifier, outbound_request("Pippo", 26, &[]))
                .await
                .err(),
            caller
                .set_smtp_outbound_enabled(identifier, false)
                .await
                .err(),
            caller.test_smtp_outbound(identifier).await.err(),
            caller.delete_smtp_outbound(identifier).await.err(),
            caller.list_routing_actions(scope).await.err(),
            caller
                .create_routing_action(
                    scope,
                    action_request(identifier, RoutingActionKind::Relay, &[]),
                )
                .await
                .err(),
            caller
                .list_outbound_deliveries(OutboundDeliveryListRequest::default())
                .await
                .err(),
            caller
                .list_message_deliveries(MessageIdentifier::new())
                .await
                .err(),
        ];
        for result in results {
            assert!(
                matches!(result, Some(ApplicationError::Forbidden)),
                "{role:?}: {result:?}"
            );
        }
    }
}

/**
 * Passwords are sealed with the secret key, never returned, kept when omitted, replaced when
 * supplied, cleared without authentication, and refused when no key is configured.
 */
#[tokio::test]
async fn passwords_are_encrypted_and_never_returned() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let mut request = outbound_request("Pippo", 587, &["*"]);
    request.security = SmtpOutboundSecurity::StartTls;
    request.authentication = SmtpOutboundAuthentication::Plain;
    request.username = Some("relay".into());
    request.password = Some("very-secret-password".into());
    let created = caller.create_smtp_outbound(request.clone()).await.unwrap();
    assert!(created.outbound.password_configured);
    let identifier = created.outbound.identifier;
    for serialized in [
        serde_json::to_string(&created).unwrap(),
        serde_json::to_string(&caller.list_smtp_outbounds().await.unwrap()).unwrap(),
        serde_json::to_string(&caller.get_smtp_outbound(identifier).await.unwrap()).unwrap(),
    ] {
        assert!(!serialized.contains("very-secret-password"), "{serialized}");
        assert!(!serialized.contains("encrypted"), "{serialized}");
    }
    let sealed = storage
        .get_smtp_outbound_credential(identifier)
        .await
        .unwrap()
        .unwrap()
        .encrypted_password
        .unwrap();
    assert!(
        !sealed
            .windows(b"very-secret".len())
            .any(|window| window == b"very-secret")
    );
    let key = application.outbound_settings().secret_key.clone().unwrap();
    assert_eq!(
        key.open(identifier, &sealed).as_deref(),
        Some("very-secret-password")
    );

    let mut kept = request.clone();
    kept.password = None;
    kept.host = "smtp.example.test".into();
    caller.update_smtp_outbound(identifier, kept).await.unwrap();
    let after_keep = storage
        .get_smtp_outbound_credential(identifier)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_keep.encrypted_password.as_deref(),
        Some(sealed.as_slice())
    );

    let mut replaced = request.clone();
    replaced.password = Some("rotated".into());
    caller
        .update_smtp_outbound(identifier, replaced)
        .await
        .unwrap();
    let after_replace = storage
        .get_smtp_outbound_credential(identifier)
        .await
        .unwrap()
        .unwrap()
        .encrypted_password
        .unwrap();
    assert_eq!(
        key.open(identifier, &after_replace).as_deref(),
        Some("rotated")
    );

    let mut cleared = request.clone();
    cleared.authentication = SmtpOutboundAuthentication::None;
    cleared.password = None;
    let without = caller
        .update_smtp_outbound(identifier, cleared)
        .await
        .unwrap();
    assert!(!without.outbound.password_configured);
    assert_eq!(without.outbound.username, None);

    let keyless = Application::new(storage.clone(), broadcast::channel(4).0)
        .await
        .unwrap();
    let refused = keyless
        .authorized(&session(&owner))
        .create_smtp_outbound(SmtpOutboundRequest {
            name: "Keyless".into(),
            ..request
        })
        .await;
    assert!(
        matches!(refused, Err(ApplicationError::InvalidInput(_))),
        "{refused:?}"
    );
}

/**
 * Incomplete authentication, invalid hosts and out-of-range settings are refused, and names are
 * unique ignoring case.
 */
#[tokio::test]
async fn invalid_outbound_definitions_are_refused() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let base = outbound_request("Pippo", 587, &[]);
    let mut cases = Vec::new();
    let mut missing_username = base.clone();
    missing_username.authentication = SmtpOutboundAuthentication::Login;
    missing_username.password = Some("secret".into());
    cases.push(missing_username);
    let mut missing_password = base.clone();
    missing_password.authentication = SmtpOutboundAuthentication::Plain;
    missing_password.username = Some("relay".into());
    cases.push(missing_password);
    let mut url_host = base.clone();
    url_host.host = "smtp://example.test".into();
    cases.push(url_host);
    let mut port = base.clone();
    port.port = 0;
    cases.push(port);
    let mut timeout = base.clone();
    timeout.connect_timeout_seconds = Some(0);
    cases.push(timeout);
    let mut connections = base.clone();
    connections.maximum_connections = Some(100);
    cases.push(connections);
    let mut sender = base.clone();
    sender.envelope_from = Some("not an address".into());
    cases.push(sender);
    let mut pattern = base.clone();
    pattern.allowed_recipients = vec!["two words".into()];
    cases.push(pattern);
    for request in cases {
        let result = caller.create_smtp_outbound(request).await;
        assert!(
            matches!(result, Err(ApplicationError::InvalidInput(_))),
            "{result:?}"
        );
    }
    caller.create_smtp_outbound(base.clone()).await.unwrap();
    let duplicate = caller
        .create_smtp_outbound(SmtpOutboundRequest {
            name: "PIPPO".into(),
            ..base
        })
        .await;
    assert!(
        matches!(duplicate, Err(ApplicationError::InUse(_))),
        "{duplicate:?}"
    );
}

/**
 * A scope holds several actions sharing one SMTP Outbound, forward recipients must pass the
 * allowlist, and a referenced SMTP Outbound cannot be deleted until its actions are gone.
 */
#[tokio::test]
async fn routing_actions_reference_outbounds_safely() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let pippo = caller
        .create_smtp_outbound(outbound_request("Pippo", 25, &["*@example.test"]))
        .await
        .unwrap()
        .outbound
        .identifier;
    let scope = scope(&storage, None, "").await;
    let refused = caller
        .create_routing_action(
            scope,
            action_request(pippo, RoutingActionKind::Forward, &["qa@elsewhere.test"]),
        )
        .await;
    assert!(
        matches!(refused, Err(ApplicationError::InvalidInput(_))),
        "{refused:?}"
    );
    let unknown = caller
        .create_routing_action(
            scope,
            action_request(SmtpOutboundIdentifier::new(), RoutingActionKind::Relay, &[]),
        )
        .await;
    assert!(
        matches!(unknown, Err(ApplicationError::InvalidInput(_))),
        "{unknown:?}"
    );
    let missing_scope = caller
        .create_routing_action(
            ScopeIdentifier::new(),
            action_request(pippo, RoutingActionKind::Relay, &[]),
        )
        .await;
    assert!(
        matches!(missing_scope, Err(ApplicationError::NotFound)),
        "{missing_scope:?}"
    );
    let forward = caller
        .create_routing_action(
            scope,
            action_request(pippo, RoutingActionKind::Forward, &["QA@example.test"]),
        )
        .await
        .unwrap();
    assert_eq!(forward.forward_recipients, ["qa@example.test"]);
    let second_forward = caller
        .create_routing_action(
            scope,
            action_request(pippo, RoutingActionKind::Forward, &["audit@example.test"]),
        )
        .await
        .unwrap();
    let relay = caller
        .create_routing_action(scope, action_request(pippo, RoutingActionKind::Relay, &[]))
        .await
        .unwrap();
    assert_eq!(caller.list_routing_actions(scope).await.unwrap().len(), 3);
    assert_eq!(
        caller
            .get_smtp_outbound(pippo)
            .await
            .unwrap()
            .routing_action_count,
        3
    );
    let in_use = caller.delete_smtp_outbound(pippo).await;
    assert!(
        matches!(in_use, Err(ApplicationError::InUse(_))),
        "{in_use:?}"
    );
    for action in [forward, second_forward, relay] {
        caller
            .delete_routing_action(action.identifier)
            .await
            .unwrap();
    }
    caller.delete_smtp_outbound(pippo).await.unwrap();
    assert!(matches!(
        caller.get_smtp_outbound(pippo).await,
        Err(ApplicationError::NotFound)
    ));
}

/**
 * Only the most specific matching scopes route a message: a parent's actions do not apply to
 * mail its child also matches, while sibling scopes both apply.
 */
#[tokio::test]
async fn routing_uses_the_most_specific_matching_scopes() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let outbound = caller
        .create_smtp_outbound(outbound_request("Pippo", 25, &["*"]))
        .await
        .unwrap()
        .outbound
        .identifier;
    let parent = scope(&storage, None, "subject contains \"client\"").await;
    let child = scope(&storage, Some(parent), "subject contains \"reset\"").await;
    let sibling = scope(&storage, None, "subject contains \"urgent\"").await;
    let mut actions = std::collections::HashMap::new();
    for (name, scope) in [("parent", parent), ("child", child), ("sibling", sibling)] {
        let action = caller
            .create_routing_action(
                scope,
                action_request(
                    outbound,
                    RoutingActionKind::Forward,
                    &[&format!("{name}@example.test")],
                ),
            )
            .await
            .unwrap();
        actions.insert(action.identifier, name);
    }
    let cases = [
        ("client welcome", vec!["parent"]),
        ("client password reset", vec!["child"]),
        ("urgent client password reset", vec!["child", "sibling"]),
        ("unrelated", vec![]),
    ];
    let mut messages = Vec::new();
    for (subject, _) in &cases {
        messages.push(capture(&application, &storage, subject, &["someone@example.test"]).await);
    }
    application.outbound.evaluate(&application).await.unwrap();
    for ((subject, expected), message) in cases.iter().zip(messages) {
        let mut routed: Vec<&str> = caller
            .list_message_deliveries(message)
            .await
            .unwrap()
            .iter()
            .map(|delivery| actions[&delivery.routing_action_identifier.unwrap()])
            .collect();
        routed.sort_unstable();
        assert_eq!(&routed, expected, "{subject}");
    }
}

/**
 * Mail is captured without any routing trace while no action is configured, and disabled actions
 * or outbounds route nothing.
 */
#[tokio::test]
async fn disabled_or_missing_routing_delivers_nothing() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let unrouted = capture(&application, &storage, "plain capture", &["a@example.test"]).await;
    assert!(
        storage
            .list_routing_evaluations(10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(caller.get_message(unrouted).await.is_ok());

    let target = Capture::start(AuthenticationPolicy::Optional).await;
    let outbound = caller
        .create_smtp_outbound(outbound_request("Pippo", target.address.port(), &["*"]))
        .await
        .unwrap()
        .outbound
        .identifier;
    let scope = scope(&storage, None, "").await;
    let mut disabled = action_request(outbound, RoutingActionKind::Relay, &[]);
    disabled.enabled = false;
    let action = caller
        .create_routing_action(scope, disabled.clone())
        .await
        .unwrap();
    capture(
        &application,
        &storage,
        "disabled action",
        &["a@example.test"],
    )
    .await;
    assert!(
        storage
            .list_routing_evaluations(10)
            .await
            .unwrap()
            .is_empty()
    );

    disabled.enabled = true;
    caller
        .update_routing_action(action.identifier, disabled)
        .await
        .unwrap();
    caller
        .set_smtp_outbound_enabled(outbound, false)
        .await
        .unwrap();
    let message = capture(
        &application,
        &storage,
        "disabled outbound",
        &["a@example.test"],
    )
    .await;
    assert!(
        storage
            .list_routing_evaluations(10)
            .await
            .unwrap()
            .is_empty()
    );
    application.outbound.evaluate(&application).await.unwrap();
    assert!(
        caller
            .list_message_deliveries(message)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(target.messages.lock().unwrap().is_empty());
}

/**
 * Relay delivers the original MIME to the SMTP envelope recipients (Bcc included), forward
 * delivers a copy only to its own recipients, and both share one SMTP Outbound.
 */
#[tokio::test]
async fn relay_and_forward_deliver_through_one_outbound() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let target = Capture::start(AuthenticationPolicy::Required).await;
    let mut request = outbound_request("Pippo", target.address.port(), &["*@example.test"]);
    request.authentication = SmtpOutboundAuthentication::Login;
    request.username = Some("relay".into());
    request.password = Some("secret".into());
    request.envelope_from = Some("bounces@example.test".into());
    let outbound = caller
        .create_smtp_outbound(request)
        .await
        .unwrap()
        .outbound
        .identifier;
    let scope = scope(&storage, None, "").await;
    caller
        .create_routing_action(
            scope,
            action_request(outbound, RoutingActionKind::Relay, &[]),
        )
        .await
        .unwrap();
    caller
        .create_routing_action(
            scope,
            action_request(outbound, RoutingActionKind::Forward, &["qa@example.test"]),
        )
        .await
        .unwrap();
    let dispatcher = Dispatcher::start(&application);
    let message = capture(
        &application,
        &storage,
        "Welcome",
        &["first@example.test", "Hidden.Bcc@example.test"],
    )
    .await;
    let deliveries = settled(&caller, message).await;
    dispatcher.stop().await;
    assert_eq!(deliveries.len(), 2);
    for delivery in &deliveries {
        assert_eq!(
            delivery.status,
            OutboundDeliveryStatus::Delivered,
            "{delivery:?}"
        );
        assert_eq!(
            delivery.envelope_from.as_deref(),
            Some("bounces@example.test")
        );
        assert_eq!(delivery.smtp_outbound_name, "Pippo");
        assert!(delivery.recipients.iter().all(|recipient| {
            recipient.status == OutboundRecipientStatus::Delivered && recipient.response.is_some()
        }));
    }
    let mut envelopes: Vec<Vec<String>> = target
        .envelopes()
        .into_iter()
        .map(|mut envelope| {
            envelope.sort();
            envelope
        })
        .collect();
    envelopes.sort();
    assert_eq!(
        envelopes,
        [
            vec![
                "Hidden.Bcc@example.test".to_owned(),
                "first@example.test".to_owned()
            ],
            vec!["qa@example.test".to_owned()],
        ],
        "relay uses the envelope (Bcc included); forward only its own recipients"
    );
    let received = target.messages.lock().unwrap().clone();
    let relayed = received
        .iter()
        .find(|message| message.facts.envelope_to.len() == 2)
        .expect("the relayed copy arrived");
    let mut relayed_recipients: Vec<_> = relayed
        .facts
        .envelope_to
        .iter()
        .map(|mailbox| mailbox.address.as_str())
        .collect();
    relayed_recipients.sort_unstable();
    assert_eq!(
        relayed_recipients,
        ["Hidden.Bcc@example.test", "first@example.test"]
    );
    let forwarded = received
        .iter()
        .find(|message| message.facts.envelope_to.len() == 1)
        .expect("the forwarded copy arrived");
    assert_eq!(forwarded.facts.envelope_to[0].address, "qa@example.test");
    for copy in received.iter() {
        let text = String::from_utf8_lossy(&copy.raw_message);
        assert!(
            !text.contains("Hidden.Bcc"),
            "Bcc never enters headers: {text}"
        );
        assert!(text.contains("Subject: Welcome\r\n"));
        assert!(text.ends_with("\r\nHello\r\n"));
        assert_eq!(
            text.matches("Return-Path").count(),
            1,
            "only the receiver's own"
        );
        assert_eq!(
            copy.facts
                .envelope_from
                .as_ref()
                .map(|mailbox| mailbox.address.as_str()),
            Some("bounces@example.test")
        );
    }
    let stored = storage.get_message(message).await.unwrap().unwrap();
    assert!(
        String::from_utf8_lossy(&stored.raw_message).starts_with("Return-Path: <app@example.test>")
    );
}

/**
 * Recipients outside the allowlist fail without being attempted while permitted ones are
 * delivered, so the delivery is recorded per recipient.
 */
#[tokio::test]
async fn the_allowlist_limits_relay_recipients() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let target = Capture::start(AuthenticationPolicy::Optional).await;
    let outbound = caller
        .create_smtp_outbound(outbound_request(
            "Pippo",
            target.address.port(),
            &["*@allowed.test"],
        ))
        .await
        .unwrap()
        .outbound
        .identifier;
    let scope = scope(&storage, None, "").await;
    caller
        .create_routing_action(
            scope,
            action_request(outbound, RoutingActionKind::Relay, &[]),
        )
        .await
        .unwrap();
    let dispatcher = Dispatcher::start(&application);
    let message = capture(
        &application,
        &storage,
        "Mixed",
        &["customer@real-world.test", "qa@allowed.test"],
    )
    .await;
    let delivery = settled(&caller, message).await.remove(0);
    dispatcher.stop().await;
    assert_eq!(delivery.status, OutboundDeliveryStatus::Failed);
    let statuses: Vec<_> = delivery
        .recipients
        .iter()
        .map(|recipient| (recipient.address.as_str(), recipient.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("customer@real-world.test", OutboundRecipientStatus::Failed),
            ("qa@allowed.test", OutboundRecipientStatus::Delivered),
        ]
    );
    assert_eq!(
        delivery.recipients[0]
            .error
            .as_ref()
            .map(|error| error.kind),
        Some(OutboundFailureKind::Policy)
    );
    assert_eq!(target.envelopes(), [vec!["qa@allowed.test".to_owned()]]);

    let blocked = capture(
        &application,
        &storage,
        "Blocked",
        &["customer@real-world.test"],
    )
    .await;
    application.outbound.evaluate(&application).await.unwrap();
    let blocked = caller
        .list_message_deliveries(blocked)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(blocked.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        blocked.last_error.map(|error| error.kind),
        Some(OutboundFailureKind::Policy)
    );
}

/**
 * A transient failure is retried with backoff until the attempts run out, a permanent failure is
 * not retried, and an Owner can schedule a failed delivery again.
 */
#[tokio::test]
async fn transient_failures_retry_and_permanent_failures_stop() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(
        storage.clone(),
        OutboundSettings {
            maximum_attempts: 2,
            ..settings()
        },
    )
    .await;
    let caller = application.authorized(&session(&owner));
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let unreachable = caller
        .create_smtp_outbound(outbound_request("Unreachable", closed_port, &["*"]))
        .await
        .unwrap()
        .outbound
        .identifier;
    let target = Capture::start(AuthenticationPolicy::Required).await;
    let mut wrong = outbound_request("Wrong password", target.address.port(), &["*"]);
    wrong.authentication = SmtpOutboundAuthentication::Plain;
    wrong.username = Some("relay".into());
    wrong.password = Some("wrong".into());
    let rejected = caller
        .create_smtp_outbound(wrong)
        .await
        .unwrap()
        .outbound
        .identifier;
    let transient_scope = scope(&storage, None, "subject contains \"transient\"").await;
    let permanent_scope = scope(&storage, None, "subject contains \"permanent\"").await;
    caller
        .create_routing_action(
            transient_scope,
            action_request(unreachable, RoutingActionKind::Relay, &[]),
        )
        .await
        .unwrap();
    caller
        .create_routing_action(
            permanent_scope,
            action_request(rejected, RoutingActionKind::Relay, &[]),
        )
        .await
        .unwrap();
    let dispatcher = Dispatcher::start(&application);
    let transient = capture(&application, &storage, "transient", &["a@example.test"]).await;
    let permanent = capture(&application, &storage, "permanent", &["a@example.test"]).await;
    let retrying = reached(&caller, transient, OutboundDeliveryStatus::Retrying).await;
    assert_eq!(retrying.attempt_count, 1);
    assert_eq!(
        retrying.last_error.as_ref().map(|error| error.kind),
        Some(OutboundFailureKind::Connection)
    );
    let delay = retrying.next_attempt_at.unwrap() - retrying.last_attempt_at.unwrap();
    assert!(
        (30..=31).contains(&delay),
        "first backoff is the retry delay: {delay}"
    );
    assert_eq!(
        retrying.recipients[0].status,
        OutboundRecipientStatus::Pending
    );
    let failed = settled(&caller, permanent).await.remove(0);
    assert_eq!(failed.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        failed.attempt_count, 1,
        "a permanent failure is not retried"
    );
    let error = failed.last_error.unwrap();
    assert_eq!(error.kind, OutboundFailureKind::Authentication);
    assert_eq!(error.code, Some(535));
    dispatcher.stop().await;

    assert_eq!(application.outbound.backoff(1), 30);
    assert_eq!(application.outbound.backoff(2), 60);
    assert_eq!(
        application.outbound.backoff(30),
        3600,
        "the delay is capped"
    );

    // Make the retry due now instead of waiting for the backoff, then exhaust the attempts.
    storage
        .claim_outbound_delivery(retrying.identifier, i64::MAX)
        .await
        .unwrap()
        .unwrap();
    storage.interrupt_outbound_deliveries(0).await.unwrap();
    let dispatcher = Dispatcher::start(&application);
    let exhausted = settled(&caller, transient).await.remove(0);
    dispatcher.stop().await;
    assert_eq!(exhausted.status, OutboundDeliveryStatus::Failed);
    assert!(exhausted.attempt_count >= 2);
    assert!(
        exhausted.recipients[0]
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("gave up after")),
        "{exhausted:?}"
    );

    let retried = caller
        .retry_outbound_delivery(failed.identifier)
        .await
        .unwrap();
    assert_eq!(retried.status, OutboundDeliveryStatus::Retrying);
    assert_eq!(
        retried.recipients[0].status,
        OutboundRecipientStatus::Pending
    );
    let not_failed = caller.retry_outbound_delivery(failed.identifier).await;
    assert!(
        matches!(not_failed, Err(ApplicationError::InUse(_))),
        "{not_failed:?}"
    );
}

/**
 * A delivery left processing by a stopped process is retried after restart and its attempt is
 * recorded as interrupted.
 */
#[tokio::test]
async fn restart_recovers_interrupted_deliveries() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let first = application(storage.clone(), settings()).await;
    let caller = first.authorized(&session(&owner));
    let target = Capture::start(AuthenticationPolicy::Optional).await;
    let outbound = caller
        .create_smtp_outbound(outbound_request("Pippo", target.address.port(), &["*"]))
        .await
        .unwrap()
        .outbound
        .identifier;
    let scope = scope(&storage, None, "").await;
    caller
        .create_routing_action(
            scope,
            action_request(outbound, RoutingActionKind::Relay, &[]),
        )
        .await
        .unwrap();
    let message = capture(&first, &storage, "Interrupted", &["a@example.test"]).await;
    first.outbound.evaluate(&first).await.unwrap();
    let pending = caller
        .list_message_deliveries(message)
        .await
        .unwrap()
        .remove(0);
    // The first process claimed the job and stopped before recording the outcome.
    storage
        .claim_outbound_delivery(pending.identifier, i64::MAX)
        .await
        .unwrap()
        .unwrap();
    drop(first);

    let restarted = application(storage.clone(), settings()).await;
    let caller = restarted.authorized(&session(&owner));
    let dispatcher = Dispatcher::start(&restarted);
    let delivered = settled(&caller, message).await.remove(0);
    dispatcher.stop().await;
    assert_eq!(delivered.status, OutboundDeliveryStatus::Delivered);
    assert_eq!(target.envelopes().len(), 1);
    let history = caller
        .get_outbound_delivery(delivered.identifier)
        .await
        .unwrap();
    let outcomes: Vec<_> = history
        .attempts
        .iter()
        .map(|attempt| attempt.outcome)
        .collect();
    assert_eq!(
        outcomes,
        [
            Some(OutboundAttemptOutcome::Interrupted),
            Some(OutboundAttemptOutcome::Delivered)
        ]
    );
}

/**
 * Delivery history follows the Owner's own mail visibility, so it never reveals unreadable mail.
 */
#[tokio::test]
async fn delivery_history_follows_the_owner_visibility() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let scoped_owner = user(&storage, GlobalRole::Owner, MailAccess::Scoped).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let outbound = caller
        .create_smtp_outbound(outbound_request("Pippo", 25, &["*"]))
        .await
        .unwrap()
        .outbound
        .identifier;
    let scope = scope(&storage, None, "").await;
    caller
        .create_routing_action(
            scope,
            action_request(outbound, RoutingActionKind::Relay, &[]),
        )
        .await
        .unwrap();
    let message = capture(&application, &storage, "Secret", &["a@example.test"]).await;
    application.outbound.evaluate(&application).await.unwrap();
    assert_eq!(
        caller.list_message_deliveries(message).await.unwrap().len(),
        1
    );
    let delivery = caller
        .list_outbound_deliveries(OutboundDeliveryListRequest::default())
        .await
        .unwrap()
        .remove(0);
    let blind = application.authorized(&session(&scoped_owner));
    assert!(matches!(
        blind.list_message_deliveries(message).await,
        Err(ApplicationError::NotFound)
    ));
    assert!(
        blind
            .list_outbound_deliveries(OutboundDeliveryListRequest::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        blind.get_outbound_delivery(delivery.identifier).await,
        Err(ApplicationError::NotFound)
    ));
}

/**
 * Connection tests use saved or draft settings, reuse the stored password for drafts, and are
 * refused by the destination policy for internal hosts unless allowed.
 */
#[tokio::test]
async fn connection_tests_send_no_mail() {
    let storage = storage().await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let application = application(storage.clone(), settings()).await;
    let caller = application.authorized(&session(&owner));
    let target = Capture::start(AuthenticationPolicy::Required).await;
    let mut request = outbound_request("Pippo", target.address.port(), &[]);
    request.authentication = SmtpOutboundAuthentication::Plain;
    request.username = Some("relay".into());
    request.password = Some("secret".into());
    let outbound = caller
        .create_smtp_outbound(request.clone())
        .await
        .unwrap()
        .outbound
        .identifier;
    let saved = caller.test_smtp_outbound(outbound).await.unwrap();
    assert!(saved.success, "{:?}", saved.error);
    assert!(saved.authenticated);
    let draft = caller
        .test_smtp_outbound_draft(SmtpOutboundTestRequest {
            outbound: SmtpOutboundRequest {
                password: None,
                ..request.clone()
            },
            identifier: Some(outbound),
        })
        .await
        .unwrap();
    assert!(draft.success, "{:?}", draft.error);
    let wrong = caller
        .test_smtp_outbound_draft(SmtpOutboundTestRequest {
            outbound: SmtpOutboundRequest {
                password: Some("wrong".into()),
                ..request.clone()
            },
            identifier: None,
        })
        .await
        .unwrap();
    assert_eq!(
        wrong.error.map(|error| error.kind),
        Some(OutboundFailureKind::Authentication)
    );
    assert!(target.messages.lock().unwrap().is_empty());

    let guarded = application_with_policy(storage.clone()).await;
    let refused = guarded
        .authorized(&session(&owner))
        .test_smtp_outbound(outbound)
        .await;
    // The guarded application has no key for the stored password, so the policy is checked
    // with a draft that needs none.
    assert!(
        matches!(refused, Err(ApplicationError::InvalidInput(_))),
        "{refused:?}"
    );
    let policy = guarded
        .authorized(&session(&owner))
        .test_smtp_outbound_draft(SmtpOutboundTestRequest {
            outbound: outbound_request("Draft", target.address.port(), &[]),
            identifier: None,
        })
        .await
        .unwrap();
    assert_eq!(
        policy.error.map(|error| error.kind),
        Some(OutboundFailureKind::Policy)
    );
}

/**
 * Build an application with default outbound settings, which refuse internal destinations.
 */
async fn application_with_policy(storage: Arc<dyn Storage>) -> Application {
    application(storage, OutboundSettings::default()).await
}
