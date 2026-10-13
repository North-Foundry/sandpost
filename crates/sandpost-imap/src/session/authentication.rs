//! STARTTLS (RFC 3501 section 6.2.1), LOGIN (section 6.2.3), and AUTHENTICATE PLAIN
//! (section 6.2.2, RFC 4616, with RFC 4959 initial responses).
//!
//! Only PLAIN is offered: passwords are stored as one-way hashes, so challenge-response
//! mechanisms cannot be verified. Password authentication is refused while the server requires
//! TLS and the connection is still in clear text (LOGINDISABLED).
use super::{Authenticated, Next, Session, bounded, connection::LineRead};
use crate::{
    TransportSecurity,
    backend::{Account, AuthenticationMechanism, AuthenticationOutcome, Credentials, ImapBackend},
    limits::MAXIMUM_AUTHENTICATION_LINE_SIZE,
    protocol::{
        response::{ResponseCode, Status},
        syntax::is_base64_response,
    },
    tls::SessionStream,
};
use mail_parser::decoders::base64::base64_decode;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};

/// Longest accepted username or password, matching RFC 4616's required minimum capacity.
const MAXIMUM_CREDENTIAL_LENGTH: usize = 1024;

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// Upgrade the connection with TLS.
    ///
    /// Cleartext received after the STARTTLS line is thrown away and never interpreted inside
    /// the encrypted session (the command injection defense of RFC 9051 section 6.2.1, option 2).
    pub(super) async fn start_tls(&mut self, tag: &str) -> std::io::Result<Next> {
        let configuration = match &self.configuration.transport_security {
            TransportSecurity::StartTls { configuration, .. }
                if !self.connection.is_encrypted() =>
            {
                configuration.clone()
            }
            _ => {
                self.connection.output.status(
                    Some(tag),
                    Status::Bad,
                    None,
                    "STARTTLS is not available",
                );
                return Ok(Next::Continue);
            }
        };
        self.ok(tag, "Begin TLS negotiation now");
        self.connection.flush().await?;
        self.connection.discard_buffered_input();
        let stream = std::mem::replace(self.connection.stream_mut(), SessionStream::Upgrading);
        let SessionStream::Plain(plain) = stream else {
            return Err(std::io::ErrorKind::InvalidInput.into());
        };
        let encrypted = match timeout(
            self.limits().tls_handshake_timeout,
            configuration.acceptor().accept(plain),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => return Err(std::io::ErrorKind::TimedOut.into()),
        };
        *self.connection.stream_mut() = SessionStream::Encrypted(Box::new(encrypted));
        Ok(Next::Continue)
    }

    /// Run `AUTHENTICATE PLAIN`, reading the response when no initial response was sent.
    pub(super) async fn authenticate(
        &mut self,
        tag: &str,
        mechanism: &str,
        initial_response: Option<String>,
    ) -> std::io::Result<Next> {
        if !self.login_permitted() {
            self.no(
                tag,
                ResponseCode::PrivacyRequired,
                "Authentication requires TLS; use STARTTLS first",
            );
            return Ok(Next::Continue);
        }
        if mechanism != "PLAIN" {
            self.connection.output.status(
                Some(tag),
                Status::No,
                None,
                "Unsupported authentication mechanism",
            );
            return Ok(Next::Continue);
        }
        let encoded = match initial_response {
            Some(response) => response,
            None => {
                self.connection.output.empty_challenge();
                self.connection.flush().await?;
                let read = timeout(
                    self.limits().unauthenticated_timeout,
                    self.connection.read_line(MAXIMUM_AUTHENTICATION_LINE_SIZE),
                )
                .await;
                match read {
                    Ok(Ok(LineRead::Line(line))) => String::from_utf8_lossy(&line).into_owned(),
                    Ok(Ok(LineRead::TooLong)) => {
                        self.connection.output.status(
                            Some(tag),
                            Status::Bad,
                            None,
                            "Authentication response too long",
                        );
                        return Ok(Next::Continue);
                    }
                    Ok(Ok(LineRead::Closed)) => return Ok(Next::Close),
                    Ok(Err(error)) => return Err(error),
                    Err(_) => {
                        self.connection.output.status(
                            None,
                            Status::Bye,
                            None,
                            "Authentication timed out",
                        );
                        return Ok(Next::Close);
                    }
                }
            }
        };
        if encoded == "*" {
            self.connection
                .output
                .status(Some(tag), Status::Bad, None, "Authentication cancelled");
            return Ok(Next::Continue);
        }
        let decoded = if encoded == "=" {
            Some(Vec::new())
        } else if is_base64_response(encoded.as_bytes()) {
            base64_decode(encoded.as_bytes())
        } else {
            None
        };
        let Some((username, password)) = decoded.as_deref().and_then(plain_credentials) else {
            self.connection
                .output
                .status(Some(tag), Status::Bad, None, "Invalid PLAIN response");
            return Ok(Next::Continue);
        };
        self.login(tag, username, password, AuthenticationMechanism::Plain)
            .await
    }

    /// Verify credentials and enter the authenticated state.
    pub(super) async fn login(
        &mut self,
        tag: &str,
        username: String,
        password: String,
        mechanism: AuthenticationMechanism,
    ) -> std::io::Result<Next> {
        if !self.login_permitted() {
            self.no(
                tag,
                ResponseCode::PrivacyRequired,
                "LOGIN requires TLS; use STARTTLS first",
            );
            return Ok(Next::Continue);
        }
        if username.len() > MAXIMUM_CREDENTIAL_LENGTH || password.len() > MAXIMUM_CREDENTIAL_LENGTH
        {
            return self.authentication_failed(tag).await;
        }
        let credentials = Credentials {
            username,
            password,
            mechanism,
            encrypted: self.connection.is_encrypted(),
            remote_address: self.peer_address,
        };
        let limit = self.limits().backend_timeout;
        let outcome = bounded(limit, self.configuration.backend.authenticate(credentials)).await;
        match outcome {
            Ok(Ok(AuthenticationOutcome::Authenticated(account))) => {
                let watch = account.watch();
                self.account = Some(Authenticated { account, watch });
                let capabilities = ResponseCode::Capability(self.capabilities());
                self.connection.output.status(
                    Some(tag),
                    Status::Ok,
                    Some(&capabilities),
                    "Logged in",
                );
                Ok(Next::Continue)
            }
            Ok(Ok(AuthenticationOutcome::InvalidCredentials)) => {
                self.authentication_failed(tag).await
            }
            Ok(Ok(AuthenticationOutcome::Throttled)) => {
                self.no(
                    tag,
                    ResponseCode::Unavailable,
                    "Too many failed logins, try again later",
                );
                Ok(Next::Continue)
            }
            Ok(Err(_)) | Err(_) => {
                self.unavailable(tag);
                Ok(Next::Continue)
            }
        }
    }

    /// Delay, answer a failed authentication, and close after too many failures.
    async fn authentication_failed(&mut self, tag: &str) -> std::io::Result<Next> {
        self.authentication_failures += 1;
        tokio::time::sleep(self.limits().authentication_failure_delay).await;
        self.no(
            tag,
            ResponseCode::AuthenticationFailed,
            "Invalid credentials",
        );
        if self.authentication_failures >= self.limits().maximum_authentication_failures {
            self.connection.output.status(
                None,
                Status::Bye,
                None,
                "Too many authentication failures",
            );
            return Ok(Next::Close);
        }
        Ok(Next::Continue)
    }
}

