//! One IMAP conversation: greeting, state machine, and command dispatch.
//!
//! States follow RFC 3501 section 3: not authenticated, authenticated, selected, and logout.
//! Commands used in the wrong state receive BAD. Every command in the authenticated states first
//! revalidates the account. Commands synchronize selected state where needed, deferring foreign
//! EXPUNGE responses when sequence targets must stay stable, without polling the mailbox.
mod authentication;
mod command_input;
pub(crate) mod connection;
mod fetch;
mod idle;
mod mailboxes;
mod messages;
mod search;
pub(crate) mod selected;

use crate::{
    Limits, TransportSecurity,
    backend::{Account, BackendError, ImapBackend, MailboxError},
    limits::MAXIMUM_CONSECUTIVE_BAD_COMMANDS,
    protocol::{
        command::{Command, CommandKind},
        mailbox_name,
        parser::parse_command,
        response::{ResponseCode, Status},
    },
    tls::SessionStream,
};
use command_input::CommandRead;
use connection::Connection;
use selected::SelectedMailbox;
use std::{future::Future, net::IpAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{OwnedSemaphorePermit, watch},
    time::timeout,
};

/// Everything a session needs from its server, shared by all sessions.
pub(crate) struct SessionConfiguration<Backend> {
    pub(crate) backend: Backend,
    pub(crate) transport_security: TransportSecurity,
    pub(crate) limits: Limits,
    pub(crate) greeting: String,
}

/// Whether the conversation continues after a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Next {
    Continue,
    Close,
}

/// Run one conversation over an accepted stream until logout, disconnection, or shutdown.
pub(crate) async fn run_session<Stream, Backend>(
    stream: Stream,
    configuration: Arc<SessionConfiguration<Backend>>,
    peer_address: Option<IpAddr>,
    shutdown: watch::Receiver<bool>,
    _connection_permit: OwnedSemaphorePermit,
) -> std::io::Result<()>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    let limits = configuration.limits;
    let stream = match &configuration.transport_security {
        TransportSecurity::Implicit {
            configuration: tls_configuration,
        } => {
            match timeout(
                limits.tls_handshake_timeout,
                tls_configuration.acceptor().accept(stream),
            )
            .await
            {
                Ok(result) => SessionStream::Encrypted(Box::new(result?)),
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "TLS handshake timed out",
                    ));
                }
            }
        }
        TransportSecurity::Plaintext | TransportSecurity::StartTls { .. } => {
            SessionStream::Plain(stream)
        }
    };
    let mut session = Session {
        connection: Connection::new(stream, limits.input_output_timeout),
        configuration,
        peer_address,
        shutdown,
        account: None,
        selected: None,
        authentication_failures: 0,
    };
    let result = session.converse().await;
    let closed = session.connection.shutdown().await;
    result.and(closed.or_else(|error| match error.kind() {
        std::io::ErrorKind::NotConnected | std::io::ErrorKind::BrokenPipe => Ok(()),
        _ => Err(error),
    }))
}

/// An authenticated account and its change subscription.
struct Authenticated<A: Account> {
    account: A,
    watch: A::Watch,
}

/// The state of one IMAP conversation.
pub(crate) struct Session<Stream, Backend: ImapBackend> {
    connection: Connection<Stream>,
    configuration: Arc<SessionConfiguration<Backend>>,
    peer_address: Option<IpAddr>,
    shutdown: watch::Receiver<bool>,
    account: Option<Authenticated<Backend::Account>>,
    selected: Option<SelectedMailbox<<Backend::Account as Account>::Mailbox>>,
    authentication_failures: usize,
}

/// Wait until shutdown is requested or the sender is gone.
async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
    std::future::pending::<()>().await;
}

