//! IDLE conformance (RFC 2177): continuation, pushed EXISTS/EXPUNGE/FETCH updates, DONE,
//! keepalives, time limits, revocation, and multiple concurrent sessions.
mod support;

use sandpost_imap::{Flags, Limits, SystemFlag};
use std::time::Duration;
use support::{RunningServer, plain_message, test_limits};

/// New mail, removals, and flag changes are pushed while idling; DONE ends with a tagged OK.
#[tokio::test]
async fn idle_pushes_changes_until_done() {
    let server = RunningServer::start(test_limits()).await;
    server.backend.deliver("INBOX", &plain_message("one"), 0);
    let mut client = server.selected("INBOX").await;
    client.send(b"i1 IDLE\r\n").await;
    assert_eq!(client.read_response().await.unwrap(), "+ idling\r\n");
    server.backend.deliver("INBOX", &plain_message("two"), 0);
    assert_eq!(client.read_response().await.unwrap(), "* 2 EXISTS\r\n");
    assert_eq!(client.read_response().await.unwrap(), "* 2 RECENT\r\n");
    let mut flagged = Flags::new();
    flagged.insert(SystemFlag::Flagged);
    server.backend.set_flags("INBOX", 2, flagged);
    assert_eq!(
        client.read_response().await.unwrap(),
        "* 2 FETCH (UID 2 FLAGS (\\Flagged \\Recent))\r\n"
    );
    server.backend.remove("INBOX", 1);
    assert_eq!(client.read_response().await.unwrap(), "* 1 EXPUNGE\r\n");
    client.send(b"DONE\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "i1 OK IDLE terminated\r\n"
    );
    assert_eq!(
        client.status_of("i2", "NOOP").await,
        "i2 OK NOOP completed\r\n"
    );
    server.stop().await;
}

/// Anything other than DONE ends IDLE with BAD; IDLE also works without a selected mailbox.
#[tokio::test]
async fn idle_requires_done() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client.login().await;
    client.send(b"i1 IDLE\r\n").await;
    assert_eq!(client.read_response().await.unwrap(), "+ idling\r\n");
    client.send(b"i2 NOOP\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "i1 BAD Expected DONE\r\n"
    );
    assert_eq!(
        client.status_of("i3", "NOOP").await,
        "i3 OK NOOP completed\r\n"
    );
    server.stop().await;
}

/// Revoking the account during IDLE ends the session with BYE.
#[tokio::test]
async fn revocation_during_idle_ends_the_session() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.selected("INBOX").await;
    client.send(b"i1 IDLE\r\n").await;
    client.read_response().await.unwrap();
    server.backend.revoke();
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE This IMAP account is no longer available\r\n"
    );
    client.expect_closed().await;
    server.stop().await;
}

/// Long IDLEs receive keepalives and end at the IDLE time limit.
#[tokio::test]
async fn keepalives_and_idle_time_limit() {
    let limits = Limits {
        idle_keepalive_interval: Duration::from_millis(60),
        idle_timeout: Duration::from_millis(200),
        ..test_limits()
    };
    let server = RunningServer::start(limits).await;
    let mut client = server.selected("INBOX").await;
    client.send(b"i1 IDLE\r\n").await;
    client.read_response().await.unwrap();
    assert_eq!(client.read_response().await.unwrap(), "* OK Still here\r\n");
    let mut last = client.read_response().await.unwrap();
    while last == "* OK Still here\r\n" {
        last = client.read_response().await.unwrap();
    }
    assert_eq!(last, "* BYE IDLE timed out; please reconnect\r\n");
    client.expect_closed().await;
    server.stop().await;
}

/// Several sessions of one account idle concurrently and each sees new mail.
#[tokio::test]
async fn concurrent_sessions_all_receive_updates() {
    let server = RunningServer::start(test_limits()).await;
    let mut clients = Vec::new();
    for index in 0..4 {
        let mut client = server
            .selected(if index % 2 == 0 { "INBOX" } else { "Trash" })
            .await;
        client.send(b"i IDLE\r\n").await;
        assert_eq!(client.read_response().await.unwrap(), "+ idling\r\n");
        clients.push(client);
    }
    server.backend.deliver("INBOX", &plain_message("one"), 0);
    server.backend.deliver("Trash", &plain_message("two"), 0);
    for client in &mut clients {
        assert_eq!(client.read_response().await.unwrap(), "* 1 EXISTS\r\n");
        client.send(b"DONE\r\n").await;
        let mut response = client.read_response().await.unwrap();
        while response != "i OK IDLE terminated\r\n" {
            assert!(response.starts_with("* "), "{response}");
            response = client.read_response().await.unwrap();
        }
    }
    server.stop().await;
}