/// Split a decoded PLAIN message `[authzid] NUL authcid NUL passwd` (RFC 4616 section 2).
///
/// An authorization identity other than the authentication identity is refused, because a
/// session may only act as its own account.
fn plain_credentials(message: &[u8]) -> Option<(String, String)> {
    let mut fields = message.split(|byte| *byte == 0);
    let authorization = fields.next()?;
    let username = fields.next()?;
    let password = fields.next()?;
    if fields.next().is_some() || username.is_empty() || password.is_empty() {
        return None;
    }
    if !authorization.is_empty() && authorization != username {
        return None;
    }
    Some((
        String::from_utf8(username.to_vec()).ok()?,
        String::from_utf8(password.to_vec()).ok()?,
    ))
}

impl<A: Account> std::fmt::Debug for Authenticated<A> {
    /// Identify the state without exposing account internals.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Authenticated { .. }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PLAIN messages need an identity and password; a foreign authorization is refused.
    #[test]
    fn plain_messages_are_validated() {
        assert_eq!(
            plain_credentials(b"\0fred\0secret"),
            Some(("fred".into(), "secret".into()))
        );
        assert_eq!(
            plain_credentials(b"fred\0fred\0secret"),
            Some(("fred".into(), "secret".into()))
        );
        assert_eq!(plain_credentials(b"admin\0fred\0secret"), None);
        assert_eq!(plain_credentials(b"\0fred"), None);
        assert_eq!(plain_credentials(b"\0\0secret"), None);
        assert_eq!(plain_credentials(b"\0fred\0secret\0extra"), None);
        assert_eq!(plain_credentials(b""), None);
    }
}
