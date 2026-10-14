use super::*;
use sandpost_application::SmtpAuthenticator;
use sandpost_storage::Storage;
use sandpost_storage_sqlite::SqliteStorage;
use tokio::sync::broadcast;

/// Create a stopped server over an in-memory database with the automatic policy.
async fn test_server() -> MailServer {
    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::memory().await.unwrap());
    let (events, _) = broadcast::channel(16);
    MailServer::new(
        MailDelivery {
            authenticator: SmtpAuthenticator::new(storage.clone()),
            storage,
            events,
        },
        MailAuthenticationSetting::Automatic,
        MailEncryption::plaintext(),
    )
}

/// Reserve a free loopback port by binding and immediately releasing it.
async fn free_port() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
}

/// Report whether a TCP connection to the address succeeds.
async fn accepts(address: SocketAddr) -> bool {
    tokio::net::TcpStream::connect(address).await.is_ok()
}

/// Wait until a retired listener has released its port.
///
/// Retiring only signals the listener task, which drops its socket when it next runs, so the
/// kernel may briefly keep queueing connections for the old port.
async fn released(address: SocketAddr) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while accepts(address).await {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    true
}

/// Binding, moving, and disabling the listener take effect without a restart.
#[tokio::test]
async fn apply_binds_moves_and_stops_the_listener() {
    let server = test_server().await;
    let first = free_port().await;
    let status = server.apply(Some(first)).await;
    assert_eq!(status.state, MailListenerState::Listening);
    assert_eq!(status.bound_address, Some(first));
    assert!(
        !status.authentication_required,
        "loopback stays open locally"
    );
    assert!(accepts(first).await);

    let second = free_port().await;
    let status = server.apply(Some(second)).await;
    assert_eq!(status.bound_address, Some(second));
    assert!(accepts(second).await);
    assert!(released(first).await, "the previous port must be released");

    // Moving back to a just-released port succeeds within the rebind grace period.
    let status = server.apply(Some(first)).await;
    assert_eq!(status.state, MailListenerState::Listening);

    let status = server.apply(None).await;
    assert_eq!(status.state, MailListenerState::Disabled);
    assert!(released(first).await);
    assert_eq!(server.status().await.state, MailListenerState::Disabled);
    server.shutdown().await;
}

/// An occupied port is reported as a failed listener and can be retried by saving again.
#[tokio::test]
async fn occupied_port_is_reported_and_retried() {
    let server = test_server().await;
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let status = server.apply(Some(address)).await;
    assert_eq!(status.state, MailListenerState::Failed);
    assert_eq!(status.bound_address, None);
    assert!(status.error.unwrap().contains("already in use"));
    assert_eq!(server.status().await.state, MailListenerState::Failed);

    drop(occupied);
    let status = server.apply(Some(address)).await;
    assert_eq!(status.state, MailListenerState::Listening);
    server.shutdown().await;
    assert!(released(address).await);
}

/// Startup tolerates an occupied persisted address but fails for an environment override.
#[tokio::test]
async fn startup_fails_only_for_environment_overrides() {
    let server = test_server().await;
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    server
        .start(Some(DesiredListener {
            address,
            source: MailListenerSource::Database,
        }))
        .await
        .unwrap();
    assert_eq!(server.status().await.state, MailListenerState::Failed);

    assert!(
        test_server()
            .await
            .start(Some(DesiredListener {
                address,
                source: MailListenerSource::Environment,
            }))
            .await
            .is_err()
    );
}

/// An environment override keeps its address when the persisted definition changes.
#[tokio::test]
async fn environment_overrides_are_not_rebound() {
    let server = test_server().await;
    let address = free_port().await;
    server
        .start(Some(DesiredListener {
            address,
            source: MailListenerSource::Environment,
        }))
        .await
        .unwrap();
    let status = server.apply(None).await;
    assert_eq!(status.source, MailListenerSource::Environment);
    assert_eq!(status.state, MailListenerState::Listening);
    assert!(accepts(address).await);
    server.shutdown().await;
    let status = server.apply(Some(address)).await;
    assert_eq!(status.state, MailListenerState::Failed);
}

/// The automatic policy requires AUTH everywhere except loopback; explicit settings win.
#[test]
fn authentication_policy_depends_on_the_address_and_setting() {
    let automatic = MailAuthenticationSetting::Automatic;
    for (address, expected) in [
        ("127.0.0.1:1025", AuthenticationPolicy::Optional),
        ("[::1]:1025", AuthenticationPolicy::Optional),
        ("[::ffff:127.0.0.1]:1025", AuthenticationPolicy::Optional),
        ("0.0.0.0:1025", AuthenticationPolicy::Required),
        ("[::]:1025", AuthenticationPolicy::Required),
        ("192.0.2.10:1025", AuthenticationPolicy::Required),
    ] {
        assert_eq!(
            authentication_policy(automatic, address.parse().unwrap()),
            expected,
            "{address}"
        );
    }
    assert_eq!(
        authentication_policy(
            MailAuthenticationSetting::Required,
            "127.0.0.1:1025".parse().unwrap()
        ),
        AuthenticationPolicy::Required
    );
    assert_eq!(
        authentication_policy(
            MailAuthenticationSetting::Optional,
            "0.0.0.0:1025".parse().unwrap()
        ),
        AuthenticationPolicy::Optional
    );
}

