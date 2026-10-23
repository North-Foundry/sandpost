//! Opening an authenticated, optionally encrypted SMTP client session.
use crate::destination::{DestinationPolicy, resolve};
use crate::failure::{Phase, classify, failure, timeout};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{AsyncSmtpConnection, TlsParameters};
use lettre::transport::smtp::extension::{ClientId, Extension};
use sandpost_core::{
    OutboundFailure, OutboundFailureKind, SmtpOutboundAuthentication, SmtpOutboundSecurity,
};
use std::net::SocketAddr;
use std::time::Duration;

/// The `EHLO` name used when an SMTP Outbound does not configure one.
pub const DEFAULT_HELO_NAME: &str = "sandpost.localhost";

/// Everything needed to connect and authenticate to one SMTP server.
///
/// Intentionally lacks Debug because it carries the password.
#[derive(Clone)]
pub struct OutboundConnection {
    /// Host name or IP address; also the TLS server name.
    pub host: String,
    pub port: u16,
    pub security: SmtpOutboundSecurity,
    /// Credentials to authenticate with, when the server requires them.
    pub credentials: Option<OutboundCredentials>,
    /// Verify the certificate chain and host name; disabling it accepts any certificate.
    pub verify_certificates: bool,
    /// The name sent in `EHLO`.
    pub helo_name: String,
    /// Bound on resolving, connecting, TLS, the greeting and `EHLO`.
    pub connect_timeout: Duration,
    /// Bound on each later command and its reply.
    pub command_timeout: Duration,
}

/// SMTP AUTH credentials in clear text, held only while connecting.
///
/// Intentionally lacks Debug so the password is never formatted into logs.
#[derive(Clone)]
pub struct OutboundCredentials {
    /// `Plain` or `Login`; `None` means no authentication and is ignored.
    pub mechanism: SmtpOutboundAuthentication,
    pub username: String,
    pub password: String,
}

/// An open SMTP session ready for a mail transaction.
pub(crate) struct Session {
    pub(crate) connection: AsyncSmtpConnection,
    pub(crate) address: SocketAddr,
    pub(crate) encrypted: bool,
    pub(crate) authenticated: bool,
}

/// Why a session could not be opened, with the address reached when there was one.
pub(crate) struct SessionFailure {
    pub(crate) failure: OutboundFailure,
    pub(crate) address: Option<SocketAddr>,
    pub(crate) encrypted: bool,
}

/**
 * Build the TLS parameters for the configured server name and verification choice.
 */
fn tls_parameters(connection: &OutboundConnection) -> Result<TlsParameters, OutboundFailure> {
    TlsParameters::builder(connection.host.clone())
        .dangerous_accept_invalid_certs(!connection.verify_certificates)
        .dangerous_accept_invalid_hostnames(!connection.verify_certificates)
        .build_rustls()
        .map_err(|error| {
            failure(
                OutboundFailureKind::Tls,
                format!("TLS could not be configured: {error}"),
            )
        })
}

/**
 * Connect to the first reachable resolved address and read the greeting and `EHLO` reply.
 *
 * Implicit TLS is negotiated before the greeting. Each address gets the whole connect timeout.
 */
async fn connect(
    connection: &OutboundConnection,
    addresses: &[SocketAddr],
    client: &ClientId,
    implicit_tls: Option<TlsParameters>,
) -> Result<(AsyncSmtpConnection, SocketAddr), OutboundFailure> {
    let mut last_failure = failure(OutboundFailureKind::Connection, "no address to connect to");
    for address in addresses {
        let attempt = AsyncSmtpConnection::connect_tokio1(
            *address,
            Some(connection.connect_timeout),
            client,
            implicit_tls.clone(),
            None,
        );
        match tokio::time::timeout(connection.connect_timeout, attempt).await {
            Ok(Ok(smtp)) => return Ok((smtp, *address)),
            Ok(Err(error)) => {
                last_failure = classify(&error, Phase::Connect);
                last_failure.message = format!("{address}: {}", last_failure.message);
            }
            Err(_) => {
                last_failure = timeout(Phase::Connect, connection.connect_timeout);
                last_failure.message = format!("{address}: {}", last_failure.message);
            }
        }
    }
    Err(last_failure)
}

/**
 * Open a session: resolve under `policy`, connect, enforce the configured TLS mode and
 * authenticate.
 *
 * STARTTLS is required when configured: a server that does not offer it fails the session with
 * a TLS failure instead of continuing in clear text. The session is closed before returning a
 * failure that occurs after connecting.
 */
