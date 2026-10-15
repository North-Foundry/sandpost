use super::*;

/// Preserve environment directory and database overrides above platform defaults.
#[test]
fn storage_environment_overrides() {
    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_DATA_DIRECTORY" => Some("custom data".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(configuration.data_directory, PathBuf::from("custom data"));
    assert_eq!(
        configuration.database_path,
        PathBuf::from("custom data/sandpost.sqlite3")
    );

    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_DATABASE_PATH" => Some("custom.sqlite3".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(configuration.data_directory, default_data_directory());
    assert_eq!(
        configuration.database_path,
        default_data_directory().join("custom.sqlite3")
    );

    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_DATA_DIRECTORY" => Some("custom data".into()),
        "SANDPOST_DATABASE_PATH" => Some("db/mail.sqlite3".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        configuration.database_path,
        PathBuf::from("custom data/db/mail.sqlite3")
    );
}

/// Own a temporary configuration directory without touching the user's real home or environment.
struct ConfigurationDirectory(PathBuf);

impl ConfigurationDirectory {
    /// Create a unique directory for real file-loading tests.
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("sandpost-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }

    /// Return the shared filename within the isolated configuration directory.
    fn configuration_path(&self) -> PathBuf {
        self.0.join("config.toml")
    }
}

impl Drop for ConfigurationDirectory {
    /// Remove only the temporary files created by this test.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Select the shared home file for tests; it is read only when it exists.
fn shared_file(configuration_path: impl Into<PathBuf>) -> ConfigurationFile {
    ConfigurationFile::shared(configuration_path.into())
}

/// Missing files, missing fields, and blank strings select platform defaults without writing files.
#[test]
fn optional_storage_file_defaults() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    let configuration =
        Configuration::from_sources(|_| None, Some(shared_file(configuration_path.clone())))
            .unwrap();
    assert_eq!(configuration.data_directory, default_data_directory());
    assert!(configuration.configuration_path.is_none());
    assert!(!configuration_path.exists());
    for contents in [
        "",
        "[storage]",
        "[storage]\ndata_directory = \"\"",
        "[storage]\ndata_directory = \"   \"",
    ] {
        std::fs::write(&configuration_path, contents).unwrap();
        let configuration =
            Configuration::from_sources(|_| None, Some(shared_file(configuration_path.clone())))
                .unwrap();
        assert_eq!(configuration.data_directory, default_data_directory());
        assert_eq!(
            configuration.database_path,
            configuration.data_directory.join("sandpost.sqlite3")
        );
        assert_eq!(
            configuration.configuration_path.as_deref(),
            Some(configuration_path.as_path())
        );
    }
}

/// Anchor relative file paths to the config folder and preserve absolute paths and spaces.
#[test]
fn storage_file_paths_and_environment_precedence() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    std::fs::write(
        &configuration_path,
        "[storage]\ndata_directory = \"mail data\"",
    )
    .unwrap();
    let configuration =
        Configuration::from_sources(|_| None, Some(shared_file(configuration_path.clone())))
            .unwrap();
    assert_eq!(configuration.data_directory, directory.0.join("mail data"));
    assert_eq!(
        configuration.database_path,
        directory.0.join("mail data/sandpost.sqlite3")
    );
    assert!(!configuration.data_directory.exists());

    let configuration = Configuration::from_sources(
        |setting_name| match setting_name {
            "SANDPOST_DATA_DIRECTORY" => Some("environment data".into()),
            _ => None,
        },
        Some(shared_file(configuration_path.clone())),
    )
    .unwrap();
    assert_eq!(
        configuration.data_directory,
        PathBuf::from("environment data")
    );
    assert_eq!(
        configuration.database_path,
        PathBuf::from("environment data/sandpost.sqlite3")
    );

    let configuration = Configuration::from_sources(
        |setting_name| match setting_name {
            "SANDPOST_DATABASE_PATH" => Some("independent.sqlite3".into()),
            _ => None,
        },
        Some(shared_file(configuration_path.clone())),
    )
    .unwrap();
    assert_eq!(configuration.data_directory, directory.0.join("mail data"));
    assert_eq!(
        configuration.database_path,
        directory.0.join("mail data/independent.sqlite3")
    );

    let absolute_database = directory.0.join("external.sqlite3");
    for data_directory in [None, Some("environment data")] {
        let configuration = Configuration::from_sources(
            |setting_name| match setting_name {
                "SANDPOST_DATA_DIRECTORY" => data_directory.map(str::to_owned),
                "SANDPOST_DATABASE_PATH" => Some(absolute_database.to_string_lossy().into_owned()),
                _ => None,
            },
            Some(shared_file(configuration_path.clone())),
        )
        .unwrap();
        assert_eq!(configuration.database_path, absolute_database);
    }