/// A self-signed certificate written to temporary PEM files, removed when dropped.
struct CertificateFiles {
    directory: std::path::PathBuf,
    paths: crate::configuration::MailCertificatePaths,
}

impl CertificateFiles {
    /// Generate a certificate for `localhost` and write its PEM files.
    fn generate() -> Self {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let directory =
            std::env::temp_dir().join(format!("sandpost-mail-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let paths = crate::configuration::MailCertificatePaths {
            certificate_chain: directory.join("smtp.crt"),
            private_key: directory.join("smtp.key"),
        };
        std::fs::write(&paths.certificate_chain, certified.cert.pem()).unwrap();
        std::fs::write(&paths.private_key, certified.signing_key.serialize_pem()).unwrap();
        Self { directory, paths }
    }

    /// Load encryption settings for these files with the given setting and mode.
    fn encryption(
        &self,
        setting: MailEncryptionSetting,
        mode: MailEncryptionMode,
    ) -> MailEncryption {
        let mut configuration = Configuration::from_values(|_| None).unwrap();
        configuration.mail_encryption = setting;
        configuration.mail_encryption_mode = mode;
        configuration.mail_certificate = Some(self.paths.clone());
        MailEncryption::load(&configuration).unwrap()
    }
}

impl Drop for CertificateFiles {
    /// Remove the temporary certificate files.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// TLS follows the setting, the mode, and whether the address is loopback.
#[test]
fn encryption_depends_on_setting_mode_and_address() {
    let files = CertificateFiles::generate();
    let loopback: SocketAddr = "127.0.0.1:587".parse().unwrap();
    let network: SocketAddr = "0.0.0.0:587".parse().unwrap();
    let automatic = files.encryption(
        MailEncryptionSetting::Automatic,
        MailEncryptionMode::StartTls,
    );
    assert_eq!(
        automatic.describe(loopback),
        MailListenerEncryption::StartTlsOptional
    );
    assert_eq!(
        automatic.describe(network),
        MailListenerEncryption::StartTlsRequired
    );
    let required = files.encryption(
        MailEncryptionSetting::Required,
        MailEncryptionMode::StartTls,
    );
    assert_eq!(
        required.describe(loopback),
        MailListenerEncryption::StartTlsRequired
    );
    let implicit = files.encryption(
        MailEncryptionSetting::Automatic,
        MailEncryptionMode::Implicit,
    );
    assert_eq!(implicit.describe(network), MailListenerEncryption::Implicit);
    let disabled = files.encryption(
        MailEncryptionSetting::Disabled,
        MailEncryptionMode::StartTls,
    );
    assert_eq!(disabled.describe(network), MailListenerEncryption::None);
    assert_eq!(
        MailEncryption::load(&Configuration::from_values(|_| None).unwrap())
            .unwrap()
            .describe(network),
        MailListenerEncryption::None,
        "no certificate, no TLS"
    );
}

/// An unreadable certificate fails startup with the reason.
#[test]
fn unusable_certificates_fail_to_load() {
    let mut configuration = Configuration::from_values(|_| None).unwrap();
    configuration.mail_certificate = Some(crate::configuration::MailCertificatePaths {
        certificate_chain: "/nonexistent/smtp.crt".into(),
        private_key: "/nonexistent/smtp.key".into(),
    });
    let error = MailEncryption::load(&configuration).err().unwrap();
    assert!(
        error.to_string().contains("SMTP TLS certificate"),
        "{error}"
    );
}

/// A bound listener offers STARTTLS and reports it in its status.
#[tokio::test]
async fn bound_listener_offers_start_tls_and_reports_it() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let files = CertificateFiles::generate();
    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::memory().await.unwrap());
    let (events, _) = broadcast::channel(16);
    let server = MailServer::new(
        MailDelivery {
            authenticator: SmtpAuthenticator::new(storage.clone()),
            storage,
            events,
        },
        MailAuthenticationSetting::Automatic,
        files.encryption(
            MailEncryptionSetting::Automatic,
            MailEncryptionMode::StartTls,
        ),
    );
    let address = free_port().await;
    let status = server.apply(Some(address)).await;
    assert_eq!(status.encryption, MailListenerEncryption::StartTlsOptional);
    let mut client = BufReader::new(tokio::net::TcpStream::connect(address).await.unwrap());
    let mut line = String::new();
    client.read_line(&mut line).await.unwrap();
    client
        .get_mut()
        .write_all(b"EHLO client\r\n")
        .await
        .unwrap();
    let mut capabilities = Vec::new();
    loop {
        line.clear();
        client.read_line(&mut line).await.unwrap();
        capabilities.push(line.trim_end().to_owned());
        if line.as_bytes().get(3) != Some(&b'-') {
            break;
        }
    }
    assert!(
        capabilities.contains(&"250-STARTTLS".to_owned()),
        "{capabilities:?}"
    );
    drop(client);
    server.shutdown().await;
}
