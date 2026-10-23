//! Persona ownership, authorization, execution, recovery, and isolation tests.
use super::access::classify_address;
use crate::{
    AddressAccess, Application, ApplicationError, MessageEvent, PersonaExecutionDetails,
    PersonaRequest, PersonaSettings, QueryOptions, SessionUser,
};
use sandpost_core::{
    GlobalRole, MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier, PersonaActionDetail,
    PersonaActionStatus, PersonaExecutionAction, PersonaExecutionStatus, PersonaIdentifier, Scope,
    ScopeIdentifier, User, UserIdentifier,
};
use sandpost_query::compile;
use sandpost_storage::{NewUser, Storage, UpdateUser};
use sandpost_storage_sqlite::SqliteStorage;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch};

/// A loopback server recording request paths in arrival order.
struct MockServer {
    address: SocketAddr,
    paths: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockServer {
    /**
     * Answer `/hang` never, `/missing` with 404, and everything else with a small 200 body.
     */
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let paths = paths.clone();
            async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let paths = paths.clone();
                    tokio::spawn(async move {
                        let mut head = Vec::new();
                        let mut buffer = [0_u8; 1024];
                        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                            match stream.read(&mut buffer).await {
                                Ok(0) | Err(_) => return,
                                Ok(read) => head.extend_from_slice(&buffer[..read]),
                            }
                        }
                        let path = String::from_utf8_lossy(&head)
                            .split(' ')
                            .nth(1)
                            .unwrap_or_default()
                            .to_owned();
                        paths.lock().unwrap().push(path.clone());
                        if path == "/hang" {
                            tokio::time::sleep(Duration::from_secs(3600)).await;
                            return;
                        }
                        let status = if path == "/missing" {
                            "404 Not Found"
                        } else {
                            "200 OK"
                        };
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: image/gif\r\nContent-Length: 4\r\nConnection: close\r\n\r\nGIF8"
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Self {
            address,
            paths,
            task,
        }
    }

    /**
     * Return an absolute URL on this server.
     */
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /**
     * Return the paths requested so far.
     */
    fn paths(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    /**
     * Stop accepting connections.
     */
    fn drop(&mut self) {
        self.task.abort();
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
 * Assign a scoped user to a new root scope with the supplied filter.
 */
async fn assign_scope(storage: &Arc<dyn Storage>, user: &User, filter: &str) -> ScopeIdentifier {
    let scope = Scope {
        identifier: ScopeIdentifier::new(),
        parent: None,
        name: "scope".into(),
        description: None,
        filter: filter.into(),
        position: 0,
        policy_version: 1,
    };
    storage.save_scope(&scope).await.unwrap();
    storage
        .assign_user_to_scope(user.identifier, scope.identifier)
        .await
        .unwrap();
    scope.identifier
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
 * Settings that allow the loopback mock server and keep tests fast.
 */
fn settings() -> PersonaSettings {
    PersonaSettings {
        allow_internal_addresses: true,
        request_timeout: Duration::from_secs(3),
        poll_interval: Duration::from_millis(25),
        ..PersonaSettings::default()
    }
}

/**
 * Build an application with Persona settings and its message notification channel.
 */
async fn application(storage: Arc<dyn Storage>, settings: PersonaSettings) -> Application {
    let (events, _) = broadcast::channel::<MessageEvent>(64);
    Application::new(storage, events)
        .await
        .unwrap()
        .with_persona_settings(settings)
}

/// A running dispatcher stopped when dropped.
struct Dispatcher {
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Dispatcher {
    /**
     * Start the Persona dispatcher of an application.
     */
    fn start(application: &Application) -> Self {
        let (shutdown, receiver) = watch::channel(false);
        let application = application.clone();
        let task = tokio::spawn(async move { application.run_personas(receiver).await });
        Self { shutdown, task }
    }

    /**
     * Stop the dispatcher and wait until its executions have stopped.
     */
    async fn stop(self) {
        self.shutdown.send_replace(true);
        self.task.await.unwrap();
    }
}

/**
 * Build a Persona request.
 */
fn request(address: &str, script: &str) -> PersonaRequest {
    PersonaRequest {
        name: "Alice".into(),
        address: address.into(),
        icon: None,
        script: script.into(),
        enabled: true,
    }
}

/**
 * Store a message with envelope recipients, subject, and HTML, then notify the dispatcher.
 */
async fn deliver(
    application: &Application,
    storage: &Arc<dyn Storage>,
    recipients: &[&str],
    subject: &str,
    html: &str,
) -> MessageIdentifier {
    let identifier = MessageIdentifier::new();
    let sequence = storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: subject.into(),
                markup_body: html.into(),
                envelope_to: recipients
                    .iter()
                    .map(|address| Mailbox {
                        address: (*address).into(),
                        domain: address.rsplit_once('@').unwrap().1.to_ascii_lowercase(),
                    })
                    .collect(),
                received_at: 1_700_000_000,
                ..MessageFacts::default()
            },
            raw_message: Vec::new(),
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
 * Poll a Persona's newest visible execution until it is terminal.
 */
async fn finished(
    application: &Application,
    owner: &User,
    persona: PersonaIdentifier,
) -> PersonaExecutionDetails {
    let reader = application.authorized(&session(owner));
    for _ in 0..400 {
        if let Some(summary) = reader
            .list_persona_executions(persona, None, 1)
            .await
            .unwrap()
            .into_iter()
            .next()
            && summary.execution.status.is_terminal()
        {
            return reader
                .get_persona_execution(persona, summary.execution.identifier)
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the execution did not finish");
}

/**
 * Wait until a Persona has a running execution and return it.
 */
async fn running(storage: &Arc<dyn Storage>) -> sandpost_core::PersonaExecution {
    for _ in 0..400 {
        if let Some(execution) = storage
            .list_persona_executions_in_status(PersonaExecutionStatus::Running, None, 10)
            .await
            .unwrap()
            .into_iter()
            .next()
        {
            return execution;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no execution started");
}

/**
 * Return the kinds and statuses of recorded steps.
 */
fn steps(details: &PersonaExecutionDetails) -> Vec<(String, PersonaActionStatus)> {
    details
        .actions
        .iter()
        .map(|action| {
            let kind = match &action.detail {
                PersonaActionDetail::Branch { outcome, .. } => format!("branch:{outcome}"),
                PersonaActionDetail::Open { .. } => "open".into(),
                PersonaActionDetail::Wait { .. } => "wait".into(),
                PersonaActionDetail::Click { .. } => "click".into(),
            };
            (kind, action.status)
        })
        .collect()
}

#[tokio::test]
/**
 * Owners create, read, update, enable, disable, and delete their own Personas.
 */
async fn users_manage_their_own_personas() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let member = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let caller = application.authorized(&session(&member));
    let created = caller
        .create_persona(request("  Alice@Example.TEST ", "open;"))
        .await
        .unwrap();
    assert_eq!(created.persona.address, "alice@example.test");
    assert_eq!(created.persona.owner_identifier, member.identifier);
    assert_eq!(created.address_access, AddressAccess::Confirmed);
    assert!(created.last_execution.is_none());
    let identifier = created.persona.identifier;
    assert_eq!(caller.list_personas().await.unwrap().len(), 1);
    let updated = caller
        .update_persona(identifier, request("alice@example.test", "wait 1s; open;"))
        .await
        .unwrap();
    assert_eq!(updated.persona.revision, 2);
    assert!(
        !caller
            .set_persona_enabled(identifier, false)
            .await
            .unwrap()
            .persona
            .enabled
    );
    assert!(
        caller
            .set_persona_enabled(identifier, true)
            .await
            .unwrap()
            .persona
            .enabled
    );
    caller.delete_persona(identifier).await.unwrap();
    assert!(matches!(
        caller.get_persona(identifier).await,
        Err(ApplicationError::NotFound)
    ));
}

#[tokio::test]
/**
 * Missing icon fields remain compatible and icon edits allow interrupted executions to resume.
 */
async fn persona_icons_preserve_execution_state() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let member = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let caller = application.authorized(&session(&member));
    let mut definition: PersonaRequest = serde_json::from_value(serde_json::json!({
        "name": "Alice", "address": "alice@example.test", "script": "open;"
    }))
    .unwrap();
    assert_eq!(definition.icon, None);
    definition.icon = Some("unknown-future-icon".into());
    let created = caller
        .create_persona(definition.clone())
        .await
        .unwrap()
        .persona;
    assert_eq!(created.icon, definition.icon);
    let mut legacy = serde_json::to_value(&created).unwrap();
    legacy.as_object_mut().unwrap().remove("icon");
    assert_eq!(
        serde_json::from_value::<sandpost_core::Persona>(legacy)
            .unwrap()
            .icon,
        None
    );

    deliver(&application, &storage, &["alice@example.test"], "Icon", "").await;
    let pending = storage
        .list_persona_executions_in_status(PersonaExecutionStatus::Pending, None, 1)
        .await
        .unwrap()
        .remove(0);
    let started = storage
        .start_persona_execution(
            pending.identifier,
            crate::credentials::current_unix_seconds(),
            created.revision,
        )
        .await
        .unwrap()
        .unwrap();
    definition.icon = Some("bug".into());
    let updated = caller
        .update_persona(created.identifier, definition.clone())
        .await
        .unwrap()
        .persona;
    assert_eq!(updated.icon, definition.icon);
    assert_eq!(updated.revision, created.revision);
    assert_eq!(
        storage
            .get_persona_execution(started.identifier)
            .await
            .unwrap()
            .unwrap()
            .0,
        started
    );
    let dispatcher = Dispatcher::start(&application);
    let resumed = finished(&application, &member, created.identifier).await;
    dispatcher.stop().await;
    assert_eq!(resumed.execution.identifier, started.identifier);
    assert_eq!(resumed.execution.status, PersonaExecutionStatus::Completed);
    assert_eq!(resumed.execution.persona_revision, Some(created.revision));

    definition.name = "Renamed".into();
    for expected_revision in [created.revision + 1, created.revision + 2] {
        let updated = caller
            .update_persona(created.identifier, definition.clone())
            .await
            .unwrap();
        assert_eq!(updated.persona.revision, expected_revision);
    }

    let mut invalid = definition.clone();
    invalid.icon = Some("BadIcon".into());
    assert!(matches!(
        caller.update_persona(created.identifier, invalid).await,
        Err(ApplicationError::InvalidInput(_))
    ));
    for enabled in [false, true] {
        let toggled = caller
            .set_persona_enabled(created.identifier, enabled)
            .await
            .unwrap()
            .persona;
        assert_eq!(toggled.icon, definition.icon);
        assert_eq!(toggled.enabled, enabled);
    }
}

#[tokio::test]
/**
 * Other users cannot see or change a Persona; Owners administer it but cannot edit its script.
 */
async fn personas_are_private_to_their_owner() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let alice = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let bob = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let admin = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let persona = application
        .authorized(&session(&alice))
        .create_persona(request("alice@example.test", "open;"))
        .await
        .unwrap()
        .persona
        .identifier;
    let intruder = application.authorized(&session(&bob));
    assert!(intruder.list_personas().await.unwrap().is_empty());
    assert!(matches!(
        intruder.get_persona(persona).await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        intruder
            .update_persona(persona, request("x@example.test", "open;"))
            .await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        intruder.set_persona_enabled(persona, false).await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        intruder.delete_persona(persona).await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        intruder.list_persona_executions(persona, None, 10).await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        intruder.list_all_personas().await,
        Err(ApplicationError::Forbidden)
    ));
    assert!(matches!(
        intruder
            .query_persona_messages(persona, QueryOptions::default())
            .await,
        Err(ApplicationError::NotFound)
    ));

    let administrator = application.authorized(&session(&admin));
    assert_eq!(administrator.list_all_personas().await.unwrap().len(), 1);
    assert!(administrator.list_personas().await.unwrap().is_empty());
    assert_eq!(
        administrator
            .get_persona(persona)
            .await
            .unwrap()
            .persona
            .owner_identifier,
        alice.identifier
    );
    assert!(matches!(
        administrator
            .update_persona(persona, request("alice@example.test", "wait 1s;"))
            .await,
        Err(ApplicationError::Forbidden)
    ));
    assert!(
        !administrator
            .set_persona_enabled(persona, false)
            .await
            .unwrap()
            .persona
            .enabled
    );
    administrator.delete_persona(persona).await.unwrap();
}

#[tokio::test]
/**
 * Viewers cannot create or change Personas.
 */
async fn viewers_cannot_manage_personas() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let viewer = user(&storage, GlobalRole::Viewer, MailAccess::All).await;
    let caller = application.authorized(&session(&viewer));
    assert!(matches!(
        caller
            .create_persona(request("v@example.test", "open;"))
            .await,
        Err(ApplicationError::Forbidden)
    ));
    assert!(caller.list_personas().await.unwrap().is_empty());
    assert!(caller.validate_persona_script("open;").await.unwrap().valid);
}

#[tokio::test]
/**
 * An address used by any Persona is refused without revealing who uses it.
 */
async fn addresses_are_globally_unique() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let alice = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let bob = user(&storage, GlobalRole::Member, MailAccess::All).await;
    application
        .authorized(&session(&alice))
        .create_persona(request("franco@ecommerce.it", "open;"))
        .await
        .unwrap();
    let error = application
        .authorized(&session(&bob))
        .create_persona(request("FRANCO@ecommerce.it", "open;"))
        .await
        .unwrap_err();
    let ApplicationError::InUse(message) = error else {
        panic!("expected a conflict, got {error:?}");
    };
    assert!(message.contains("already used by a persona"));
    assert!(!message.contains(&alice.email) && !message.contains("Alice"));
    let own = application
        .authorized(&session(&bob))
        .create_persona(request("bob@ecommerce.it", "open;"))
        .await
        .unwrap();
    assert!(matches!(
        application
            .authorized(&session(&bob))
            .update_persona(
                own.persona.identifier,
                request("franco@ecommerce.it", "open;")
            )
            .await,
        Err(ApplicationError::InUse(_))
    ));
}

#[tokio::test]
/**
 * Invalid names, addresses, and scripts are rejected with actionable messages.
 */
async fn invalid_definitions_are_rejected() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let member = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let caller = application.authorized(&session(&member));
    for address in [
        "not-an-address",
        "a@",
        "@b.test",
        "a b@c.test",
        "ünï@code.test",
    ] {
        let error = caller
            .create_persona(request(address, "open;"))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, ApplicationError::InvalidInput(message) if message.contains("valid ASCII email address")),
            "{address}: {error:?}"
        );
    }
    let error = caller
        .create_persona(request("a@b.test", "open;\n  jump;"))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ApplicationError::InvalidInput(message) if message.contains("line 2, column 3") && message.contains("unknown instruction `jump`")),
        "{error:?}"
    );
    let mut nameless = request("a@b.test", "open;");
    nameless.name = "   ".into();
    assert!(matches!(
        caller.create_persona(nameless).await,
        Err(ApplicationError::InvalidInput(_))
    ));
    let mut invalid_icon = request("icon@b.test", "open;");
    invalid_icon.icon = Some("VenetianMask".into());
    assert!(matches!(
        caller.create_persona(invalid_icon).await,
        Err(ApplicationError::InvalidInput(message)) if message.contains("icon must be a lowercase kebab-case")
    ));
    let validation = caller
        .validate_persona_script("wait 5s;\nclick link(\"a[\");")
        .await
        .unwrap();
    assert!(!validation.valid);
    let diagnostic = validation.error.unwrap();
    assert_eq!((diagnostic.line, diagnostic.column), (2, 12));
    assert!(diagnostic.message.contains("invalid CSS selector"));
    let valid = caller
        .validate_persona_script("wait 5s; if (true) { wait 2s; }")
        .await
        .unwrap();
    assert_eq!(
        (valid.statements, valid.longest_wait_milliseconds),
        (Some(3), Some(7000))
    );
}

