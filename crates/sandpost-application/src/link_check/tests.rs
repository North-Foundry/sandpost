//! Link check access, shared results, rechecks, concurrency, and bulk requests.
use crate::{
    Application, ApplicationError, BulkLinkCheckRequest, LinkCheckReport, LinkCheckSettings,
    MessageEvent, SessionUser,
};
use sandpost_core::{
    GlobalRole, LinkCheckErrorKind, LinkOutcome, MailAccess, Mailbox, Message, MessageFacts,
    MessageIdentifier, User, UserIdentifier,
};
use sandpost_storage::{NewUser, Storage};
use sandpost_storage_sqlite::SqliteStorage;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// A loopback HTTP server answering every request with an adjustable status after a delay.
struct StatusServer {
    address: SocketAddr,
    status: Arc<AtomicU16>,
    requests: Arc<Mutex<Vec<String>>>,
    delay: Arc<Mutex<Duration>>,
    task: tokio::task::JoinHandle<()>,
}

impl StatusServer {
    /**
     * Start a server answering with the supplied status immediately.
     */
    async fn start(status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let status = Arc::new(AtomicU16::new(status));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let delay = Arc::new(Mutex::new(Duration::ZERO));
        let task = tokio::spawn({
            let status = status.clone();
            let requests = requests.clone();
            let delay = delay.clone();
            async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let status = status.clone();
                    let requests = requests.clone();
                    let delay = *delay.lock().unwrap();
                    tokio::spawn(async move {
                        let mut head = Vec::new();
                        let mut buffer = [0_u8; 512];
                        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                            match stream.read(&mut buffer).await {
                                Ok(0) | Err(_) => return,
                                Ok(read) => head.extend_from_slice(&buffer[..read]),
                            }
                        }
                        let line = String::from_utf8_lossy(&head)
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_owned();
                        requests.lock().unwrap().push(line);
                        tokio::time::sleep(delay).await;
                        let code = status.load(Ordering::SeqCst);
                        let response = format!(
                            "HTTP/1.1 {code} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });
        Self {
            address,
            status,
            requests,
            delay,
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
     * Return how many requests the server has received.
     */
    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for StatusServer {
    /**
     * Stop accepting connections when the test ends.
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
 * Create a user with the supplied role and mail access.
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
 * Capture a message whose HTML and text bodies contain the supplied links.
 */
async fn message_with_links(
    storage: &Arc<dyn Storage>,
    html: &[String],
    text: &str,
) -> MessageIdentifier {
    let identifier = MessageIdentifier::new();
    let markup: String = html
        .iter()
        .map(|url| format!(r#"<a href="{url}">link</a>"#))
        .collect();
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: "Links".into(),
                markup_body: markup,
                text: text.into(),
                envelope_to: vec![Mailbox {
                    address: "team@example.test".into(),
                    domain: "example.test".into(),
                }],
                ..MessageFacts::default()
            },
            raw_message: Vec::new(),
            attachments: Vec::new(),
        })
        .await
        .unwrap();
    identifier
}

/**
 * Build an application allowed to verify the loopback test servers.
 */
async fn application(storage: Arc<dyn Storage>) -> Application {
    let (events, _) = broadcast::channel::<MessageEvent>(16);
    Application::new(storage, events)
        .await
        .unwrap()
        .with_link_check_settings(LinkCheckSettings {
            allow_internal_addresses: true,
            link_timeout: Duration::from_secs(5),
            ..LinkCheckSettings::default()
        })
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
 * Poll a message's state until no check is running.
 */
async fn completed(
    application: &Application,
    user: &User,
    message: MessageIdentifier,
) -> LinkCheckReport {
    let reader = application.authorized(&session(user));
    for _ in 0..200 {
        let report = reader.get_link_check(message).await.unwrap();
        if !report.running {
            return report;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the link check did not complete");
}

/**
 * Every role can start single and bulk checks of messages it can read.
 */
#[tokio::test]
async fn every_role_can_start_single_and_bulk_checks() {
    let storage = storage().await;
    let server = StatusServer::start(200).await;
    let message = message_with_links(&storage, &[server.url("/")], "").await;
    let application = application(storage.clone()).await;

    for role in [GlobalRole::Owner, GlobalRole::Member, GlobalRole::Viewer] {
        let reader = user(&storage, role, MailAccess::All).await;
        let caller = application.authorized(&session(&reader));
        assert!(caller.start_link_check(message).await.unwrap().running);
        assert_eq!(
            completed(&application, &reader, message)
                .await
                .summary
                .healthy,
            1
        );
        let bulk = caller
            .start_bulk_link_checks(BulkLinkCheckRequest {
                identifiers: vec![message],
            })
            .await
            .unwrap();
        assert_eq!(
            (bulk.started, bulk.already_running, bulk.skipped),
            (1, 0, 0)
        );
        completed(&application, &reader, message).await;
    }
    assert_eq!(server.request_count(), 6);
}

/**
 * One user's check is stored once and served to every authorized reader without new requests.
 */
#[tokio::test]
async fn results_are_shared_and_reading_never_checks_again() {
    let storage = storage().await;
    let healthy = StatusServer::start(200).await;
    let broken = StatusServer::start(404).await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let reader = user(&storage, GlobalRole::Viewer, MailAccess::All).await;
    let outsider = user(&storage, GlobalRole::Member, MailAccess::Scoped).await;
    let message = message_with_links(
        &storage,
        &[
            healthy.url("/a#one"),
            broken.url("/missing"),
            healthy.url("/a#two"),
        ],
        &format!("Plain copy: {}", healthy.url("/text-only")),
    )
    .await;
    let application = application(storage).await;
    application
        .authorized(&session(&owner))
        .start_link_check(message)
        .await
        .unwrap();
    let first = completed(&application, &owner, message).await;
    assert_eq!(
        first
            .links
            .iter()
            .map(|link| link.url.clone())
            .collect::<Vec<_>>(),
        [
            healthy.url("/a"),
            broken.url("/missing"),
            healthy.url("/text-only")
        ]
    );
    assert_eq!(
        first.urls,
        [
            healthy.url("/a"),
            broken.url("/missing"),
            healthy.url("/text-only")
        ]
    );
    assert_eq!(first.summary.total, 3);
    assert_eq!(first.summary.healthy, 2);
    assert_eq!(first.summary.broken, 1);
    let missing = &first.links[1];
    assert_eq!(missing.outcome, LinkOutcome::Broken);
    assert_eq!(missing.status_code, Some(404));
    assert_eq!(missing.status_text, Some("Not Found"));
    assert!(first.checked_at.is_some());

    let requests = healthy.request_count() + broken.request_count();
    let shared = completed(&application, &reader, message).await;
    assert_eq!(shared.checked_at, first.checked_at);
    assert_eq!(shared.summary, first.summary);
    assert_eq!(healthy.request_count() + broken.request_count(), requests);

    let unauthorized = application.authorized(&session(&outsider));
    assert!(matches!(
        unauthorized.get_link_check(message).await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        unauthorized.start_link_check(message).await,
        Err(ApplicationError::NotFound)
    ));
    let bulk = unauthorized
        .start_bulk_link_checks(BulkLinkCheckRequest {
            identifiers: vec![message, MessageIdentifier::new()],
        })
        .await
        .unwrap();
    assert_eq!(
        (bulk.started, bulk.already_running, bulk.skipped),
        (0, 0, 2)
    );
}

/**
 * A recheck keeps the previous results readable until the replacement is stored.
 */
#[tokio::test]
async fn rechecks_replace_results_without_hiding_previous_ones() {
    let storage = storage().await;
    let server = StatusServer::start(503).await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let message = message_with_links(&storage, &[server.url("/status")], "").await;
    let application = application(storage).await;
    let caller = application.authorized(&session(&owner));
    caller.start_link_check(message).await.unwrap();
    let first = completed(&application, &owner, message).await;
    assert_eq!(first.links[0].outcome, LinkOutcome::ServerError);

    server.status.store(200, Ordering::SeqCst);
    *server.delay.lock().unwrap() = Duration::from_millis(300);
    let running = caller.start_link_check(message).await.unwrap();
    assert!(running.running);
    assert!(running.started_at.is_some());
    assert_eq!(running.checked_at, first.checked_at);
    assert_eq!(running.links[0].outcome, LinkOutcome::ServerError);

    let second = completed(&application, &owner, message).await;
    assert_eq!(second.links.len(), 1);
    assert_eq!(second.links[0].outcome, LinkOutcome::Healthy);
    assert_eq!(second.links[0].status_code, Some(200));
    assert!(second.checked_at >= first.checked_at);
}

/**
 * Simultaneous requests for one message join a single run instead of duplicating it.
 */
#[tokio::test]
async fn concurrent_requests_share_one_run() {
    let storage = storage().await;
    let server = StatusServer::start(200).await;
    *server.delay.lock().unwrap() = Duration::from_millis(200);
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let bulk_member = user(&storage, GlobalRole::Member, MailAccess::All).await;
    let message = message_with_links(&storage, &[server.url("/once")], "").await;
    let other = message_with_links(&storage, &[server.url("/other")], "").await;
    let application = application(storage).await;
    let owner_caller = application.authorized(&session(&owner));
    let (first, second) = tokio::join!(
        owner_caller.start_link_check(message),
        owner_caller.start_link_check(message)
    );
    assert!(first.unwrap().running && second.unwrap().running);
    let bulk = application
        .authorized(&session(&bulk_member))
        .start_bulk_link_checks(BulkLinkCheckRequest {
            identifiers: vec![message, other, other],
        })
        .await
        .unwrap();
    assert_eq!(
        (bulk.started, bulk.already_running, bulk.skipped),
        (1, 1, 1)
    );
    completed(&application, &owner, message).await;
    completed(&application, &owner, other).await;
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.contains(" /once "))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.contains(" /other "))
            .count(),
        1
    );
    let empty = application
        .authorized(&session(&bulk_member))
        .start_bulk_link_checks(BulkLinkCheckRequest {
            identifiers: Vec::new(),
        })
        .await;
    assert!(matches!(empty, Err(ApplicationError::InvalidInput(_))));
}

/**
 * Without an explicit opt-in, loopback links are reported without being requested.
 */
#[tokio::test]
async fn default_policy_refuses_internal_links() {
    let storage = storage().await;
    let server = StatusServer::start(200).await;
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let message = message_with_links(&storage, &[server.url("/private")], "").await;
    let (events, _) = broadcast::channel::<MessageEvent>(16);
    let application = Application::new(storage, events).await.unwrap();
    application
        .authorized(&session(&owner))
        .start_link_check(message)
        .await
        .unwrap();
    let report = completed(&application, &owner, message).await;
    assert_eq!(report.links[0].outcome, LinkOutcome::Inconclusive);
    assert_eq!(
        report.links[0].error_kind,
        Some(LinkCheckErrorKind::BlockedAddress)
    );
    assert_eq!(report.summary.inconclusive, 1);
    assert_eq!(server.request_count(), 0);
}

/**
 * Cancelled checks leave stored results untouched and refuse new runs.
 */
#[tokio::test]
async fn cancellation_stops_running_checks() {
    let storage = storage().await;
    let server = StatusServer::start(200).await;
    *server.delay.lock().unwrap() = Duration::from_secs(30);
    let owner = user(&storage, GlobalRole::Owner, MailAccess::All).await;
    let message = message_with_links(&storage, &[server.url("/slow")], "").await;
    let application = application(storage).await;
    let caller = application.authorized(&session(&owner));
    caller.start_link_check(message).await.unwrap();
    application.cancel_link_checks();
    let report = completed(&application, &owner, message).await;
    assert_eq!(report.checked_at, None);
    assert!(matches!(
        caller.start_link_check(message).await,
        Err(ApplicationError::Unavailable(_))
    ));
}
