//! Building a TLS configuration from PEM data.
use super::support::test_certificate;
use crate::{TlsConfiguration, TlsConfigurationError};

/// A generated certificate and key load from memory and from files.
#[test]
fn certificates_load_from_pem_data_and_files() {
    let certificate = test_certificate();
    let directory = std::env::temp_dir().join(format!("sandpost-smtp-tls-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let certificate_path = directory.join("smtp.crt");
    let private_key_path = directory.join("smtp.key");
    std::fs::write(&certificate_path, &certificate.certificate_pem).unwrap();
    std::fs::write(&private_key_path, &certificate.private_key_pem).unwrap();
    let loaded = TlsConfiguration::from_pem_files(&certificate_path, &private_key_path);
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(loaded.is_ok(), "{loaded:?}");
    assert_eq!(format!("{:?}", loaded.unwrap()), "TlsConfiguration { .. }");
}

/// Missing files, missing certificates, unusable keys, and mismatched pairs are reported.
#[test]
fn unusable_certificates_and_keys_are_rejected() {
    let certificate = test_certificate();
    let other = test_certificate();
    assert!(matches!(
        TlsConfiguration::from_pem_files("/nonexistent/smtp.crt", "/nonexistent/smtp.key"),
        Err(TlsConfigurationError::Unreadable { .. })
    ));
    assert!(matches!(
        TlsConfiguration::from_pem(b"", certificate.private_key_pem.as_bytes()),
        Err(TlsConfigurationError::MissingCertificate)
    ));
    assert!(matches!(
        TlsConfiguration::from_pem(certificate.certificate_pem.as_bytes(), b"not a key"),
        Err(TlsConfigurationError::InvalidPrivateKey(_))
    ));
    assert!(matches!(
        TlsConfiguration::from_pem(
            certificate.certificate_pem.as_bytes(),
            other.private_key_pem.as_bytes()
        ),
        Err(TlsConfigurationError::Rejected(_))
    ));
}
