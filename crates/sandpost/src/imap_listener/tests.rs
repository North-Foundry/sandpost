use super::*;
use crate::configuration::MailCertificatePaths;

/// Write a self-signed certificate pair into a temporary directory.
fn certificate_files() -> (std::path::PathBuf, MailCertificatePaths) {
    let directory =
        std::env::temp_dir().join(format!("sandpost-imap-tls-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate_chain = directory.join("imap.crt");
    let private_key = directory.join("imap.key");
    std::fs::write(&certificate_chain, certified.cert.pem()).unwrap();
    std::fs::write(&private_key, certified.signing_key.serialize_pem()).unwrap();
    (
        directory,
        MailCertificatePaths {
            certificate_chain,
            private_key,
        },
    )
}

/// Build a configuration for policy tests.
fn configuration(
    encryption: ImapEncryptionSetting,
    encryption_mode: ImapEncryptionMode,
    certificate: Option<MailCertificatePaths>,
) -> ImapConfiguration {
    ImapConfiguration {
        listen_address: None,
        encryption,
        encryption_mode,
        certificate,
        advertised_host: None,
        maximum_connection_count: 4,
    }
}

/// The TLS policy follows the setting, mode, certificate, and address.
#[test]
fn transport_security_policy() {
    let (directory, paths) = certificate_files();
    let loopback: SocketAddr = "127.0.0.1:1143".parse().unwrap();
    let network: SocketAddr = "0.0.0.0:143".parse().unwrap();
    let describe = |setting, mode, certificate: Option<MailCertificatePaths>, address| {
        transport_security(&configuration(setting, mode, certificate), address)
            .unwrap()
            .1
    };
    use ImapEncryptionMode::{Implicit, StartTls};
    use ImapEncryptionSetting::{Automatic, Disabled, Required};
    assert_eq!(
        describe(Automatic, Implicit, None, loopback),
        ImapSecurity::None
    );
    assert_eq!(
        describe(Automatic, Implicit, Some(paths.clone()), network),
        ImapSecurity::Implicit
    );
    assert_eq!(
        describe(Automatic, StartTls, Some(paths.clone()), loopback),
        ImapSecurity::StartTlsOptional
    );
    assert_eq!(
        describe(Automatic, StartTls, Some(paths.clone()), network),
        ImapSecurity::StartTlsRequired
    );
    assert_eq!(
        describe(Required, StartTls, Some(paths.clone()), loopback),
        ImapSecurity::StartTlsRequired
    );
    assert_eq!(
        describe(Disabled, Implicit, Some(paths), network),
        ImapSecurity::None
    );
    let _ = std::fs::remove_dir_all(directory);
}

/// The actual STARTTLS transport enforces encryption on network addresses and when required.
#[test]
fn start_tls_transport_enforces_the_configured_policy() {
    let (directory, paths) = certificate_files();
    for (setting, address, expected_required) in [
        (ImapEncryptionSetting::Automatic, "127.0.0.1:143", false),
        (ImapEncryptionSetting::Automatic, "0.0.0.0:143", true),
        (ImapEncryptionSetting::Required, "127.0.0.1:143", true),
    ] {
        let (transport, _) = transport_security(
            &configuration(setting, ImapEncryptionMode::StartTls, Some(paths.clone())),
            address.parse().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            transport,
            TransportSecurity::StartTls { required, .. } if required == expected_required
        ));
    }
    let _ = std::fs::remove_dir_all(directory);
}