#[test]
/**
 * Address previews decide only what the address can decide and leave the rest per message.
 */
fn address_access_uses_three_valued_logic() {
    let classify = |filter: &str, address: &str| {
        classify_address(compile(filter).unwrap().expression(), address)
    };
    assert_eq!(classify("true", "a@b.test"), AddressAccess::Confirmed);
    assert_eq!(classify("false", "a@b.test"), AddressAccess::Denied);
    let domain_scope = r#"envelope.to.domain == "ecommerce.it""#;
    assert_eq!(
        classify(domain_scope, "franco@ecommerce.it"),
        AddressAccess::Confirmed
    );
    assert_eq!(
        classify(domain_scope, "franco@other-team.it"),
        AddressAccess::PerMessage
    );
    assert_eq!(
        classify(
            r#"envelope.to.address matches "*@ECOMMERCE.it""#,
            "franco@ecommerce.it"
        ),
        AddressAccess::Confirmed
    );
    assert_eq!(
        classify(r#"subject contains "x""#, "a@b.test"),
        AddressAccess::PerMessage
    );
    assert_eq!(
        classify(r#"to.domain == "b.test""#, "a@b.test"),
        AddressAccess::PerMessage,
        "MIME recipients are not implied by the envelope"
    );
    assert_eq!(
        classify(r#"not (envelope.to.domain == "b.test")"#, "a@b.test"),
        AddressAccess::Denied
    );
    assert_eq!(
        classify(
            r#"envelope.to.domain == "b.test" and subject contains "x""#,
            "a@b.test"
        ),
        AddressAccess::PerMessage
    );
    assert_eq!(
        classify(
            r#"envelope.to.domain == "c.test" or envelope.to.domain == "b.test""#,
            "a@b.test"
        ),
        AddressAccess::Confirmed
    );
}

#[tokio::test]
/**
 * Creation refuses addresses the owner provably cannot reach and allows undecidable ones.
 */
async fn creation_checks_address_against_scopes() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let unassigned = user(&storage, GlobalRole::Member, MailAccess::Scoped).await;
    let error = application
        .authorized(&session(&unassigned))
        .create_persona(request("a@b.test", "open;"))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ApplicationError::InvalidInput(message) if message.contains("outside your mail access"))
    );
    let scoped = user(&storage, GlobalRole::Member, MailAccess::Scoped).await;
    assign_scope(&storage, &scoped, r#"envelope.to.domain == "ecommerce.it""#).await;
    let caller = application.authorized(&session(&scoped));
    let inside = caller
        .create_persona(request("franco@ecommerce.it", "open;"))
        .await
        .unwrap();
    assert_eq!(inside.address_access, AddressAccess::Confirmed);
    let outside = caller
        .create_persona(request("franco@other-team.it", "open;"))
        .await
        .unwrap();
    assert_eq!(outside.address_access, AddressAccess::PerMessage);
}

#[tokio::test]
/**
 * A script runs in order: it waits, opens (loading remote images), branches, and clicks,
 * without changing the owner's read state.
 */
async fn executions_open_wait_branch_and_click_in_order() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request(
            "alice@example.test",
            "wait 50ms; open; wait 50ms; if (subject contains \"Verify\") { click link(\"a.verify\"); } else { click link(text: \"Other\"); }",
        ))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    let html = format!(
        r#"<p>Hello</p><img src="{}" width="1" height="1"><img src="cid:logo"><a class="verify" href="{}">Verify</a><a href="{}">Other</a>"#,
        server.url("/pixel.gif?u=1"),
        server.url("/verify?token=abc"),
        server.url("/other")
    );
    let message = deliver(
        &application,
        &storage,
        &["Alice@example.test"],
        "Please Verify",
        &html,
    )
    .await;
    let details = finished(&application, &owner, persona).await;
    dispatcher.stop().await;
    assert_eq!(
        details.execution.status,
        PersonaExecutionStatus::Completed,
        "{details:?}"
    );
    assert_eq!(details.execution.message_identifier, message);
    assert_eq!(details.subject, "Please Verify");
    assert_eq!(
        steps(&details),
        [
            ("wait".into(), PersonaActionStatus::Succeeded),
            ("open".into(), PersonaActionStatus::Succeeded),
            ("wait".into(), PersonaActionStatus::Succeeded),
            ("branch:true".into(), PersonaActionStatus::Succeeded),
            ("click".into(), PersonaActionStatus::Succeeded),
        ]
    );
    assert_eq!(server.paths(), ["/pixel.gif?u=1", "/verify?token=abc"]);
    let PersonaActionDetail::Open {
        images,
        embedded_images,
        ..
    } = &details.actions[1].detail
    else {
        panic!("expected open");
    };
    assert_eq!((images.len(), *embedded_images), (1, 1));
    assert_eq!(images[0].status_code, Some(200));
    assert_eq!(images[0].received_bytes, 4);
    let PersonaActionDetail::Click {
        url, request, text, ..
    } = &details.actions[4].detail
    else {
        panic!("expected click");
    };
    assert_eq!(
        url.as_deref(),
        Some(server.url("/verify?token=abc").as_str())
    );
    assert_eq!(text.as_deref(), Some("Verify"));
    assert_eq!(request.as_ref().unwrap().status_code, Some(200));
    assert_eq!(details.actions[4].line, 1);
    let summaries = application
        .authorized(&session(&owner))
        .query_persona_messages(persona, QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert!(
        !summaries[0].seen,
        "simulated opens never change the owner's read state"
    );
}

