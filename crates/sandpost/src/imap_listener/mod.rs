//! Bind and serve the single IMAP service for every user's IMAP accounts.
//!
//! The listener is configured from TOML and environment overrides at startup. Its TLS policy follows the
//! configuration: implicit TLS or STARTTLS with a certificate, required STARTTLS on network
//! addresses, and clear text only on loopback unless explicitly allowed. The certificate is loaded
//! by [`prepare`] before storage opens; a certificate or bind failure stops startup, like the
//! HTTP listener.
use crate::configuration::{ImapConfiguration, ImapEncryptionMode, ImapEncryptionSetting};
use crate::imap_backend::ImapAccounts;
use crate::lifecycle::RuntimeError;
use crate::network::{connection_address, connection_host};
use sandpost_application::{Application, ImapSecurity, ImapServerDetails};
use sandpost_imap::{ImapServer, Limits, TlsConfiguration, TransportSecurity};
use std::net::SocketAddr;
use tokio::net::TcpListener;

/// The enabled IMAP listener's address and loaded transport security, ready to bind.
pub(crate) struct PreparedImap {
    address: SocketAddr,
    transport_security: TransportSecurity,
    security: ImapSecurity,
}

/// A bound IMAP listener ready to serve.
pub(crate) struct BoundImap {
    pub listener: TcpListener,
    pub server: ImapServer<ImapAccounts>,
    pub details: ImapServerDetails,
}

/// Decide transport security for the configured address.
fn transport_security(
    configuration: &ImapConfiguration,
    address: SocketAddr,
) -> Result<(TransportSecurity, ImapSecurity), RuntimeError> {
    let certificate = match (&configuration.certificate, configuration.encryption) {
        (_, ImapEncryptionSetting::Disabled) | (None, _) => None,
        (Some(paths), _) => Some(
            TlsConfiguration::from_pem_files(&paths.certificate_chain, &paths.private_key)
                .map_err(|error| format!("IMAP TLS certificate: {error}"))?,
        ),
    };
    let Some(configuration_tls) = certificate else {
        return Ok((TransportSecurity::Plaintext, ImapSecurity::None));
    };
    Ok(match configuration.encryption_mode {
        ImapEncryptionMode::Implicit => (
            TransportSecurity::Implicit {
                configuration: configuration_tls,
            },
            ImapSecurity::Implicit,
        ),
        ImapEncryptionMode::StartTls => {
            let required = configuration.encryption == ImapEncryptionSetting::Required
                || !address.ip().to_canonical().is_loopback();
            (
                TransportSecurity::StartTls {
                    configuration: configuration_tls,
                    required,
                },
                if required {
                    ImapSecurity::StartTlsRequired
                } else {
                    ImapSecurity::StartTlsOptional
                },
            )
        }
    })
}

/// Load the enabled listener's certificate before storage opens, so a bad path fails startup
/// without initializing data; `None` when IMAP is disabled.
pub(crate) fn prepare(
    configuration: &ImapConfiguration,
) -> Result<Option<PreparedImap>, RuntimeError> {
    let Some(address) = configuration.listen_address else {
        return Ok(None);
    };
    let (transport_security, security) = transport_security(configuration, address)?;
    Ok(Some(PreparedImap {
        address,
        transport_security,
        security,
    }))
}

/// Bind the IMAP listener when configured and describe it to the application.
pub(crate) async fn bind(
    configuration: &ImapConfiguration,
    prepared: Option<PreparedImap>,
    application: &Application,
) -> Result<Option<BoundImap>, RuntimeError> {
    let Some(PreparedImap {
        address,
        transport_security,
        security,
    }) = prepared
    else {
        application.set_imap_server(None);
        return Ok(None);
    };
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| format!("IMAP listener {address}: {error}"))?;
    let bound = listener.local_addr()?;
    let details = ImapServerDetails {
        enabled: true,
        host: Some(
            configuration
                .advertised_host
                .clone()
                .unwrap_or_else(|| connection_host(connection_address(bound))),
        ),
        port: Some(bound.port()),
        security,
        listen_address: Some(bound),
    };
    application.set_imap_server(Some(details.clone()));
    let server = ImapServer::new(ImapAccounts {
        service: application.imap_service(),
    })
    .transport_security(transport_security)
    .limits(Limits {
        maximum_connection_count: configuration.maximum_connection_count,
        ..Limits::default()
    })
    .greeting("Sandpost IMAP");
    tracing::info!(listen = %bound, security = ?security, "IMAP listener bound");
    Ok(Some(BoundImap {
        listener,
        server,
        details,
    }))
}

#[cfg(test)]
mod tests;
