//! One SMTP conversation: greeting, command dispatch, transaction state, TLS upgrade, and delivery.
mod authentication;
mod command;
mod delivery;
pub(crate) mod mailbox;
mod trace;

pub(crate) use command::{ArgumentError, Command, MailRequest};

use crate::{
    AuthenticationOutcome, AuthenticationPolicy, Limits, SessionHandler, TransportSecurity,
    framing::{
        MessageDataError, ReceivedLine, is_line_too_long, read_line_with_limit, read_message_data,
        write_reply,
    },
    limits::{MAXIMUM_COMMAND_LINE_SIZE, MAXIMUM_EXTENDED_MAIL_LINE_SIZE, describe_size},
    tls::SessionStream,
};
use authentication::{AUTHENTICATION_CAPABILITY, AuthenticationExchange};
use sandpost_core::Mailbox;
use sandpost_mime::Envelope;
use std::{io::ErrorKind, net::IpAddr, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::OwnedSemaphorePermit,
    time::timeout,
};

/// Everything a session needs from its server, shared by all sessions.
pub(crate) struct SessionConfiguration<Handler> {
    pub(crate) handler: Handler,
    pub(crate) authentication: AuthenticationPolicy,
    pub(crate) transport_security: TransportSecurity,
    pub(crate) limits: Limits,
    pub(crate) server_name: String,
}

/// Run one SMTP conversation over an accepted stream until the client quits or disconnects.
///
/// With implicit TLS the handshake completes before the greeting. The whole conversation is
/// bounded by [`Limits::maximum_session_duration`]; when it expires the client receives `421`
/// and the connection closes, even if each command arrived in time.
pub(crate) async fn run_session<Stream, Handler>(
    stream: Stream,
    configuration: Arc<SessionConfiguration<Handler>>,
    peer_address: Option<IpAddr>,
    connection_permit: Arc<OwnedSemaphorePermit>,
) -> std::io::Result<()>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Handler: SessionHandler,
{
    let limits = configuration.limits;
    let stream = match &configuration.transport_security {
        TransportSecurity::Implicit {
            configuration: tls_configuration,
        } => SessionStream::Encrypted(Box::new(
            handshake(tls_configuration.acceptor(), stream, limits).await?,
        )),
        TransportSecurity::Plaintext | TransportSecurity::StartTls { .. } => {
            SessionStream::Plain(stream)
        }
    };
    let mut session = Session::new(stream, configuration, peer_address, connection_permit);
    let result = match timeout(limits.maximum_session_duration, session.converse()).await {
        Ok(result) => result,
        Err(_) => session.reply("421 4.4.2 session time limit reached").await,
    };
    // Flush the TLS close alert on every normal exit, including QUIT and protocol timeouts.
    let closed = timeout(
        limits.input_output_timeout,
        session.reader.get_mut().shutdown(),
    )
    .await;
    result.and(closed.unwrap_or_else(|_| {
        Err(std::io::Error::new(
            ErrorKind::TimedOut,
            "SMTP shutdown timed out",
        ))
    }))
}

/// Perform a server-side TLS handshake within the configured time.
async fn handshake<Stream: AsyncRead + AsyncWrite + Unpin>(
    acceptor: &tokio_rustls::TlsAcceptor,
    stream: Stream,
    limits: Limits,
) -> std::io::Result<tokio_rustls::server::TlsStream<Stream>> {
    match timeout(limits.tls_handshake_timeout, acceptor.accept(stream)).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            ErrorKind::TimedOut,
            "TLS handshake timed out",
        )),
    }
}

/// Whether the conversation continues after a command.
enum Next {
    Continue,
    Close,
}

/// The state of one SMTP conversation.
struct Session<Stream, Handler: SessionHandler> {
    reader: BufReader<SessionStream<Stream>>,
    configuration: Arc<SessionConfiguration<Handler>>,
    /// Whether the client greeted with EHLO, which AUTH requires.
    extended_hello: bool,
    /// The validated client greeting, forgotten after STARTTLS.
    client_name: Option<String>,
    peer_address: Option<IpAddr>,
    /// Parsing shares the admission lease so a cancelled session cannot release a running parser's slot.
    connection_permit: Arc<OwnedSemaphorePermit>,
    /// The authenticated identity; it survives RSET and repeated greetings, but not STARTTLS.
    principal: Option<Handler::Principal>,
    authentication_failures: usize,
    /// The open mail transaction: present once `MAIL FROM` is accepted, until DATA or RSET.
    transaction: Option<Envelope>,
}

