//! Transport security: server certificates, how TLS is offered, and the upgradable session stream.
use std::{
    fmt,
    io::ErrorKind,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        crypto::ring,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    },
    server::TlsStream,
};

/// A server certificate chain and private key, ready to secure SMTP sessions.
///
/// Built once from PEM data; cloning shares the same configuration.
///
/// ```no_run
/// let configuration = sandpost_smtp::TlsConfiguration::from_pem_files(
///     "/etc/sandpost/smtp.crt",
///     "/etc/sandpost/smtp.key",
/// )?;
/// # Ok::<(), sandpost_smtp::TlsConfigurationError>(())
/// ```
#[derive(Clone)]
pub struct TlsConfiguration {
    acceptor: TlsAcceptor,
}

impl TlsConfiguration {
    /// Build a configuration from a PEM certificate chain (leaf first) and a PEM private key.
    pub fn from_pem(
        certificate_chain: &[u8],
        private_key: &[u8],
    ) -> Result<Self, TlsConfigurationError> {
        let certificates = CertificateDer::pem_slice_iter(certificate_chain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| TlsConfigurationError::InvalidCertificate(error.to_string()))?;
        if certificates.is_empty() {
            return Err(TlsConfigurationError::MissingCertificate);
        }
        let private_key = PrivateKeyDer::from_pem_slice(private_key)
            .map_err(|error| TlsConfigurationError::InvalidPrivateKey(error.to_string()))?;
        let configuration = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|error| TlsConfigurationError::Rejected(error.to_string()))?
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .map_err(|error| TlsConfigurationError::Rejected(error.to_string()))?;
        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(configuration)),
        })
    }

    /// Read a PEM certificate chain and private key from files and build a configuration.
    pub fn from_pem_files(
        certificate_chain: impl AsRef<Path>,
        private_key: impl AsRef<Path>,
    ) -> Result<Self, TlsConfigurationError> {
        let read = |path: &Path| {
            std::fs::read(path).map_err(|source| TlsConfigurationError::Unreadable {
                path: path.to_owned(),
                source,
            })
        };
        Self::from_pem(
            &read(certificate_chain.as_ref())?,
            &read(private_key.as_ref())?,
        )
    }

    /// The acceptor that performs server-side handshakes with this configuration.
    pub(crate) fn acceptor(&self) -> &TlsAcceptor {
        &self.acceptor
    }
}

impl fmt::Debug for TlsConfiguration {
    /// Identify the type without exposing key material.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TlsConfiguration { .. }")
    }
}

/// Why a certificate and private key could not become a [`TlsConfiguration`].
#[derive(Debug, Error)]
pub enum TlsConfigurationError {
    /// A certificate or key file could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The certificate data contains no PEM certificate.
    #[error("no PEM certificate found")]
    MissingCertificate,
    /// The certificate data is not valid PEM.
    #[error("invalid PEM certificate: {0}")]
    InvalidCertificate(String),
    /// The key data contains no usable PEM private key.
    #[error("no usable PEM private key: {0}")]
    InvalidPrivateKey(String),
    /// TLS rejected the pair, for example because the key does not match the certificate.
    #[error("certificate and private key were rejected: {0}")]
    Rejected(String),
}

/// How an [`SmtpServer`](crate::SmtpServer) protects sessions in transit.
#[derive(Debug, Clone, Default)]
pub enum TransportSecurity {
    /// No TLS: STARTTLS is not offered and everything, including AUTH, travels in clear text.
    #[default]
    Plaintext,
    /// Sessions start in clear text and may upgrade with STARTTLS (RFC 3207).
    ///
    /// When `required`, AUTH is neither advertised nor accepted (`538`) and `MAIL FROM` is
    /// refused (`530`) until the session has upgraded.
    StartTls {
        configuration: TlsConfiguration,
        required: bool,
    },
    /// Every connection performs a TLS handshake before the greeting (RFC 8314).
    Implicit { configuration: TlsConfiguration },
}

/// A session's connection, which STARTTLS can upgrade in place.
pub(crate) enum SessionStream<Stream> {
    Plain(Stream),
    Encrypted(Box<TlsStream<Stream>>),
    /// Momentarily empty while the plain stream is handed to the TLS handshake; any I/O fails.
    Upgrading,
}

impl<Stream> SessionStream<Stream> {
    /// Report whether the connection is protected by TLS.
    pub(crate) fn is_encrypted(&self) -> bool {
        matches!(self, Self::Encrypted(_))
    }
}

/// The error for I/O attempted while the stream is being upgraded.
fn upgrading_error() -> std::io::Error {
    std::io::Error::new(
        ErrorKind::NotConnected,
        "the stream is being upgraded to TLS",
    )
}

impl<Stream: AsyncRead + AsyncWrite + Unpin> AsyncRead for SessionStream<Stream> {
    /// Read from whichever connection is current.
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_read(context, buffer),
            Self::Encrypted(stream) => Pin::new(stream).poll_read(context, buffer),
            Self::Upgrading => Poll::Ready(Err(upgrading_error())),
        }
    }
}

impl<Stream: AsyncRead + AsyncWrite + Unpin> AsyncWrite for SessionStream<Stream> {
    /// Write to whichever connection is current.
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_write(context, buffer),
            Self::Encrypted(stream) => Pin::new(stream).poll_write(context, buffer),
            Self::Upgrading => Poll::Ready(Err(upgrading_error())),
        }
    }

    /// Flush whichever connection is current, including buffered TLS records.
    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_flush(context),
            Self::Encrypted(stream) => Pin::new(stream).poll_flush(context),
            Self::Upgrading => Poll::Ready(Err(upgrading_error())),
        }
    }

    /// Shut down whichever connection is current, sending a TLS close notification if encrypted.
    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_shutdown(context),
            Self::Encrypted(stream) => Pin::new(stream).poll_shutdown(context),
            Self::Upgrading => Poll::Ready(Ok(())),
        }
    }
}