/// Await a backend call within a time bound, turning a timeout into a temporary failure.
pub(crate) async fn bounded<T>(
    limit: Duration,
    call: impl Future<Output = T>,
) -> Result<T, BackendError> {
    timeout(limit, call)
        .await
        .map_err(|_| BackendError::Unavailable("backend timed out".into()))
}

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// The bounds this session runs under.
    fn limits(&self) -> Limits {
        self.configuration.limits
    }

    /// Report whether the client may authenticate with a password on this connection.
    fn login_permitted(&self) -> bool {
        !matches!(
            self.configuration.transport_security,
            TransportSecurity::StartTls { required: true, .. }
        ) || self.connection.is_encrypted()
    }

    /// Report whether STARTTLS can still be issued.
    fn offers_start_tls(&self) -> bool {
        matches!(
            self.configuration.transport_security,
            TransportSecurity::StartTls { .. }
        ) && !self.connection.is_encrypted()
    }

    /// The capability list for the current state (RFC 3501 section 7.2.1).
    fn capabilities(&self) -> String {
        let mut capabilities = vec!["IMAP4rev1"];
        if self.account.is_none() {
            if self.offers_start_tls() {
                capabilities.push("STARTTLS");
            }
            if self.login_permitted() {
                capabilities.extend(["AUTH=PLAIN", "SASL-IR"]);
            } else {
                capabilities.push("LOGINDISABLED");
            }
        }
        capabilities.extend([
            "LITERAL-",
            "ID",
            "IDLE",
            "NAMESPACE",
            "UIDPLUS",
            "MOVE",
            "UNSELECT",
            "CHILDREN",
        ]);
        capabilities.join(" ")
    }

    /// Greet the client, then read and execute commands until the conversation ends.
    async fn converse(&mut self) -> std::io::Result<()> {
        let greeting = format!("{} ready", self.configuration.greeting);
        let capabilities = ResponseCode::Capability(self.capabilities());
        self.connection
            .output
            .status(None, Status::Ok, Some(&capabilities), &greeting);
        self.connection.flush().await?;
        let mut consecutive_bad = 0;
        loop {
            let wait = if self.account.is_some() {
                self.limits().authenticated_timeout
            } else {
                self.limits().unauthenticated_timeout
            };
            let next = match self.read_command(wait).await? {
                CommandRead::Command(bytes) => match parse_command(&bytes) {
                    Ok(command) => {
                        consecutive_bad = 0;
                        self.execute(command).await?
                    }
                    Err(error) => {
                        consecutive_bad += 1;
                        self.connection.output.status(
                            error.tag.as_deref(),
                            Status::Bad,
                            None,
                            &format!("Invalid command: {}", error.message),
                        );
                        if consecutive_bad >= MAXIMUM_CONSECUTIVE_BAD_COMMANDS {
                            self.connection.output.status(
                                None,
                                Status::Bye,
                                None,
                                "Too many invalid commands",
                            );
                            Next::Close
                        } else {
                            Next::Continue
                        }
                    }
                },
                CommandRead::Refused => Next::Continue,
                CommandRead::Closed => return Ok(()),
                CommandRead::Fatal => Next::Close,
                CommandRead::TimedOut => {
                    self.connection.output.status(
                        None,
                        Status::Bye,
                        None,
                        "Autologout; idle for too long",
                    );
                    Next::Close
                }
                CommandRead::ShuttingDown => {
                    self.connection
                        .output
                        .status(None, Status::Bye, None, "Server shutting down");
                    Next::Close
                }
            };
            self.connection.flush().await?;
            if next == Next::Close {
                return Ok(());
            }
        }
    }

    /// The authenticated account; callers check the state first.
    fn account(&self) -> &Backend::Account {
        &self.account.as_ref().expect("authenticated").account
    }

    /// Execute one parsed command and report whether the conversation continues.
    async fn execute(&mut self, command: Command) -> std::io::Result<Next> {
        let Command { tag, kind } = command;
        if let Some(problem) = self.state_problem(&kind) {
            self.connection
                .output
                .status(Some(&tag), Status::Bad, None, problem);
            return Ok(Next::Continue);
        }
        if self.account.is_some() && !matches!(kind, CommandKind::Logout) {
            match self.validate_account().await {
                Ok(()) => {}
                Err(BackendError::AccountRevoked) => return Ok(self.revoked()),
                Err(BackendError::Unavailable(_)) => {
                    self.unavailable(&tag);
                    return Ok(Next::Continue);
                }
            }
        }
        if self.selected.is_some()
            && !matches!(
                kind,
                CommandKind::Logout
                    | CommandKind::Select { .. }
                    | CommandKind::Close
                    | CommandKind::Unselect
            )
        {
            let allow_expunge = !kind.forbids_expunge_responses();
            if let Err(error) = self.synchronize(allow_expunge).await {
                return self.mailbox_failure(&tag, error);
            }
        }
        match kind {
            CommandKind::Capability => {
                let capabilities = self.capabilities();
                self.connection
                    .output
                    .raw("* CAPABILITY ")
                    .raw(&capabilities)
                    .end_line();
                self.ok(&tag, "CAPABILITY completed");
            }
            CommandKind::Noop => self.ok(&tag, "NOOP completed"),
            CommandKind::Check => self.ok(&tag, "CHECK completed"),
            CommandKind::Logout => {
                self.connection
                    .output
                    .status(None, Status::Bye, None, "Logging out");
                self.ok(&tag, "LOGOUT completed");
                return Ok(Next::Close);
            }
            CommandKind::Id => {
                self.connection
                    .output
                    .raw("* ID (\"name\" \"Sandpost\")")
                    .end_line();
                self.ok(&tag, "ID completed");
            }
            CommandKind::Namespace => {
                self.connection
                    .output
                    .raw(&format!(
                        "* NAMESPACE ((\"\" \"{}\")) NIL NIL",
                        mailbox_name::HIERARCHY_DELIMITER
                    ))
                    .end_line();
                self.ok(&tag, "NAMESPACE completed");
            }
            CommandKind::StartTls => return self.start_tls(&tag).await,
            CommandKind::Login { username, password } => {
                return self
                    .login(
                        &tag,
                        username,
                        password,
                        crate::backend::AuthenticationMechanism::Login,
                    )
                    .await;
            }
            CommandKind::Authenticate {
                mechanism,
                initial_response,
            } => return self.authenticate(&tag, &mechanism, initial_response).await,
            CommandKind::Select { mailbox, read_only } => {
                return self.select(&tag, &mailbox, read_only).await;
            }
            CommandKind::Create { mailbox } => return self.create(&tag, &mailbox).await,
            CommandKind::Delete { mailbox } => return self.delete(&tag, &mailbox).await,
            CommandKind::Rename {
                source,
                destination,
            } => return self.rename(&tag, &source, &destination).await,
            CommandKind::Subscribe { mailbox, subscribe } => {
                return self.subscribe(&tag, &mailbox, subscribe).await;
            }
            CommandKind::List {
                reference,
                pattern,
                subscribed_only,
            } => return self.list(&tag, &reference, &pattern, subscribed_only).await,
            CommandKind::Status { mailbox, items } => {
                return self.status(&tag, &mailbox, &items).await;
            }
            CommandKind::Append { mailbox } => {
                return self.refuse_append(Some(&tag), Some(&mailbox)).await;
            }
            CommandKind::Close => return self.close(&tag).await,
            CommandKind::Unselect => {
                self.selected = None;
                self.ok(&tag, "UNSELECT completed");
            }
            CommandKind::Expunge { uids } => return self.expunge(&tag, uids.as_ref()).await,
            CommandKind::Search {
                uid,
                charset,
                criteria,
            } => return self.search(&tag, uid, charset.as_deref(), &criteria).await,
            CommandKind::Fetch {
                uid,
                set,
                attributes,
            } => return self.fetch(&tag, uid, &set, &attributes).await,
            CommandKind::Store {
                uid,
                set,
                change,
                silent,
            } => return self.store(&tag, uid, &set, &change, silent).await,
            CommandKind::Copy {
                uid,
                set,
                mailbox,
                remove_from_source,
            } => {
                return self
                    .copy(&tag, uid, &set, &mailbox, remove_from_source)
                    .await;
            }
            CommandKind::Idle => return self.idle(&tag).await,
        }
        Ok(Next::Continue)
    }

    /// Explain why a command is not valid in the current state, if it is not.
    fn state_problem(&self, kind: &CommandKind) -> Option<&'static str> {
        let authenticated = self.account.is_some();
        let selected = self.selected.is_some();
        match kind {
            CommandKind::Capability | CommandKind::Noop | CommandKind::Logout | CommandKind::Id => {
                None
            }
            CommandKind::StartTls
            | CommandKind::Login { .. }
            | CommandKind::Authenticate { .. } => authenticated.then_some("Already authenticated"),
            CommandKind::Check
            | CommandKind::Close
            | CommandKind::Unselect
            | CommandKind::Expunge { .. }
            | CommandKind::Search { .. }
            | CommandKind::Fetch { .. }
            | CommandKind::Store { .. }
            | CommandKind::Copy { .. } => {
                (!selected).then_some("Command requires a selected mailbox")
            }
            _ => (!authenticated).then_some("Command requires authentication"),
        }
    }

    /// Confirm the account may continue.
    async fn validate_account(&mut self) -> Result<(), BackendError> {
        let limit = self.limits().backend_timeout;
        bounded(limit, self.account().validate()).await?
    }

    /// Synchronize the selected mailbox and queue untagged responses for what changed.
    async fn synchronize(&mut self, allow_expunge: bool) -> Result<(), MailboxError> {
        let limit = self.limits().backend_timeout;
        let Some(selected) = self.selected.as_ref() else {
            return Ok(());
        };
        let request = selected.synchronization_request();
        let account = &self.account.as_ref().expect("authenticated").account;
        let result = bounded(limit, account.synchronize(&selected.handle, request))
            .await
            .map_err(MailboxError::Backend)?;
        let selected = self.selected.as_mut().expect("selected");
        match result {
            Ok(changes) => {
                selected.apply(request, changes, allow_expunge, &mut self.connection.output);
                Ok(())
            }
            Err(MailboxError::NonExistent) => {
                // The mailbox disappeared or became inaccessible: nothing in it stays visible.
                selected.mark_all_expunged();
                if allow_expunge {
                    selected.announce_expunges(&mut self.connection.output);
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Queue a tagged OK.
    fn ok(&mut self, tag: &str, text: &str) {
        self.connection
            .output
            .status(Some(tag), Status::Ok, None, text);
    }

    /// Queue a tagged NO with a response code.
    fn no(&mut self, tag: &str, code: ResponseCode, text: &str) {
        self.connection
            .output
            .status(Some(tag), Status::No, Some(&code), text);
    }

    /// Queue a tagged NO for a temporary backend failure.
    fn unavailable(&mut self, tag: &str) {
        self.no(
            tag,
            ResponseCode::Unavailable,
            "Temporarily unavailable, try again later",
        );
    }

    /// Queue BYE for a revoked account and end the session.
    fn revoked(&mut self) -> Next {
        self.connection.output.status(
            None,
            Status::Bye,
            None,
            "This IMAP account is no longer available",
        );
        Next::Close
    }

    /// Answer a mailbox operation failure; a revoked account ends the session.
    fn mailbox_failure(&mut self, tag: &str, error: MailboxError) -> std::io::Result<Next> {
        match error {
            MailboxError::NonExistent => {
                self.no(tag, ResponseCode::NonExistent, "Mailbox does not exist");
            }
            MailboxError::NotPermitted(reason) => self.no(tag, ResponseCode::Cannot, &reason),
            MailboxError::Backend(BackendError::Unavailable(_)) => self.unavailable(tag),
            MailboxError::Backend(BackendError::AccountRevoked) => return Ok(self.revoked()),
        }
        Ok(Next::Continue)
    }
}