pub(crate) async fn open(
    connection: &OutboundConnection,
    policy: &DestinationPolicy,
) -> Result<Session, SessionFailure> {
    let addresses = resolve(
        &connection.host,
        connection.port,
        policy,
        connection.connect_timeout,
    )
    .await
    .map_err(|failure| SessionFailure {
        failure,
        address: None,
        encrypted: false,
    })?;
    let client = ClientId::Domain(connection.helo_name.clone());
    let tls = match connection.security {
        SmtpOutboundSecurity::None => None,
        SmtpOutboundSecurity::StartTls | SmtpOutboundSecurity::Tls => Some(
            tls_parameters(connection).map_err(|failure| SessionFailure {
                failure,
                address: None,
                encrypted: false,
            })?,
        ),
    };
    let implicit = tls
        .clone()
        .filter(|_| connection.security == SmtpOutboundSecurity::Tls);
    let (mut smtp, address) = connect(connection, &addresses, &client, implicit)
        .await
        .map_err(|failure| SessionFailure {
            failure,
            address: None,
            encrypted: false,
        })?;
    let fail = |failure: OutboundFailure, encrypted: bool| SessionFailure {
        failure,
        address: Some(address),
        encrypted,
    };
    if connection.security == SmtpOutboundSecurity::StartTls {
        if !smtp.server_info().supports_feature(Extension::StartTls) {
            abandon(smtp).await;
            return Err(fail(
                failure(
                    OutboundFailureKind::Tls,
                    "the server does not offer STARTTLS; delivery stops instead of continuing without encryption",
                ),
                false,
            ));
        }
        let parameters = tls.clone().expect("STARTTLS parameters were built");
        match tokio::time::timeout(
            connection.command_timeout,
            smtp.starttls(parameters, &client),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                abandon(smtp).await;
                let mut failure = classify(&error, Phase::Encrypt);
                if failure.code.is_none() {
                    failure.kind = OutboundFailureKind::Tls;
                }
                return Err(fail(failure, false));
            }
            Err(_) => {
                abandon(smtp).await;
                return Err(fail(
                    timeout(Phase::Encrypt, connection.command_timeout),
                    false,
                ));
            }
        }
    }
    let encrypted = smtp.is_encrypted();
    let mut authenticated = false;
    if let Some(credentials) = &connection.credentials {
        let mechanism = match credentials.mechanism {
            SmtpOutboundAuthentication::Plain => Some(Mechanism::Plain),
            SmtpOutboundAuthentication::Login => Some(Mechanism::Login),
            SmtpOutboundAuthentication::None => None,
        };
        if let Some(mechanism) = mechanism {
            if !smtp.server_info().supports_auth_mechanism(mechanism) {
                abandon(smtp).await;
                return Err(fail(
                    failure(
                        OutboundFailureKind::Authentication,
                        format!("the server does not offer AUTH {mechanism}"),
                    ),
                    encrypted,
                ));
            }
            let secret =
                Credentials::new(credentials.username.clone(), credentials.password.clone());
            match tokio::time::timeout(connection.command_timeout, smtp.auth(&[mechanism], &secret))
                .await
            {
                Ok(Ok(_)) => authenticated = true,
                Ok(Err(error)) => {
                    abandon(smtp).await;
                    return Err(fail(classify(&error, Phase::Authenticate), encrypted));
                }
                Err(_) => {
                    abandon(smtp).await;
                    return Err(fail(
                        timeout(Phase::Authenticate, connection.command_timeout),
                        encrypted,
                    ));
                }
            }
        }
    }
    Ok(Session {
        connection: smtp,
        address,
        encrypted,
        authenticated,
    })
}

/// How long a broken or refused session may take to say goodbye before it is dropped.
const ABANDON_TIMEOUT: Duration = Duration::from_secs(1);

/**
 * Close a session that cannot continue: send `QUIT` without waiting more than a moment for its
 * reply, since a broken or silent server may never answer, then drop the connection.
 */
pub(crate) async fn abandon(mut connection: AsyncSmtpConnection) {
    let _ = tokio::time::timeout(ABANDON_TIMEOUT, connection.abort()).await;
}

impl Session {
    /**
     * End the session politely with `QUIT`, bounded by `limit`, then close the connection.
     */
    pub(crate) async fn close(mut self, limit: Duration) {
        if tokio::time::timeout(limit, self.connection.quit())
            .await
            .is_err()
        {
            abandon(self.connection).await;
        }
    }
}