impl<Stream, Handler> Session<Stream, Handler>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Handler: SessionHandler,
{
    /// Start a session over an accepted, possibly already encrypted, stream.
    fn new(
        stream: SessionStream<Stream>,
        configuration: Arc<SessionConfiguration<Handler>>,
        peer_address: Option<IpAddr>,
        connection_permit: Arc<OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            reader: BufReader::new(stream),
            configuration,
            extended_hello: false,
            client_name: None,
            peer_address,
            connection_permit,
            principal: None,
            authentication_failures: 0,
            transaction: None,
        }
    }

    /// The bounds this session runs under.
    fn limits(&self) -> Limits {
        self.configuration.limits
    }

    /// Report whether the connection is protected by TLS.
    fn is_encrypted(&self) -> bool {
        self.reader.get_ref().is_encrypted()
    }

    /// Report whether STARTTLS can still be issued on this connection.
    fn offers_start_tls(&self) -> bool {
        matches!(
            self.configuration.transport_security,
            TransportSecurity::StartTls { .. }
        ) && !self.is_encrypted()
    }

    /// Report whether AUTH and mail must wait until the session upgrades with STARTTLS.
    fn awaits_required_tls(&self) -> bool {
        matches!(
            self.configuration.transport_security,
            TransportSecurity::StartTls { required: true, .. }
        ) && !self.is_encrypted()
    }

    /// Greet the client, then read and execute commands until the conversation ends.
    async fn converse(&mut self) -> std::io::Result<()> {
        self.reply(&format!(
            "220 {} ESMTP ready",
            self.configuration.server_name
        ))
        .await?;
        loop {
            let read_timeout = self.limits().input_output_timeout;
            let line = read_line_with_limit(
                &mut self.reader,
                read_timeout,
                MAXIMUM_EXTENDED_MAIL_LINE_SIZE,
            )
            .await;
            let command_line = match line {
                Err(error) if is_line_too_long(&error) => {
                    self.reply("500 5.5.2 command line too long").await?;
                    return Ok(());
                }
                Err(error) => return Err(error),
                Ok(ReceivedLine::Line(command_line)) => command_line,
                Ok(ReceivedLine::Closed) => return Ok(()),
                Ok(ReceivedLine::TimedOut) => return self.reply("421 4.4.2 timeout").await,
            };
            if !command_line.is_ascii()
                || command_line
                    .iter()
                    .any(|byte| byte.is_ascii_control() && *byte != b'\t')
            {
                self.reply("500 5.5.2 invalid command encoding").await?;
                continue;
            }
            let Ok(command_line) = String::from_utf8(command_line) else {
                self.reply("500 5.5.2 invalid command encoding").await?;
                continue;
            };
            let command = Command::parse(&command_line);
            let limit = if matches!(command, Command::Mail { .. }) && self.extended_hello {
                MAXIMUM_EXTENDED_MAIL_LINE_SIZE
            } else {
                MAXIMUM_COMMAND_LINE_SIZE
            };
            if command_line.len() + 2 > limit {
                self.reply("500 5.5.2 command line too long").await?;
                continue;
            }
            if let Next::Close = self.execute(command).await? {
                return Ok(());
            }
        }
    }

    /// Execute one command and report whether the conversation continues.
    async fn execute(&mut self, command: Command<'_>) -> std::io::Result<Next> {
        match command {
            Command::Hello { extended, domain } => self.hello(extended, domain).await,
            Command::Authenticate { argument } => self.authenticate(argument).await,
            Command::StartTls => self.start_tls().await,
            Command::Mail { request } => self.mail(request).await,
            Command::Recipient { recipient } => self.recipient(recipient).await,
            Command::Data => self.data().await,
            Command::Reset => {
                self.transaction = None;
                self.reply_and_continue("250 2.0.0 reset").await
            }
            Command::NoOperation => self.reply_and_continue("250 2.0.0 OK").await,
            Command::Quit => {
                self.reply("221 2.0.0 bye").await?;
                Ok(Next::Close)
            }
            Command::Verify => {
                self.reply_and_continue(
                    "252 2.1.5 cannot VRFY user, but will accept the message and attempt delivery",
                )
                .await
            }
            Command::Help => {
                self.reply_and_continue(
                    "214 2.0.0 supported: EHLO HELO STARTTLS AUTH MAIL RCPT DATA RSET NOOP QUIT VRFY HELP",
                )
                .await
            }
            Command::NotImplemented => {
                self.reply_and_continue("502 5.5.1 command not implemented")
                    .await
            }
            Command::Unrecognized => {
                self.reply_and_continue("500 5.5.2 command not recognized")
                    .await
            }
            Command::InvalidSyntax(explanation) => {
                self.reply_and_continue(&format!("501 5.5.4 {explanation}"))
                    .await
            }
        }
    }

    /// Answer HELO, or EHLO with the extensions available on this connection.
    ///
    /// A greeting abandons any open transaction but keeps the principal. STARTTLS is listed only
    /// before TLS, and AUTH only once the connection meets the transport requirement.
    async fn hello(&mut self, extended: bool, domain: &str) -> std::io::Result<Next> {
        self.transaction = None;
        self.extended_hello = extended;
        self.client_name = Some(domain.to_owned());
        if !extended {
            return self
                .reply_and_continue(&format!("250 {}", self.configuration.server_name))
                .await;
        }
        let mut lines = vec![
            self.configuration.server_name.clone(),
            "PIPELINING".to_owned(),
            if self.limits().maximum_message_size == 0 {
                "SIZE".to_owned()
            } else {
                format!("SIZE {}", self.limits().maximum_message_size)
            },
            "8BITMIME".to_owned(),
            "ENHANCEDSTATUSCODES".to_owned(),
        ];
        if self.offers_start_tls() {
            lines.push("STARTTLS".to_owned());
        }
        if !self.awaits_required_tls() {
            lines.push(AUTHENTICATION_CAPABILITY.to_owned());
        }
        let last = lines.len() - 1;
        for (index, line) in lines.iter().enumerate() {
            let separator = if index == last { ' ' } else { '-' };
            self.reply(&format!("250{separator}{line}")).await?;
        }
        Ok(Next::Continue)
    }

    /// Upgrade the connection with STARTTLS and start the conversation over.
    ///
    /// Bytes the client sent after the command, before the handshake, are refused by closing
    /// the connection: executing them as if they had been encrypted is the STARTTLS command
    /// injection attack. After the handshake the client must greet again, and any principal,
    /// failure count, or transaction from the plaintext part is forgotten.
    async fn start_tls(&mut self) -> std::io::Result<Next> {
        let configuration = Arc::clone(&self.configuration);
        let TransportSecurity::StartTls {
            configuration: tls_configuration,
            ..
        } = &configuration.transport_security
        else {
            return self
                .reply_and_continue("502 5.5.1 STARTTLS is not available")
                .await;
        };
        if self.is_encrypted() {
            return self
                .reply_and_continue("503 5.5.1 TLS is already active")
                .await;
        }
        if self.transaction.is_some() {
            return self
                .reply_and_continue("503 5.5.1 STARTTLS is not permitted during a mail transaction")
                .await;
        }
        if !self.reader.buffer().is_empty() {
            self.reply("421 4.7.0 commands pipelined after STARTTLS are refused")
                .await?;
            return Ok(Next::Close);
        }
        self.reply("220 2.0.0 ready to start TLS").await?;
        let SessionStream::Plain(plain) =
            std::mem::replace(self.reader.get_mut(), SessionStream::Upgrading)
        else {
            return Err(std::io::Error::other("the connection is not in plaintext"));
        };
        let encrypted = handshake(tls_configuration.acceptor(), plain, self.limits()).await?;
        *self.reader.get_mut() = SessionStream::Encrypted(Box::new(encrypted));
        self.extended_hello = false;
        self.client_name = None;
        self.principal = None;
        self.authentication_failures = 0;
        Ok(Next::Continue)
    }

    /// Run an AUTH exchange and bind the verified principal to the session.
    ///
    /// Invalid credentials count toward the failure limit; a handler that cannot verify them
    /// right now answers `454` without counting.
    async fn authenticate(&mut self, argument: &str) -> std::io::Result<Next> {
        if self.awaits_required_tls() {
            return self
                .reply_and_continue(
                    "538 5.7.11 encryption required for requested authentication mechanism",
                )
                .await;
        }
        if self.principal.is_some() {
            return self
                .reply_and_continue("503 5.5.1 already authenticated")
                .await;
        }
        if !self.extended_hello {
            return self.reply_and_continue("503 5.5.1 send EHLO first").await;
        }
        if self.transaction.is_some() {
            return self
                .reply_and_continue("503 5.5.1 AUTH is not permitted during a mail transaction")
                .await;
        }
        let limits = self.limits();
        let encrypted = self.is_encrypted();
        let exchange = authentication::read_authentication_exchange(
            &mut self.reader,
            argument,
            limits.input_output_timeout,
            encrypted,
        )
        .await?;
        let outcome = match exchange {
            AuthenticationExchange::Credentials(credentials) => {
                match timeout(
                    limits.handler_timeout,
                    self.configuration.handler.authenticate(credentials),
                )
                .await
                {
                    Ok(Ok(outcome)) => outcome,
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "SMTP authentication could not be verified");
                        return self
                            .reply_and_continue("454 4.7.0 temporary authentication failure")
                            .await;
                    }
                    Err(_) => {
                        return self
                            .reply_and_continue("454 4.7.0 authentication timed out")
                            .await;
                    }
                }
            }
            AuthenticationExchange::ForeignAuthorization => {
                AuthenticationOutcome::InvalidCredentials
            }
            AuthenticationExchange::Refused(reply) => return self.reply_and_continue(reply).await,
            AuthenticationExchange::Closed => return Ok(Next::Close),
        };
        match outcome {
            AuthenticationOutcome::Authenticated(principal) => {
                self.principal = Some(principal);
                return self
                    .reply_and_continue("235 2.7.0 authentication succeeded")
                    .await;
            }
            AuthenticationOutcome::EncryptionRequired => {
                return self
                    .reply_and_continue("538 5.7.11 this account requires an encrypted connection")
                    .await;
            }
            AuthenticationOutcome::MechanismNotAllowed => {
                return self
                    .reply_and_continue(
                        "534 5.7.9 authentication mechanism not allowed for this account",
                    )
                    .await;
            }
            AuthenticationOutcome::InvalidCredentials => {}
        }
        self.authentication_failures += 1;
        if self.authentication_failures >= limits.maximum_authentication_failures {
            self.reply("421 4.7.0 too many authentication failures")
                .await?;
            return Ok(Next::Close);
        }
        self.reply_and_continue("535 5.7.8 authentication credentials invalid")
            .await
    }

    /// Open a mail transaction for the given reverse path.
    async fn mail(&mut self, request: Result<MailRequest, ArgumentError>) -> std::io::Result<Next> {
        let maximum_message_size = self.limits().maximum_message_size;
        let reply = if self.awaits_required_tls() {
            "530 5.7.0 must issue a STARTTLS command first".to_owned()
        } else if self.configuration.authentication == AuthenticationPolicy::Required
            && self.principal.is_none()
        {
            "530 5.7.0 authentication required".to_owned()
        } else if self.transaction.is_some() {
            "503 5.5.1 nested MAIL command".to_owned()
        } else if self.client_name.is_none() {
            "503 5.5.1 send HELO or EHLO first".to_owned()
        } else {
            match request {
                Ok(request) if request.has_parameters && !self.extended_hello => {
                    "555 5.5.4 MAIL parameters require EHLO".to_owned()
                }
                Ok(request)
                    if request
                        .declared_size
                        .is_some_and(|size| size > maximum_message_size) =>
                {
                    format!(
                        "552 5.3.4 message size exceeds the {} limit",
                        describe_size(maximum_message_size)
                    )
                }
                Ok(request) => {
                    self.transaction = Some(Envelope {
                        sender: request.sender,
                        recipients: Vec::new(),
                    });
                    "250 2.1.0 sender ok".to_owned()
                }
                Err(ArgumentError::InvalidPath) => "501 5.1.7 invalid sender address".to_owned(),
                Err(ArgumentError::UnsupportedParameter) => {
                    "555 5.5.4 unsupported MAIL parameter".to_owned()
                }
                Err(ArgumentError::InvalidParameter) => {
                    "501 5.5.4 invalid MAIL parameter value".to_owned()
                }
            }
        };
        self.reply_and_continue(&reply).await
    }

    /// Add one forward path to the open transaction.
    async fn recipient(
        &mut self,
        recipient: Result<Mailbox, ArgumentError>,
    ) -> std::io::Result<Next> {
        let recipient = recipient.map(|mut mailbox| {
            if mailbox.domain.is_empty() {
                mailbox.domain = self.configuration.server_name.clone();
                mailbox.address = format!("{}@{}", mailbox.address, mailbox.domain);
            }
            mailbox
        });
        let maximum_recipient_count = self.limits().maximum_recipient_count;
        let reply = match (self.transaction.as_mut(), recipient) {
            (None, _) => "503 5.5.1 send MAIL FROM first",
            (Some(envelope), _) if envelope.recipients.len() >= maximum_recipient_count => {
                "452 4.5.3 too many recipients"
            }
            (Some(envelope), Ok(recipient)) => {
                envelope.recipients.push(recipient);
                "250 2.1.5 recipient ok"
            }
            (Some(_), Err(ArgumentError::InvalidPath)) => "501 5.1.3 invalid recipient address",
            (Some(_), Err(ArgumentError::UnsupportedParameter)) => {
                "555 5.5.4 unsupported RCPT parameter"
            }
            (Some(_), Err(ArgumentError::InvalidParameter)) => {
                "501 5.5.4 invalid RCPT parameter value"
            }
        };
        self.reply_and_continue(reply).await
    }

    /// Receive the message of a complete transaction and report the delivery outcome.
    ///
    /// The transaction ends whatever the outcome: a refused or failed message must be resent
    /// from `MAIL FROM`. An oversized message or a stalled client closes the session.
    async fn data(&mut self) -> std::io::Result<Next> {
        let envelope = match self.transaction.take() {
            Some(envelope) if !envelope.recipients.is_empty() => envelope,
            unchanged => {
                self.transaction = unchanged;
                return self
                    .reply_and_continue("503 5.5.1 MAIL and RCPT required")
                    .await;
            }
        };
        let limits = self.limits();
        self.reply("354 end data with <CRLF>.<CRLF>").await?;
        let raw_message = match read_message_data(
            &mut self.reader,
            limits.maximum_message_size,
            limits.input_output_timeout,
        )
        .await
        {
            Ok(raw_message) => raw_message,
            Err(MessageDataError::MessageTooLarge) => {
                let limit = describe_size(limits.maximum_message_size);
                self.reply(&format!("552 5.3.4 message exceeds {limit}"))
                    .await?;
                return Ok(Next::Close);
            }
            Err(MessageDataError::ReadTimeout) => {
                self.reply("421 4.4.2 timeout").await?;
                return Ok(Next::Close);
            }
            Err(MessageDataError::InputOutput(error)) => return Err(error),
        };
        let trace = trace::Trace {
            client_name: self
                .client_name
                .as_deref()
                .expect("MAIL requires a greeting"),
            server_name: &self.configuration.server_name,
            peer_address: self.peer_address,
            extended: self.extended_hello,
            authenticated: self.principal.is_some(),
            encrypted: self.is_encrypted(),
        }
        .headers(&envelope);
        let reply = delivery::deliver(
            Arc::clone(&self.configuration),
            self.principal.clone(),
            envelope,
            raw_message,
            trace,
            Arc::clone(&self.connection_permit),
        )
        .await;
        self.reply_and_continue(&reply).await
    }

    /// Write one reply line to the client.
    async fn reply(&mut self, reply: &str) -> std::io::Result<()> {
        let write_timeout = self.limits().input_output_timeout;
        write_reply(self.reader.get_mut(), reply, write_timeout).await
    }

    /// Write one reply line and keep the conversation open.
    async fn reply_and_continue(&mut self, reply: &str) -> std::io::Result<Next> {
        self.reply(reply).await?;
        Ok(Next::Continue)
    }
}