#[tokio::test]
/**
 * Messages the owner cannot read are never acted on and leave no visible history; messages the
 * owner can read through another recipient are acted on.
 */
async fn executions_require_the_owner_to_read_the_message() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::Scoped).await;
    assign_scope(&storage, &owner, r#"envelope.to.domain == "ecommerce.it""#).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request("franco@other-team.it", "open;"))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    let html = format!(r#"<img src="{}">"#, server.url("/hidden.gif"));
    deliver(
        &application,
        &storage,
        &["franco@other-team.it"],
        "Secret",
        &html,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pending = storage
        .list_persona_executions_in_status(PersonaExecutionStatus::Pending, None, 10)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "the unauthorized execution was discarded"
    );
    assert!(
        server.paths().is_empty(),
        "nothing was requested for unreadable mail"
    );
    assert!(
        storage
            .list_persona_executions(
                persona,
                owner.identifier,
                &sandpost_query::Expression::True,
                None,
                10
            )
            .await
            .unwrap()
            .is_empty(),
        "no history row exists for the unreadable message"
    );

    let shared_html = format!(r#"<img src="{}">"#, server.url("/shared.gif"));
    deliver(
        &application,
        &storage,
        &["franco@other-team.it", "team@ecommerce.it"],
        "Shared",
        &shared_html,
    )
    .await;
    let details = finished(&application, &owner, persona).await;
    dispatcher.stop().await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Completed);
    assert_eq!(details.subject, "Shared");
    assert_eq!(server.paths(), ["/shared.gif"]);
}

