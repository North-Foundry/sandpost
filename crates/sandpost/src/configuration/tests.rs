use super::*;

/// Resolve the home default, platform fallback, and independent storage overrides.
#[test]
fn home_directory_defaults_and_storage_overrides() {
    for (values, expected_directory, expected_database) in [
        (
            vec![("HOME", "/home/test user")],
            "/home/test user/.sandpost",
            "/home/test user/.sandpost/sandpost.sqlite3",
        ),
        (
            vec![("USERPROFILE", "/users/test")],
            "/users/test/.sandpost",
            "/users/test/.sandpost/sandpost.sqlite3",
        ),
        (
            vec![("HOME", ""), ("USERPROFILE", "/users/test")],
            "/users/test/.sandpost",
            "/users/test/.sandpost/sandpost.sqlite3",
        ),
        (
            vec![("HOME", "/home/test"), ("USERPROFILE", "/users/test")],
            "/home/test/.sandpost",
            "/home/test/.sandpost/sandpost.sqlite3",
        ),
        (
            vec![("HOME", "/home/test"), ("SANDPOST_DATA_DIR", "/srv/mail")],
            "/srv/mail",
            "/srv/mail/sandpost.sqlite3",
        ),
        (
            vec![
                ("HOME", "/home/test"),
                ("SANDPOST_DATABASE_PATH", "/srv/custom.sqlite3"),
            ],
            "/home/test/.sandpost",
            "/srv/custom.sqlite3",
        ),
        (vec![], "data", "data/sandpost.sqlite3"),
    ] {
        let configuration = Configuration::from_values(|setting_name| {
            values
                .iter()
                .find(|(name, _)| *name == setting_name)
                .map(|(_, value)| (*value).to_owned())
        })
        .unwrap();
        assert_eq!(
            configuration.data_directory,
            PathBuf::from(expected_directory)
        );
        assert_eq!(
            configuration.database_path,
            PathBuf::from(expected_database)
        );
    }
}