/// A shutdown during IDLE says BYE.
#[tokio::test]
async fn shutdown_during_idle() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.selected("INBOX").await;
    client.send(b"i1 IDLE\r\n").await;
    client.read_response().await.unwrap();
    server.stop().await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE Server shutting down\r\n"
    );
    client.expect_closed().await;
}

/// The refresh operation that fails exactly once after IDLE starts.
#[derive(Clone, Copy)]
enum RefreshFailure {
    Validation,
    Synchronization,
    ValidationTimeout,
}

/// An in-memory backend with one explicitly armed refresh failure.
#[derive(Clone)]
struct RefreshFailureBackend {
    backend: support::backend::MemoryBackend,
    failure: std::sync::Arc<std::sync::atomic::AtomicBool>,
    failure_point: RefreshFailure,
}

/// The ordinary test account with a one-shot refresh failure.
struct RefreshFailureAccount {
    account: support::backend::MemoryAccount,
    failure: std::sync::Arc<std::sync::atomic::AtomicBool>,
    failure_point: RefreshFailure,
}

impl sandpost_imap::ImapBackend for RefreshFailureBackend {
    type Account = RefreshFailureAccount;

    /// Authenticate through the ordinary backend and attach the one-shot failure control.
    async fn authenticate(
        &self,
        credentials: sandpost_imap::Credentials,
    ) -> Result<sandpost_imap::AuthenticationOutcome<Self::Account>, sandpost_imap::BackendError>
    {
        use sandpost_imap::AuthenticationOutcome;
        Ok(match self.backend.authenticate(credentials).await? {
            AuthenticationOutcome::Authenticated(account) => {
                AuthenticationOutcome::Authenticated(RefreshFailureAccount {
                    account,
                    failure: self.failure.clone(),
                    failure_point: self.failure_point,
                })
            }
            AuthenticationOutcome::InvalidCredentials => AuthenticationOutcome::InvalidCredentials,
            AuthenticationOutcome::Throttled => AuthenticationOutcome::Throttled,
        })
    }
}

impl sandpost_imap::Account for RefreshFailureAccount {
    type Mailbox = String;
    type Watch = support::backend::MemoryWatch;

    /// Fail or time out the first armed validation, delegating every other call.
    async fn validate(&self) -> Result<(), sandpost_imap::BackendError> {
        if !matches!(self.failure_point, RefreshFailure::Synchronization)
            && self
                .failure
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            if matches!(self.failure_point, RefreshFailure::ValidationTimeout) {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            return Err(sandpost_imap::BackendError::Unavailable(
                "injected validation failure".into(),
            ));
        }
        self.account.validate().await
    }

    /// Delegate mailbox listing to the ordinary account.
    async fn mailboxes(
        &self,
    ) -> Result<Vec<sandpost_imap::MailboxListing>, sandpost_imap::BackendError> {
        self.account.mailboxes().await
    }

    /// Delegate subscription changes to the ordinary account.
    async fn set_subscription(
        &self,
        name: &str,
        subscribed: bool,
    ) -> Result<(), sandpost_imap::MailboxError> {
        self.account.set_subscription(name, subscribed).await
    }

    /// Delegate selection to the ordinary account.
    async fn open(
        &self,
        name: &str,
        read_only: bool,
    ) -> Result<sandpost_imap::MailboxSnapshot<String>, sandpost_imap::MailboxError> {
        self.account.open(name, read_only).await
    }

    /// Delegate status reads to the ordinary account.
    async fn status(
        &self,
        name: &str,
    ) -> Result<sandpost_imap::MailboxStatus, sandpost_imap::MailboxError> {
        self.account.status(name).await
    }