#[tokio::test]
/**
 * Losing access during a wait cancels the execution before its next action and hides it.
 */
async fn revoking_access_prevents_further_actions() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::Scoped).await;
    let scope = assign_scope(&storage, &owner, r#"envelope.to.domain == "ecommerce.it""#).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request(
            "franco@ecommerce.it",
            "wait 400ms; click link(\"a\");",
        ))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    let html = format!(r#"<a href="{}">Go</a>"#, server.url("/go"));
    let message = deliver(
        &application,
        &storage,
        &["franco@ecommerce.it"],
        "Go",
        &html,
    )
    .await;
    let execution = running(&storage).await;
    storage
        .remove_user_from_scope(owner.identifier, scope)
        .await
        .unwrap();
    for _ in 0..200 {
        let (record, _) = storage
            .get_persona_execution(execution.identifier)
            .await
            .unwrap()
            .unwrap();
        if record.status.is_terminal() {
            assert_eq!(record.status, PersonaExecutionStatus::Cancelled);
            assert_eq!(
                record.error_message.as_deref(),
                Some("the Persona's owner can no longer access this message")
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    dispatcher.stop().await;
    assert!(server.paths().is_empty(), "the click never happened");
    let reader = application.authorized(&session(&owner));
    assert!(
        reader
            .list_persona_executions(persona, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        reader
            .get_persona_execution(persona, execution.identifier)
            .await,
        Err(ApplicationError::NotFound)
    ));
    assert!(
        reader
            .query_persona_messages(persona, QueryOptions::default())
            .await
            .unwrap()
            .iter()
            .all(|summary| summary.identifier != message)
    );
}

#[tokio::test]
/**
 * Disabling a Persona cancels its running execution promptly; deleting it keeps its mail.
 */
async fn disabling_and_deleting_cancel_executions() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let caller = application.authorized(&session(&owner));
    let persona = caller
        .create_persona(request("alice@example.test", "wait 10m; open;"))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    let message = deliver(&application, &storage, &["alice@example.test"], "Hi", "").await;
    running(&storage).await;
    caller.set_persona_enabled(persona, false).await.unwrap();
    let details = finished(&application, &owner, persona).await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Cancelled);
    assert_eq!(
        details.execution.error_message.as_deref(),
        Some("the Persona was disabled")
    );
    assert_eq!(details.actions[0].status, PersonaActionStatus::Cancelled);

    caller.set_persona_enabled(persona, true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        caller
            .list_persona_executions(persona, None, 10)
            .await
            .unwrap()
            .len(),
        1,
        "enabling never replays earlier mail"
    );
    deliver(&application, &storage, &["alice@example.test"], "Again", "").await;
    running(&storage).await;
    caller.delete_persona(persona).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    dispatcher.stop().await;
    assert_eq!(application.personas.active_count(), 0);
    assert!(
        storage.get_message(message).await.unwrap().is_some(),
        "mail is kept"
    );
}

#[tokio::test]
/**
 * Missing links, malformed HTML, and HTTP failures fail clicks clearly; failed images do not
 * fail an open.
 */
async fn action_failures_are_reported() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let caller = application.authorized(&session(&owner));
    let missing = caller
        .create_persona(request(
            "missing@example.test",
            "open; click link(text: \"Nope\"); open;",
        ))
        .await
        .unwrap()
        .persona
        .identifier;
    let closed = caller
        .create_persona(request("closed@example.test", "click link(\"a\");"))
        .await
        .unwrap()
        .persona
        .identifier;
    let scheme = caller
        .create_persona(request("scheme@example.test", "click link(\"a.mail\");"))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    let malformed = format!(
        "<table><tr><td><img src='{}'><a href='{}'>Yes</a><p><b>unclosed",
        server.url("/missing"),
        server.url("/yes")
    );
    deliver(
        &application,
        &storage,
        &["missing@example.test"],
        "M",
        &malformed,
    )
    .await;
    let unused = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_url = format!("http://{}/gone", unused.local_addr().unwrap());
    drop(unused);
    deliver(
        &application,
        &storage,
        &["closed@example.test"],
        "C",
        &format!("<a href='{closed_url}'>x</a>"),
    )
    .await;
    deliver(
        &application,
        &storage,
        &["scheme@example.test"],
        "S",
        "<a class='mail' href='mailto:a@b.test'>Mail</a>",
    )
    .await;

    let details = finished(&application, &owner, missing).await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Failed);
    assert_eq!(
        steps(&details),
        [
            ("open".into(), PersonaActionStatus::Succeeded),
            ("click".into(), PersonaActionStatus::Failed)
        ]
    );
    let PersonaActionDetail::Open { images, .. } = &details.actions[0].detail else {
        panic!("expected open");
    };
    assert_eq!(images[0].status_code, Some(404));
    let error = details.execution.error_message.unwrap();
    assert!(
        error.contains("none of the message's 1 link(s) has matching text"),
        "{error}"
    );

    let details = finished(&application, &owner, closed).await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Failed);
    let PersonaActionDetail::Click { request, .. } = &details.actions[0].detail else {
        panic!("expected click");
    };
    assert_eq!(
        request.as_ref().unwrap().error_kind,
        Some(sandpost_core::LinkCheckErrorKind::Connection)
    );
    assert!(
        details
            .execution
            .error_message
            .unwrap()
            .contains("the click request")
    );

    let details = finished(&application, &owner, scheme).await;
    dispatcher.stop().await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Failed);
    assert!(
        details
            .execution
            .error_message
            .unwrap()
            .contains("mailto: scheme is not supported")
    );
    assert_eq!(server.paths(), ["/missing"]);
}