/// Verify defaults, environment overrides, and invalid setting errors.
#[test]
fn defaults_overrides_and_invalid_values() {
    let configuration = Configuration::from_values(|_| None).unwrap();
    assert_eq!(
        configuration.web_listen_address.to_string(),
        "127.0.0.1:8025"
    );
    assert_eq!(
        configuration.database_path,
        PathBuf::from("data/sandpost.sqlite3")
    );
    assert!(configuration.maximum_scope_depth.is_none());
    assert!(configuration.mail_listen_address.is_none());
    assert_eq!(
        configuration.mail_authentication,
        MailAuthenticationSetting::Automatic
    );
    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_DATA_DIR" => Some("/tmp/test".into()),
        "SANDPOST_MAX_SCOPE_DEPTH" => Some("0".into()),
        "SANDPOST_SMTP_LISTEN" => Some("127.0.0.1:2525".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(configuration.maximum_scope_depth, Some(0));
    assert_eq!(
        configuration.mail_listen_address,
        Some("127.0.0.1:2525".parse().unwrap())
    );
    assert_eq!(
        configuration.database_path,
        PathBuf::from("/tmp/test/sandpost.sqlite3")
    );
    assert!(
        Configuration::from_values(
            |setting_name| (setting_name == "SANDPOST_HTTP_LISTEN").then(|| "invalid".into())
        )
        .is_err()
    );
    assert!(
        Configuration::from_values(
            |setting_name| (setting_name == "SANDPOST_SMTP_LISTEN").then(|| "invalid".into())
        )
        .is_err()
    );
}

/// Parse the SMTP AUTH requirement and refuse the retired endpoint settings.
#[test]
fn smtp_authentication_setting_and_retired_listeners() {
    for (value, expected) in [
        ("auto", MailAuthenticationSetting::Automatic),
        ("Required", MailAuthenticationSetting::Required),
        (" optional ", MailAuthenticationSetting::Optional),
    ] {
        let configuration = Configuration::from_values(|setting_name| {
            (setting_name == "SANDPOST_SMTP_AUTH").then(|| value.into())
        })
        .unwrap();
        assert_eq!(configuration.mail_authentication, expected, "{value}");
    }
    assert!(
        Configuration::from_values(
            |setting_name| (setting_name == "SANDPOST_SMTP_AUTH").then(|| "anonymous".into())
        )
        .is_err()
    );
    for retired in ["SANDPOST_SMTP_ENDPOINTS", "SANDPOST_SMTP_ENDPOINT"] {
        let error = Configuration::from_values(|setting_name| {
            (setting_name == retired).then(|| "[]".into())
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("endpoints were removed"),
            "{error}"
        );
    }
}

/// Parse IMAP listener, TLS, certificate fallback, host, and connection settings, refusing
/// clear-text password logins on network addresses unless explicitly allowed.
#[test]
fn imap_settings() {
    let with = |pairs: &'static [(&'static str, &'static str)]| {
        Configuration::from_values(move |setting_name| {
            pairs
                .iter()
                .find(|(name, _)| *name == setting_name)
                .map(|(_, value)| (*value).to_owned())
        })
    };
    let defaults = with(&[]).unwrap().imap;
    assert_eq!(
        defaults.listen_address,
        Some("127.0.0.1:1143".parse().unwrap())
    );
    assert_eq!(defaults.encryption, ImapEncryptionSetting::Automatic);
    assert_eq!(defaults.encryption_mode, ImapEncryptionMode::Implicit);
    assert!(defaults.certificate.is_none());
    assert_eq!(defaults.maximum_connection_count, 64);
    assert_eq!(
        with(&[("SANDPOST_IMAP_LISTEN", "OFF")])
            .unwrap()
            .imap
            .listen_address,
        None
    );
    let inherited = with(&[
        ("SANDPOST_SMTP_TLS_CERTIFICATE", "/etc/sandpost/mail.crt"),
        ("SANDPOST_SMTP_TLS_PRIVATE_KEY", "/etc/sandpost/mail.key"),
        ("SANDPOST_IMAP_LISTEN", "0.0.0.0:993"),
        ("SANDPOST_IMAP_HOST", "mail.example.test"),
        ("SANDPOST_IMAP_MAX_CONNECTIONS", "8"),
    ])
    .unwrap()
    .imap;
    assert_eq!(
        inherited.certificate.unwrap().certificate_chain,
        std::path::PathBuf::from("/etc/sandpost/mail.crt")
    );
    assert_eq!(
        inherited.advertised_host.as_deref(),
        Some("mail.example.test")
    );
    assert_eq!(inherited.maximum_connection_count, 8);
    let opted_out = with(&[
        ("SANDPOST_IMAP_LISTEN", "0.0.0.0:1143"),
        ("SANDPOST_IMAP_TLS", "off"),
    ])
    .unwrap()
    .imap;
    assert_eq!(opted_out.encryption, ImapEncryptionSetting::Disabled);
    for invalid in [
        &[("SANDPOST_IMAP_LISTEN", "0.0.0.0:1143")][..],
        &[("SANDPOST_IMAP_LISTEN", "nonsense")][..],
        &[("SANDPOST_IMAP_TLS", "always")][..],
        &[("SANDPOST_IMAP_TLS", "required")][..],
        &[("SANDPOST_IMAP_TLS_MODE", "ssl")][..],
        &[("SANDPOST_IMAP_TLS_CERTIFICATE", "/only/one.crt")][..],
        &[("SANDPOST_IMAP_MAX_CONNECTIONS", "0")][..],
        &[("SANDPOST_IMAP_HOST", "bad host")][..],
    ] {
        assert!(with(invalid).is_err(), "{invalid:?}");
    }
}

/// Parse the SMTP TLS requirement, mode, and certificate paths, and refuse incomplete setups.
#[test]
fn smtp_encryption_settings() {
    let defaults = Configuration::from_values(|_| None).unwrap();
    assert_eq!(defaults.mail_encryption, MailEncryptionSetting::Automatic);
    assert_eq!(defaults.mail_encryption_mode, MailEncryptionMode::StartTls);
    assert!(defaults.mail_certificate.is_none());
    let with = |pairs: &'static [(&'static str, &'static str)]| {
        Configuration::from_values(move |setting_name| {
            pairs
                .iter()
                .find(|(name, _)| *name == setting_name)
                .map(|(_, value)| (*value).to_owned())
        })
    };
    let configured = with(&[
        ("SANDPOST_SMTP_TLS", " Required "),
        ("SANDPOST_SMTP_TLS_MODE", "IMPLICIT"),
        ("SANDPOST_SMTP_TLS_CERTIFICATE", "/etc/sandpost/smtp.crt"),
        ("SANDPOST_SMTP_TLS_PRIVATE_KEY", "/etc/sandpost/smtp.key"),
    ])
    .unwrap();
    assert_eq!(configured.mail_encryption, MailEncryptionSetting::Required);
    assert_eq!(
        configured.mail_encryption_mode,
        MailEncryptionMode::Implicit
    );
    assert_eq!(
        configured.mail_certificate,
        Some(MailCertificatePaths {
            certificate_chain: "/etc/sandpost/smtp.crt".into(),
            private_key: "/etc/sandpost/smtp.key".into(),
        })
    );
    assert_eq!(
        with(&[
            ("SANDPOST_SMTP_TLS", "off"),
            ("SANDPOST_SMTP_TLS_MODE", "implicit")
        ])
        .unwrap()
        .mail_encryption,
        MailEncryptionSetting::Disabled,
        "disabled TLS needs no certificate"
    );
    for invalid in [
        &[("SANDPOST_SMTP_TLS", "always")][..],
        &[("SANDPOST_SMTP_TLS_MODE", "smtps")][..],
        &[("SANDPOST_SMTP_TLS_CERTIFICATE", "/etc/sandpost/smtp.crt")][..],
        &[("SANDPOST_SMTP_TLS", "required")][..],
        &[("SANDPOST_SMTP_TLS_MODE", "implicit")][..],
    ] {
        assert!(with(invalid).is_err(), "{invalid:?}");
    }
}

