//! Mail captured by the real SMTP listener and relayed through an SMTP Outbound.

use super::personas::submit_html;
use super::*;
use sandpost_application::{RoutingActionRequest, SmtpOutboundRequest};
use sandpost_core::{
    Message, OutboundDeliveryStatus, RoutingActionKind, Scope, ScopeIdentifier,
    SmtpOutboundAuthentication, SmtpOutboundSecurity,
};
use sandpost_smtp::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    SessionHandler, SmtpServer,
};
use std::sync::{Arc, Mutex};

/// External SMTP server stand-in that records every delivered message.
struct RecordingHandler(Arc<Mutex<Vec<Message>>>);

impl SessionHandler for RecordingHandler {
    type Principal = ();

    /**
     * Refuse credentials; the relay under test does not authenticate.
     */
    fn authenticate(
        &self,
        _credentials: Credentials,
    ) -> impl std::future::Future<Output = Result<AuthenticationOutcome<()>, AuthenticationError>> + Send
    {
        std::future::ready(Ok(AuthenticationOutcome::InvalidCredentials))
    }

    /**
     * Record the delivered message.
     */
    fn deliver(
        &self,
        message: Message,
        _principal: Option<()>,
    ) -> impl std::future::Future<Output = Result<(), DeliveryError>> + Send {
        self.0.lock().unwrap().push(message);
        std::future::ready(Ok(()))
    }
}

/// SMTP capture stays sandboxed until a scope's relay action routes matching mail, which then
/// reaches the external server asynchronously with its original envelope recipient.
#[tokio::test]
async fn captured_mail_is_relayed_through_an_smtp_outbound() {
    let (mut configuration, directory) = test_configuration();
    configuration.outbound.allow_internal_addresses = true;
    let running = SandpostRuntime::start(configuration).await.unwrap();
    assert!(
        directory.0.join("outbound-secret.key").exists(),
        "the password key is created beside the database"
    );
    let instance = running.application.instance();
    let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
    instance
        .setup(sandpost_application::SetupRequest {
            bootstrap_secret: secret,
            name: "Owner".into(),
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    let login = instance
        .login(sandpost_application::LoginRequest {
            remember_me: false,
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    let owner = running.application.authorized(&login.user);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let external = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let (stop_external, external_stopped) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(
        SmtpServer::new(RecordingHandler(received.clone()))
            .authentication(AuthenticationPolicy::Optional)
            .serve(listener, async {
                let _ = external_stopped.await;
            }),
    );

    submit_html(
        running.mail_listen_address,
        "customer@example.test",
        "Relay me before configuration",
        "<p>sandboxed</p>",
    )
    .await;

    let outbound = owner
        .create_smtp_outbound(SmtpOutboundRequest {
            name: "Pippo".into(),
            host: "127.0.0.1".into(),
            port: external.port(),
            security: SmtpOutboundSecurity::None,
            authentication: SmtpOutboundAuthentication::None,
            username: None,
            password: None,
            enabled: Some(true),
            override_from: None,
            envelope_from: None,
            connect_timeout_seconds: Some(5),
            smtp_timeout_seconds: Some(5),
            verify_certificates: true,
            helo_name: None,
            maximum_connections: None,
            allowed_recipients: vec!["*@example.test".into()],
        })
        .await
        .unwrap()
        .outbound
        .identifier;
    let scope = ScopeIdentifier::new();
    owner
        .save_scope(Scope {
            identifier: scope,
            parent: None,
            name: "Scope A".into(),
            description: None,
            filter: "subject contains \"Relay\"".into(),
            position: 0,
            policy_version: 0,
        })
        .await
        .unwrap();
    owner
        .create_routing_action(
            scope,
            RoutingActionRequest {
                smtp_outbound_identifier: outbound,
                kind: RoutingActionKind::Relay,
                forward_recipients: Vec::new(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    submit_html(
        running.mail_listen_address,
        "other@example.test",
        "Not routed",
        "<p>stays in the sandbox</p>",
    )
    .await;
    submit_html(
        running.mail_listen_address,
        "Customer@example.test",
        "Relay me",
        "<p>delivered</p>",
    )
    .await;

    let mut delivered = None;
    for _ in 0..300 {
        let deliveries = owner
            .list_outbound_deliveries(Default::default())
            .await
            .unwrap();
        if let Some(delivery) = deliveries
            .into_iter()
            .find(|delivery| delivery.status.is_terminal())
        {
            delivered = Some(delivery);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let delivered = delivered.expect("the relay finished");
    assert_eq!(delivered.status, OutboundDeliveryStatus::Delivered);
    assert_eq!(delivered.subject, "Relay me");
    let messages = received.lock().unwrap().clone();
    assert_eq!(
        messages.len(),
        1,
        "only the routed message left the sandbox"
    );
    assert_eq!(messages[0].facts.subject, "Relay me");
    assert_eq!(
        messages[0].facts.envelope_to[0].address,
        "Customer@example.test"
    );
    assert_eq!(
        owner
            .list_outbound_deliveries(Default::default())
            .await
            .unwrap()
            .len(),
        1,
        "mail captured before the action and unmatched mail create no deliveries"
    );
    let _ = stop_external.send(());
    running.shutdown().await.unwrap();
}