#[tokio::test]
/**
 * The execution time limit fails an execution stuck in a request.
 */
async fn executions_time_out() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(
        storage.clone(),
        PersonaSettings {
            execution_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(30),
            ..settings()
        },
    )
    .await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request("slow@example.test", "wait 1s; click link(\"a\");"))
        .await
        .unwrap()
        .persona
        .identifier;
    let error = application
        .authorized(&session(&owner))
        .validate_persona_script("wait 2s;")
        .await
        .unwrap();
    assert!(
        !error.valid,
        "waits may use at most three quarters of the time limit"
    );
    let dispatcher = Dispatcher::start(&application);
    deliver(
        &application,
        &storage,
        &["slow@example.test"],
        "S",
        &format!("<a href='{}'>x</a>", server.url("/hang")),
    )
    .await;
    let details = finished(&application, &owner, persona).await;
    dispatcher.stop().await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Failed);
    assert!(
        details
            .execution
            .error_message
            .unwrap()
            .contains("time limit of 2 seconds")
    );
    assert_eq!(details.actions[1].status, PersonaActionStatus::Failed);
}

#[tokio::test]
/**
 * No more executions than configured run at once, and every one eventually completes.
 */
async fn concurrency_is_bounded() {
    let storage = storage().await;
    let application = application(
        storage.clone(),
        PersonaSettings {
            concurrent_executions: 2,
            ..settings()
        },
    )
    .await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request("busy@example.test", "wait 150ms;"))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    for index in 0..6 {
        deliver(
            &application,
            &storage,
            &["busy@example.test"],
            &format!("{index}"),
            "",
        )
        .await;
    }
    let mut maximum_running = 0;
    for _ in 0..200 {
        let running = storage
            .list_persona_executions_in_status(PersonaExecutionStatus::Running, None, 100)
            .await
            .unwrap()
            .len();
        maximum_running = maximum_running.max(running);
        assert!(application.personas.active_count() <= 2);
        let summaries = application
            .authorized(&session(&owner))
            .list_persona_executions(persona, None, 100)
            .await
            .unwrap();
        if summaries.len() == 6
            && summaries
                .iter()
                .all(|summary| summary.execution.status == PersonaExecutionStatus::Completed)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    dispatcher.stop().await;
    assert!(
        maximum_running <= 2,
        "{maximum_running} executions ran at once"
    );
    let summaries = application
        .authorized(&session(&owner))
        .list_persona_executions(persona, None, 100)
        .await
        .unwrap();
    assert!(
        summaries
            .iter()
            .all(|summary| summary.execution.status == PersonaExecutionStatus::Completed)
    );
}