    /// Fail the first armed synchronization, delegating every other call.
    async fn synchronize(
        &self,
        mailbox: &String,
        request: sandpost_imap::SynchronizationRequest,
    ) -> Result<sandpost_imap::MailboxChanges, sandpost_imap::MailboxError> {
        if matches!(self.failure_point, RefreshFailure::Synchronization)
            && self
                .failure
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(sandpost_imap::MailboxError::Backend(
                sandpost_imap::BackendError::Unavailable("injected synchronization failure".into()),
            ));
        }
        self.account.synchronize(mailbox, request).await
    }

    /// Delegate metadata reads to the ordinary account.
    async fn metadata(
        &self,
        mailbox: &String,
        uids: &[sandpost_imap::Uid],
    ) -> Result<Vec<sandpost_imap::MessageMetadata>, sandpost_imap::MailboxError> {
        self.account.metadata(mailbox, uids).await
    }

    /// Delegate captured-byte reads to the ordinary account.
    async fn content(
        &self,
        mailbox: &String,
        uid: sandpost_imap::Uid,
    ) -> Result<Option<sandpost_imap::MessageContent>, sandpost_imap::MailboxError> {
        self.account.content(mailbox, uid).await
    }

    /// Delegate flag changes to the ordinary account.
    async fn store(
        &self,
        mailbox: &String,
        uids: &[sandpost_imap::Uid],
        change: &sandpost_imap::FlagChange,
    ) -> Result<Vec<(sandpost_imap::Uid, sandpost_imap::Flags)>, sandpost_imap::MailboxError> {
        self.account.store(mailbox, uids, change).await
    }

    /// Delegate expunges to the ordinary account.
    async fn expunge(
        &self,
        mailbox: &String,
        uids: Option<&[sandpost_imap::Uid]>,
    ) -> Result<Vec<sandpost_imap::Uid>, sandpost_imap::MailboxError> {
        self.account.expunge(mailbox, uids).await
    }

    /// Delegate message transfers to the ordinary account.
    async fn copy(
        &self,
        mailbox: &String,
        uids: &[sandpost_imap::Uid],
        destination: &str,
        remove_from_source: bool,
    ) -> Result<sandpost_imap::CopyOutcome, sandpost_imap::MailboxError> {
        self.account
            .copy(mailbox, uids, destination, remove_from_source)
            .await
    }

    /// Subscribe to ordinary in-memory change notifications.
    fn watch(&self) -> Self::Watch {
        self.account.watch()
    }
}

/// Inject one failed refresh after the continuation and require a validated BYE plus closure.
async fn assert_idle_refresh_failure(failure_point: RefreshFailure) {
    use sandpost_imap::ImapServer;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::{
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };

    let backend = RefreshFailureBackend {
        backend: support::backend::MemoryBackend::new(),
        failure: Arc::new(AtomicBool::new(false)),
        failure_point,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, receiver) = oneshot::channel();
    let server = ImapServer::new(backend.clone()).limits(Limits {
        backend_timeout: Duration::from_millis(50),
        ..test_limits()
    });
    let task = tokio::spawn(server.serve(listener, async {
        let _ = receiver.await;
    }));
    let mut client = support::client::TestClient::new(TcpStream::connect(address).await.unwrap());
    client.greeting().await;
    client.login().await;
    client.status_of("s", "SELECT INBOX").await;
    client.send(b"i IDLE\r\n").await;
    assert_eq!(client.read_response().await.unwrap(), "+ idling\r\n");
    backend.failure.store(true, Ordering::SeqCst);
    if matches!(failure_point, RefreshFailure::Validation) {
        backend.backend.revoke();
    } else {
        backend
            .backend
            .deliver("INBOX", &plain_message("new mail"), 0);
    }
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE [UNAVAILABLE] Mailbox updates unavailable; please reconnect\r\n"
    );
    assert!(!backend.failure.load(Ordering::SeqCst));
    client.expect_closed().await;
    shutdown.send(()).unwrap();
    task.await.unwrap();
}

/// A transient validation failure cannot silently consume the revocation notification.
#[tokio::test]
async fn idle_validation_failure_requires_reconnect() {
    assert_idle_refresh_failure(RefreshFailure::Validation).await;
}

/// A transient synchronization failure cannot silently consume a new-mail notification.
#[tokio::test]
async fn idle_synchronization_failure_requires_reconnect() {
    assert_idle_refresh_failure(RefreshFailure::Synchronization).await;
}

/// A backend timeout follows the same explicit reconnect behavior as an unavailable result.
#[tokio::test]
async fn idle_validation_timeout_requires_reconnect() {
    assert_idle_refresh_failure(RefreshFailure::ValidationTimeout).await;
}