/// Revalidate SMTP TLS requirements after public configuration fields are edited.
#[test]
fn validate_edited_smtp_tls_settings() {
    let mut configuration = Configuration::from_values(|_| None).unwrap();
    configuration.mail_encryption = MailEncryptionSetting::Required;
    assert_eq!(
        configuration.validate().unwrap_err().to_string(),
        "invalid SANDPOST_SMTP_TLS: required or implicit TLS needs SANDPOST_SMTP_TLS_CERTIFICATE and SANDPOST_SMTP_TLS_PRIVATE_KEY"
    );

    configuration.mail_encryption = MailEncryptionSetting::Automatic;
    configuration.mail_encryption_mode = MailEncryptionMode::Implicit;
    assert!(configuration.validate().is_err());

    configuration.mail_encryption = MailEncryptionSetting::Disabled;
    configuration.validate().unwrap();
}

/// Revalidate IMAP TLS, listener, and connection limits after typed configuration edits.
#[test]
fn validate_edited_imap_tls_and_connection_settings() {
    let mut configuration = Configuration::from_values(|_| None).unwrap();
    configuration.imap.encryption = ImapEncryptionSetting::Required;
    assert_eq!(
        configuration.validate().unwrap_err().to_string(),
        "invalid SANDPOST_IMAP_TLS: required TLS needs SANDPOST_IMAP_TLS_CERTIFICATE and SANDPOST_IMAP_TLS_PRIVATE_KEY (or the SMTP certificate)"
    );

    configuration.imap.encryption = ImapEncryptionSetting::Automatic;
    configuration.imap.listen_address = Some("0.0.0.0:0".parse().unwrap());
    assert!(configuration.validate().is_err());
    configuration.imap.encryption = ImapEncryptionSetting::Disabled;
    configuration.validate().unwrap();

    configuration.imap.encryption = ImapEncryptionSetting::Automatic;
    configuration.imap.listen_address = None;
    for count in [0, 10_001] {
        configuration.imap.maximum_connection_count = count;
        assert_eq!(
            configuration.validate().unwrap_err().to_string(),
            "invalid SANDPOST_IMAP_MAX_CONNECTIONS: expected a number from 1 to 10000"
        );
    }
    for count in [1, 10_000] {
        configuration.imap.maximum_connection_count = count;
        configuration.validate().unwrap();
    }
}

/// Preserve the advertised-host restrictions and recognize IPv4-mapped loopback addresses.
#[test]
fn validate_edited_imap_host_and_mapped_loopback() {
    let mut configuration = Configuration::from_values(|_| None).unwrap();
    configuration.imap.listen_address = Some("[::ffff:127.0.0.1]:0".parse().unwrap());
    configuration.validate().unwrap();

    configuration.imap.listen_address = Some("[::ffff:192.0.2.1]:0".parse().unwrap());
    assert!(configuration.validate().is_err());

    configuration.imap.listen_address = None;
    for host in ["bad host", "bad/host", &"a".repeat(254)] {
        configuration.imap.advertised_host = Some(host.into());
        assert_eq!(
            configuration.validate().unwrap_err().to_string(),
            "invalid SANDPOST_IMAP_HOST: expected a host name or address"
        );
    }
    configuration.imap.advertised_host = Some("mail.example.test".into());
    configuration.validate().unwrap();
}