#[tokio::test]
/**
 * A restart resumes a waiting execution without repeating finished requests.
 */
async fn restart_resumes_without_repeating_requests() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let first = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let persona = first
        .authorized(&session(&owner))
        .create_persona(request(
            "restart@example.test",
            "click link(\"a\"); wait 400ms; open;",
        ))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&first);
    let html = format!(
        "<a href='{}'>x</a><img src='{}'>",
        server.url("/click"),
        server.url("/image")
    );
    deliver(&first, &storage, &["restart@example.test"], "R", &html).await;
    for _ in 0..200 {
        if server.paths().contains(&"/click".to_owned()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    dispatcher.stop().await;
    let interrupted = running(&storage).await;
    assert_eq!(
        interrupted.status,
        PersonaExecutionStatus::Running,
        "shutdown leaves it resumable"
    );

    let second = application(storage.clone(), settings()).await;
    let dispatcher = Dispatcher::start(&second);
    let details = finished(&second, &owner, persona).await;
    dispatcher.stop().await;
    assert_eq!(
        details.execution.status,
        PersonaExecutionStatus::Completed,
        "{details:?}"
    );
    assert_eq!(
        server.paths(),
        ["/click", "/image"],
        "the click was not repeated"
    );
    assert_eq!(
        steps(&details),
        [
            ("click".into(), PersonaActionStatus::Succeeded),
            ("wait".into(), PersonaActionStatus::Succeeded),
            ("open".into(), PersonaActionStatus::Succeeded)
        ]
    );
}

#[tokio::test]
/**
 * A request interrupted by a crash is marked interrupted and never repeated.
 */
async fn interrupted_requests_are_not_repeated() {
    interrupted_request_scenario(crate::credentials::current_unix_seconds()).await;
}

#[tokio::test]
/**
 * The interrupted request is reported as interrupted even when the time limit also elapsed
 * while the process was down.
 */
async fn interrupted_requests_are_reported_after_the_deadline() {
    interrupted_request_scenario(1).await;
}

/**
 * Seed a running execution whose click had started at `started_at`, then resume it.
 */
async fn interrupted_request_scenario(started_at: i64) {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request("crash@example.test", "click link(\"a\"); open;"))
        .await
        .unwrap()
        .persona
        .identifier;
    deliver(
        &application,
        &storage,
        &["crash@example.test"],
        "C",
        &format!("<a href='{}'>x</a>", server.url("/once")),
    )
    .await;
    let pending = storage
        .list_persona_executions_in_status(PersonaExecutionStatus::Pending, None, 1)
        .await
        .unwrap()
        .remove(0);
    let revision = storage
        .get_persona(persona)
        .await
        .unwrap()
        .unwrap()
        .revision;
    let now = started_at;
    storage
        .start_persona_execution(pending.identifier, now, revision)
        .await
        .unwrap();
    storage
        .record_persona_execution_action(
            pending.identifier,
            &PersonaExecutionAction {
                step: 0,
                status: PersonaActionStatus::Started,
                line: 1,
                column: 1,
                started_at: now,
                duration_milliseconds: None,
                error_message: None,
                detail: PersonaActionDetail::Click {
                    selector: "link(\"a\")".into(),
                    href: None,
                    text: None,
                    url: Some(server.url("/once")),
                    request: None,
                },
            },
        )
        .await
        .unwrap();
    let dispatcher = Dispatcher::start(&application);
    let details = finished(&application, &owner, persona).await;
    dispatcher.stop().await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Failed);
    assert_eq!(details.actions[0].status, PersonaActionStatus::Interrupted);
    assert!(
        details
            .execution
            .error_message
            .unwrap()
            .contains("not repeated")
    );
    assert!(server.paths().is_empty());
}