    let absolute_directory = directory.0.join("absolute mail data");
    let escaped_directory = absolute_directory.to_str().unwrap().replace('\\', "\\\\");
    std::fs::write(
        &configuration_path,
        format!("[storage]\ndata_directory = \"{escaped_directory}\"\n"),
    )
    .unwrap();
    let configuration =
        Configuration::from_sources(|_| None, Some(shared_file(configuration_path.clone())))
            .unwrap();
    assert_eq!(configuration.data_directory, absolute_directory);
}

/// Refuse invalid TOML, unsupported keys/types, and unreadable files even with an environment override.
#[test]
fn invalid_storage_files_report_the_path() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    for contents in [
        "[storage]\ndata_directory =",
        "[storage]\ndata_directory = null",
        "[storage]\ndata_directory = 42",
        "[storage]\ndata_directory = false",
        "[storage]\ndirectory = \"mail\"",
        "[storage]\ndata_directory = \"mail\"\ndata_directory = \"other\"",
        "[http]\nlisten_address = \"127.0.0.1:8025\"",
        "storage = \"mail\"",
    ] {
        std::fs::write(&configuration_path, contents).unwrap();
        let error = Configuration::from_sources(
            |name| (name == "SANDPOST_DATA_DIRECTORY").then(|| "override".into()),
            Some(shared_file(configuration_path.clone())),
        )
        .unwrap_err();
        assert!(
            matches!(error, ConfigurationError::File { ref path, .. } if path == &configuration_path),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .contains(&configuration_path.display().to_string())
        );
    }
    std::fs::remove_file(&configuration_path).unwrap();
    std::fs::create_dir(&configuration_path).unwrap();
    assert!(
        Configuration::from_sources(|_| None, Some(shared_file(configuration_path.clone())))
            .is_err()
    );
}

/// Select the configuration file with `SANDPOST_CONFIG`, refusing a blank setting.
#[test]
fn configuration_file_selection() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    let selected = configuration_file(&|setting_name: &str| {
        (setting_name == "SANDPOST_CONFIG").then(|| configuration_path.display().to_string())
    })
    .unwrap()
    .unwrap();
    assert_eq!(selected.path, configuration_path);
    assert!(selected.required);

    let error = configuration_file(&|setting_name: &str| {
        (setting_name == "SANDPOST_CONFIG").then(|| "   ".into())
    })
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigurationError::Invalid {
                setting_name: "SANDPOST_CONFIG",
                ..
            }
        ),
        "{error}"
    );

    let shared = configuration_file(&|_: &str| None).unwrap().unwrap();
    assert!(!shared.required);
    if let Some(home_directory) = dirs::home_dir() {
        assert_eq!(shared.path, home_directory.join(".sandpost/config.toml"));
    }
}

/// Read the file named by `SANDPOST_CONFIG`, keep environment precedence, and refuse a missing path.
#[test]
fn explicit_configuration_file_is_required_and_effective() {
    let directory = ConfigurationDirectory::new();
    let explicit_path = directory.0.join("sandpost.toml");
    std::fs::write(
        &explicit_path,
        "[storage]\ndata_directory = \"mail data\"\n",
    )
    .unwrap();
    let explicit_value = explicit_path.display().to_string();

    let configuration = Configuration::from_environment_values(|setting_name| {
        (setting_name == "SANDPOST_CONFIG").then(|| explicit_value.clone())
    })
    .unwrap();
    assert_eq!(
        configuration.configuration_path.as_deref(),
        Some(explicit_path.as_path())
    );
    assert_eq!(configuration.data_directory, directory.0.join("mail data"));

    let configuration = Configuration::from_environment_values(|setting_name| match setting_name {
        "SANDPOST_CONFIG" => Some(explicit_value.clone()),
        "SANDPOST_DATA_DIRECTORY" => Some("environment data".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        configuration.data_directory,
        PathBuf::from("environment data")
    );
    assert_eq!(
        configuration.database_path,
        PathBuf::from("environment data/sandpost.sqlite3")
    );

    let missing_path = directory.0.join("missing.toml");
    let missing_value = missing_path.display().to_string();
    let error = Configuration::from_environment_values(|setting_name| {
        (setting_name == "SANDPOST_CONFIG").then(|| missing_value.clone())
    })
    .unwrap_err();
    assert!(
        matches!(error, ConfigurationError::File { ref path, .. } if path == &missing_path),
        "{error}"
    );
    assert!(error.to_string().contains("SANDPOST_CONFIG"), "{error}");

    let error = Configuration::from_environment_values(|setting_name| {
        (setting_name == "SANDPOST_CONFIG").then(|| "  ".into())
    })
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigurationError::Invalid {
                setting_name: "SANDPOST_CONFIG",
                ..
            }
        ),
        "{error}"
    );
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
        default_data_directory().join("sandpost.sqlite3")
    );
    assert!(configuration.maximum_scope_depth.is_none());
    assert!(configuration.mail_listen_address.is_none());
    assert_eq!(
        configuration.mail_authentication,
        MailAuthenticationSetting::Automatic
    );
    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_DATA_DIRECTORY" => Some("/tmp/test".into()),
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