#[tokio::test]
/**
 * Repeated notifications and repeated recipients never run a Persona twice for one message.
 */
async fn duplicate_message_events_run_once() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request("dup@example.test", "click link(\"a\");"))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    let message = deliver(
        &application,
        &storage,
        &["dup@example.test", "DUP@example.test"],
        "D",
        &format!("<a href='{}'>x</a>", server.url("/one")),
    )
    .await;
    for _ in 0..5 {
        let _ = application.publish_message(MessageEvent {
            identifier: message,
            sequence: sandpost_core::MessageSequence(1),
        });
    }
    finished(&application, &owner, persona).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    dispatcher.stop().await;
    assert_eq!(server.paths(), ["/one"]);
    assert_eq!(
        application
            .authorized(&session(&owner))
            .list_persona_executions(persona, None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
/**
 * The Persona view lists only readable mail addressed to it; history follows the reader's
 * access, even for Owners.
 */
async fn persona_views_and_history_follow_reader_access() {
    let storage = storage().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let admin = user(&storage, GlobalRole::Owner, MailAccess::Scoped).await;
    assign_scope(&storage, &admin, r#"subject == "Admin can read""#).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request("view@example.test", "open;"))
        .await
        .unwrap()
        .persona
        .identifier;
    let dispatcher = Dispatcher::start(&application);
    deliver(
        &application,
        &storage,
        &["view@example.test"],
        "Admin can read",
        "",
    )
    .await;
    deliver(
        &application,
        &storage,
        &["view@example.test"],
        "Private",
        "",
    )
    .await;
    deliver(
        &application,
        &storage,
        &["someone@example.test"],
        "Unrelated",
        "",
    )
    .await;
    for _ in 0..200 {
        let summaries = application
            .authorized(&session(&owner))
            .list_persona_executions(persona, None, 10)
            .await
            .unwrap();
        if summaries.len() == 2
            && summaries
                .iter()
                .all(|summary| summary.execution.status.is_terminal())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    dispatcher.stop().await;
    let subjects = |summaries: Vec<crate::MessageSummary>| {
        summaries
            .into_iter()
            .map(|summary| summary.subject)
            .collect::<Vec<_>>()
    };
    let reader = application.authorized(&session(&owner));
    assert_eq!(
        subjects(
            reader
                .query_persona_messages(persona, QueryOptions::default())
                .await
                .unwrap()
        ),
        ["Private", "Admin can read"]
    );
    assert_eq!(
        subjects(
            reader
                .query_persona_messages(
                    persona,
                    QueryOptions {
                        query: Some("subject contains \"Admin\"".into()),
                        ..QueryOptions::default()
                    }
                )
                .await
                .unwrap()
        ),
        ["Admin can read"]
    );
    let administrator = application.authorized(&session(&admin));
    assert_eq!(
        subjects(
            administrator
                .query_persona_messages(persona, QueryOptions::default())
                .await
                .unwrap()
        ),
        ["Admin can read"]
    );
    let history: Vec<_> = administrator
        .list_persona_executions(persona, None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.subject)
        .collect();
    assert_eq!(history, ["Admin can read"]);
    let details = administrator.get_persona(persona).await.unwrap();
    assert_eq!(details.last_execution.unwrap().subject, "Admin can read");
}

#[tokio::test]
/**
 * Demoting the owner to Viewer stops their Personas from acting.
 */
async fn viewer_owners_cannot_run_personas() {
    let storage = storage().await;
    let server = MockServer::start().await;
    let application = application(storage.clone(), settings()).await;
    let owner = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let persona = application
        .authorized(&session(&owner))
        .create_persona(request("demoted@example.test", "open;"))
        .await
        .unwrap()
        .persona
        .identifier;
    storage
        .update_user(
            owner.identifier,
            &UpdateUser {
                name: owner.name.clone(),
                email: owner.email.clone(),
                password_hash: owner.password_hash.clone(),
                global_role: GlobalRole::Viewer,
                mail_access: MailAccess::All,
                personal_filter: None,
            },
        )
        .await
        .unwrap();
    let dispatcher = Dispatcher::start(&application);
    deliver(
        &application,
        &storage,
        &["demoted@example.test"],
        "V",
        &format!("<img src='{}'>", server.url("/x")),
    )
    .await;
    let details = finished(&application, &owner, persona).await;
    dispatcher.stop().await;
    assert_eq!(details.execution.status, PersonaExecutionStatus::Cancelled);
    assert_eq!(
        details.execution.error_message.as_deref(),
        Some("the Persona's owner is a read-only Viewer")
    );
    assert!(server.paths().is_empty());
}
