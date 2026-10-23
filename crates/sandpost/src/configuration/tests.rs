use super::*;

/// Frontend CLI values override TOML before URL validation; omission keeps embedded serving.
#[test]
fn frontend_proxy_configuration_and_cli_precedence() {
    assert!(
        Configuration::from_values(|_| None)
            .unwrap()
            .frontend_proxy_url
            .is_none()
    );
    let directory = ConfigurationDirectory::new();
    let path = directory.configuration_path();
    std::fs::write(&path, "[frontend]\nproxy_url = 'http://frontend:8080'\n").unwrap();
    let configuration =
        Configuration::from_sources(|_| None, Some(shared_file(path.clone()))).unwrap();
    assert_eq!(
        configuration.frontend_proxy_url.as_deref(),
        Some("http://frontend:8080")
    );
    let configuration = Configuration::from_sources_with_frontend_proxy_url(
        |_| None,
        Some(shared_file(path.clone())),
        Some("http://localhost:5173"),
    )
    .unwrap();
    assert_eq!(
        configuration.frontend_proxy_url.as_deref(),
        Some("http://localhost:5173")
    );
    std::fs::write(&path, "[frontend]\nproxy_url = 'ftp://frontend:8080'\n").unwrap();
    assert!(Configuration::from_sources(|_| None, Some(shared_file(path.clone()))).is_err());
    let configuration = Configuration::from_sources_with_frontend_proxy_url(
        |_| None,
        Some(shared_file(path.clone())),
        Some("http://localhost:5173"),
    )
    .unwrap();
    assert_eq!(
        configuration.frontend_proxy_url.as_deref(),
        Some("http://localhost:5173")
    );
    std::fs::write(&path, "[frontend]\nproxy_url = 'http://frontend:8080'\n").unwrap();
    assert!(
        Configuration::from_sources_with_frontend_proxy_url(
            |_| None,
            Some(shared_file(path.clone())),
            Some("ftp://localhost:5173"),
        )
        .is_err()
    );
}

/// Invalid frontend section fields and URL strings fail without silently selecting embedded assets.
#[test]
fn invalid_frontend_proxy_settings_are_rejected() {
    let directory = ConfigurationDirectory::new();
    let path = directory.configuration_path();
    for contents in [
        "[frontend]\nproxy_url = 42\n",
        "[frontend]\nproxy_url = ''\n",
        "[frontend]\nproxy_url = 'http://frontend:8080/path'\n",
        "[frontend]\nproxy_url = 'http://frontend:8080/?query=1'\n",
        "[frontend]\nproxy_url = 'http://user:password@frontend:8080'\n",
        "[frontend]\nunknown = true\n",
    ] {
        std::fs::write(&path, contents).unwrap();
        assert!(
            Configuration::from_sources(|_| None, Some(shared_file(path.clone()))).is_err(),
            "{contents}"
        );
    }
}

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

/// Blank storage environment values behave as unset instead of selecting the working directory.
#[test]
fn blank_storage_environment_values_select_defaults() {
    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_DATA_DIRECTORY" | "SANDPOST_DATABASE_PATH" => Some("  ".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(configuration.data_directory, default_data_directory());
    assert_eq!(
        configuration.database_path,
        default_data_directory().join("sandpost.sqlite3")
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
    assert_eq!(configuration.maximum_scope_depth, Some(2));
    assert_eq!(
        configuration.mail_listen_address,
        "127.0.0.1:1025".parse().unwrap()
    );
    assert_eq!(
        configuration.mail_authentication,
        MailAuthenticationSetting::Automatic
    );
    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_DATA_DIRECTORY" => Some("/tmp/test".into()),
        "SANDPOST_MAX_SCOPE_DEPTH" => Some("0".into()),
        "SANDPOST_SMTP_PORT" => Some("2525".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(configuration.maximum_scope_depth, Some(0));
    assert_eq!(
        configuration.mail_listen_address,
        "127.0.0.1:2525".parse().unwrap()
    );
    assert_eq!(
        configuration.database_path,
        PathBuf::from("/tmp/test/sandpost.sqlite3")
    );
    assert!(
        Configuration::from_values(
            |setting_name| (setting_name == "SANDPOST_HTTP_BIND_ADDRESS").then(|| "invalid".into())
        )
        .is_err()
    );
    assert!(
        Configuration::from_values(
            |setting_name| (setting_name == "SANDPOST_SMTP_BIND_ADDRESS").then(|| "invalid".into())
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
        with(&[("SANDPOST_IMAP_ENABLED", "false")])
            .unwrap()
            .imap
            .listen_address,
        None
    );
    let inherited = with(&[
        ("SANDPOST_SMTP_TLS_CERTIFICATE", "/etc/sandpost/mail.crt"),
        ("SANDPOST_SMTP_TLS_PRIVATE_KEY", "/etc/sandpost/mail.key"),
        ("SANDPOST_IMAP_BIND_ADDRESS", "0.0.0.0"),
        ("SANDPOST_IMAP_PORT", "993"),
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
        ("SANDPOST_IMAP_BIND_ADDRESS", "0.0.0.0"),
        ("SANDPOST_IMAP_TLS", "off"),
    ])
    .unwrap()
    .imap;
    assert_eq!(opted_out.encryption, ImapEncryptionSetting::Disabled);
    for invalid in [
        &[("SANDPOST_IMAP_BIND_ADDRESS", "0.0.0.0")][..],
        &[("SANDPOST_IMAP_BIND_ADDRESS", "nonsense")][..],
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

/// Load real TOML with supplied environment fields without mutating the process environment.
fn listener_sources(
    contents: &str,
    values: &[(&str, &str)],
) -> Result<Configuration, ConfigurationError> {
    let directory = ConfigurationDirectory::new();
    let path = directory.configuration_path();
    std::fs::write(&path, contents).unwrap();
    Configuration::from_sources(
        |name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).into())
        },
        Some(shared_file(path)),
    )
}

/// Resolve complete listener sections, leaving unrelated security and storage defaults intact.
#[test]
fn listener_toml_sections() {
    let configuration = listener_sources(
        r#"
[http]
bind_address = "::1"
port = 8080
[smtp]
bind_address = "127.0.0.2"
port = 2525
[imap]
enabled = true
bind_address = "::1"
port = 2143
"#,
        &[],
    )
    .unwrap();
    assert_eq!(
        configuration.web_listen_address,
        "[::1]:8080".parse().unwrap()
    );
    assert_eq!(
        configuration.mail_listen_address,
        "127.0.0.2:2525".parse().unwrap()
    );
    assert_eq!(
        configuration.imap.listen_address,
        Some("[::1]:2143".parse().unwrap())
    );
    assert_eq!(configuration.data_directory, default_data_directory());
    assert_eq!(
        configuration.mail_authentication,
        MailAuthenticationSetting::Automatic
    );
    assert_eq!(
        configuration.mail_encryption,
        MailEncryptionSetting::Automatic
    );
    assert_eq!(
        configuration.imap.encryption,
        ImapEncryptionSetting::Automatic
    );
    assert_eq!(configuration.imap.maximum_connection_count, 64);
}

/// Each environment address or port overrides only its corresponding TOML field.
#[test]
fn listener_fields_override_independently() {
    let contents = r#"
[http]
bind_address = "127.0.0.2"
port = 8080
[smtp]
bind_address = "127.0.0.2"
port = 2525
[imap]
bind_address = "127.0.0.2"
port = 2143
"#;
    for (variable, value, expected_addresses) in [
        (
            "SANDPOST_HTTP_BIND_ADDRESS",
            "::1",
            ["[::1]:8080", "127.0.0.2:2525", "127.0.0.2:2143"],
        ),
        (
            "SANDPOST_HTTP_PORT",
            "8081",
            ["127.0.0.2:8081", "127.0.0.2:2525", "127.0.0.2:2143"],
        ),
        (
            "SANDPOST_SMTP_BIND_ADDRESS",
            "::1",
            ["127.0.0.2:8080", "[::1]:2525", "127.0.0.2:2143"],
        ),
        (
            "SANDPOST_SMTP_PORT",
            "2526",
            ["127.0.0.2:8080", "127.0.0.2:2526", "127.0.0.2:2143"],
        ),
        (
            "SANDPOST_IMAP_BIND_ADDRESS",
            "::1",
            ["127.0.0.2:8080", "127.0.0.2:2525", "[::1]:2143"],
        ),
        (
            "SANDPOST_IMAP_PORT",
            "2144",
            ["127.0.0.2:8080", "127.0.0.2:2525", "127.0.0.2:2144"],
        ),
    ] {
        let configuration = listener_sources(contents, &[(variable, value)]).unwrap();
        assert_eq!(
            [
                configuration.web_listen_address,
                configuration.mail_listen_address,
                configuration.imap.listen_address.unwrap(),
            ],
            expected_addresses.map(|address| address.parse::<SocketAddr>().unwrap()),
            "{variable}"
        );
    }
}

/// Every listener supports environment-only IPv6 addresses and independent TCP ports.
#[test]
fn listener_environment_and_partial_defaults() {
    let configuration = Configuration::from_values(|name| match name {
        "SANDPOST_HTTP_BIND_ADDRESS"
        | "SANDPOST_SMTP_BIND_ADDRESS"
        | "SANDPOST_IMAP_BIND_ADDRESS" => Some("::1".into()),
        "SANDPOST_HTTP_PORT" => Some("8080".into()),
        "SANDPOST_SMTP_PORT" => Some("2525".into()),
        "SANDPOST_IMAP_PORT" => Some("2143".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        configuration.web_listen_address,
        "[::1]:8080".parse().unwrap()
    );
    assert_eq!(
        configuration.mail_listen_address,
        "[::1]:2525".parse().unwrap()
    );
    assert_eq!(
        configuration.imap.listen_address,
        Some("[::1]:2143".parse().unwrap())
    );

    for (contents, values, expected) in [
        ("[smtp]", vec![], Some("127.0.0.1:1025")),
        ("[smtp]\nport = 2525", vec![], Some("127.0.0.1:2525")),
        ("[smtp]\nbind_address = '::1'", vec![], Some("[::1]:1025")),
        (
            "",
            vec![("SANDPOST_SMTP_PORT", "2525")],
            Some("127.0.0.1:2525"),
        ),
        (
            "",
            vec![("SANDPOST_SMTP_BIND_ADDRESS", "::1")],
            Some("[::1]:1025"),
        ),
    ] {
        let configuration = listener_sources(contents, &values).unwrap();
        assert_eq!(
            configuration.mail_listen_address,
            expected.unwrap().parse().unwrap()
        );
    }
}

/// IMAP defaults to enabled; either source may disable it, and environment may re-enable it.
#[test]
fn imap_enable_precedence() {
    for (contents, values, expected) in [
        ("", vec![], true),
        ("[imap]\nenabled = false", vec![], false),
        ("", vec![("SANDPOST_IMAP_ENABLED", "false")], false),
        (
            "[imap]\nenabled = true",
            vec![("SANDPOST_IMAP_ENABLED", "false")],
            false,
        ),
        (
            "[imap]\nenabled = false\nbind_address = '::1'\nport = 2143",
            vec![("SANDPOST_IMAP_ENABLED", "true")],
            true,
        ),
    ] {
        let configuration = listener_sources(contents, &values).unwrap();
        assert_eq!(configuration.imap.listen_address.is_some(), expected);
        if contents.contains("2143") {
            assert_eq!(
                configuration.imap.listen_address,
                Some("[::1]:2143".parse().unwrap())
            );
        }
    }
    // Disabled network IMAP requires no plaintext opt-in because no connections can be accepted.
    assert!(
        listener_sources("[imap]\nenabled = false\nbind_address = '0.0.0.0'", &[])
            .unwrap()
            .imap
            .listen_address
            .is_none()
    );
}

/// Reject invalid typed listener values and unknown keys with file/field context.
#[test]
fn invalid_listener_files() {
    for section in ["http", "smtp", "imap"] {
        for field in [
            "bind_address = 'hostname'",
            "bind_address = '[::1]'",
            "bind_address = 123",
            "port = -1",
            "port = 65536",
            "port = '8025'",
            "port = 1.5",
            "unknown = true",
        ] {
            let error = listener_sources(&format!("[{section}]\n{field}"), &[]).unwrap_err();
            assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
            assert!(
                error.to_string().contains(field.split(' ').next().unwrap()),
                "{error}"
            );
        }
    }
    for value in ["'false'", "0", "'off'"] {
        assert!(matches!(
            listener_sources(&format!("[imap]\nenabled = {value}"), &[]),
            Err(ConfigurationError::File { .. })
        ));
    }
}

/// Invalid environment fields report their names even while IMAP is disabled.
#[test]
fn invalid_listener_environment_values() {
    for variable in [
        "SANDPOST_HTTP_BIND_ADDRESS",
        "SANDPOST_SMTP_BIND_ADDRESS",
        "SANDPOST_IMAP_BIND_ADDRESS",
    ] {
        for value in ["localhost", "127.0.0.1:80", "[::1]", "300.1.1.1", ""] {
            let error =
                listener_sources("[imap]\nenabled = false", &[(variable, value)]).unwrap_err();
            assert!(error.to_string().contains(variable), "{error}");
        }
    }
    for variable in [
        "SANDPOST_HTTP_PORT",
        "SANDPOST_SMTP_PORT",
        "SANDPOST_IMAP_PORT",
    ] {
        for value in ["-1", "65536", "1.5", "abc", ""] {
            let error =
                listener_sources("[imap]\nenabled = false", &[(variable, value)]).unwrap_err();
            assert!(error.to_string().contains(variable), "{error}");
        }
        for value in ["0", "65535"] {
            assert!(listener_sources("", &[(variable, value)]).is_ok());
        }
    }
    for value in ["off", "1", "yes", "FALSE", ""] {
        let error = listener_sources("", &[("SANDPOST_IMAP_ENABLED", value)]).unwrap_err();
        assert!(
            error.to_string().contains("SANDPOST_IMAP_ENABLED"),
            "{error}"
        );
        assert!(error.to_string().contains("true or false"), "{error}");
    }
}

/// Legacy variables fail even when empty, disabled, or superseded by valid new settings.
#[test]
fn legacy_listener_settings_require_migration() {
    for (variable, replacement) in [
        (
            "SANDPOST_HTTP_LISTEN",
            "SANDPOST_HTTP_BIND_ADDRESS and SANDPOST_HTTP_PORT",
        ),
        (
            "SANDPOST_SMTP_LISTEN",
            "SANDPOST_SMTP_BIND_ADDRESS and SANDPOST_SMTP_PORT",
        ),
        ("SANDPOST_IMAP_LISTEN", "SANDPOST_IMAP_ENABLED=false"),
    ] {
        for value in ["", "off", "127.0.0.1:0"] {
            let error = Configuration::from_values(|name| match name {
                name if name == variable => Some(value.into()),
                "SANDPOST_HTTP_PORT" => Some("8080".into()),
                _ => None,
            })
            .unwrap_err();
            assert!(error.to_string().contains(variable), "{error}");
            assert!(error.to_string().contains("no longer supported"), "{error}");
            assert!(error.to_string().contains(replacement), "{error}");
        }
    }
}

/// Detect exact, wildcard, and mapped conflicts without rejecting distinct interfaces or port zero.
#[test]
fn listener_address_conflicts() {
    for (first, second) in [
        ("127.0.0.1", "127.0.0.1"),
        ("0.0.0.0", "127.0.0.1"),
        ("127.0.0.1", "0.0.0.0"),
        ("::1", "::1"),
        ("::", "::1"),
        ("127.0.0.1", "::ffff:127.0.0.1"),
    ] {
        for (first_section, second_section) in
            [("http", "smtp"), ("http", "imap"), ("smtp", "imap")]
        {
            let contents = format!(
                "[{first_section}]\nbind_address = '{first}'\nport = 25000\n[{second_section}]\nbind_address = '{second}'\nport = 25000"
            );
            let error = listener_sources(&contents, &[("SANDPOST_IMAP_TLS", "off")]).unwrap_err();
            assert!(error.to_string().contains("conflicts"), "{error}");
        }
    }
    for (first, second, port) in [
        ("127.0.0.1", "127.0.0.2", 25000),
        ("::1", "::2", 25000),
        ("127.0.0.1", "::1", 25000),
        ("127.0.0.1", "127.0.0.1", 0),
    ] {
        let contents = format!(
            "[http]\nbind_address = '{first}'\nport = {port}\n[smtp]\nbind_address = '{second}'\nport = {port}"
        );
        listener_sources(&contents, &[]).unwrap();
    }
    listener_sources("[http]\nport = 1143\n[imap]\nenabled = false", &[]).unwrap();
}

/// Read each SMTP security policy from TOML while retaining the default listener address.
#[test]
fn mail_security_file_policies_and_defaults() {
    for (value, expected) in [
        ("auto", MailAuthenticationSetting::Automatic),
        ("required", MailAuthenticationSetting::Required),
        ("optional", MailAuthenticationSetting::Optional),
    ] {
        let configuration = listener_sources(&format!("[smtp]\nauth = '{value}'"), &[]).unwrap();
        assert_eq!(configuration.mail_authentication, expected);
        assert_eq!(
            configuration.mail_listen_address,
            "127.0.0.1:1025".parse().unwrap()
        );
    }
    for (value, expected) in [
        ("auto", MailEncryptionSetting::Automatic),
        ("required", MailEncryptionSetting::Required),
        ("off", MailEncryptionSetting::Disabled),
    ] {
        let configuration = listener_sources(
            &format!("[smtp]\ntls = '{value}'\ntls_private_key = 'mail.key'"),
            &[("SANDPOST_SMTP_TLS_CERTIFICATE", "mail.crt")],
        )
        .unwrap();
        assert_eq!(configuration.mail_encryption, expected);
        assert_eq!(
            configuration.mail_listen_address,
            "127.0.0.1:1025".parse().unwrap()
        );
    }
    for (value, expected) in [
        ("starttls", MailEncryptionMode::StartTls),
        ("implicit", MailEncryptionMode::Implicit),
    ] {
        let configuration = listener_sources(
            &format!("[smtp]\ntls_mode = '{value}'\ntls_private_key = 'mail.key'"),
            &[("SANDPOST_SMTP_TLS_CERTIFICATE", "mail.crt")],
        )
        .unwrap();
        assert_eq!(configuration.mail_encryption_mode, expected);
    }
    let configuration = listener_sources("[smtp]", &[]).unwrap();
    assert_eq!(
        configuration.mail_authentication,
        MailAuthenticationSetting::Automatic
    );
    assert_eq!(
        configuration.mail_encryption,
        MailEncryptionSetting::Automatic
    );
    assert_eq!(
        configuration.mail_encryption_mode,
        MailEncryptionMode::StartTls
    );
    assert!(configuration.mail_certificate.is_none());
}

/// Each environment policy overrides only its own TOML field; key overrides keep working-directory semantics.
#[test]
fn mail_security_environment_overrides_file_fields() {
    let contents = "[smtp]\nauth = 'required'\ntls = 'required'\ntls_mode = 'implicit'\ntls_private_key = 'file.key'";
    for (variable, value, expected_authentication, expected_encryption, expected_mode) in [
        (
            "SANDPOST_SMTP_AUTH",
            "optional",
            MailAuthenticationSetting::Optional,
            MailEncryptionSetting::Required,
            MailEncryptionMode::Implicit,
        ),
        (
            "SANDPOST_SMTP_TLS",
            "off",
            MailAuthenticationSetting::Required,
            MailEncryptionSetting::Disabled,
            MailEncryptionMode::Implicit,
        ),
        (
            "SANDPOST_SMTP_TLS_MODE",
            "starttls",
            MailAuthenticationSetting::Required,
            MailEncryptionSetting::Required,
            MailEncryptionMode::StartTls,
        ),
        (
            "SANDPOST_SMTP_TLS_PRIVATE_KEY",
            "override.key",
            MailAuthenticationSetting::Required,
            MailEncryptionSetting::Required,
            MailEncryptionMode::Implicit,
        ),
    ] {
        let configuration = listener_sources(
            contents,
            &[
                ("SANDPOST_SMTP_TLS_CERTIFICATE", "certificate.crt"),
                (variable, value),
            ],
        )
        .unwrap();
        assert_eq!(configuration.mail_authentication, expected_authentication);
        assert_eq!(configuration.mail_encryption, expected_encryption);
        assert_eq!(configuration.mail_encryption_mode, expected_mode);
        let paths = configuration.mail_certificate.unwrap();
        assert_eq!(paths.certificate_chain, PathBuf::from("certificate.crt"));
        if variable == "SANDPOST_SMTP_TLS_PRIVATE_KEY" {
            assert_eq!(paths.private_key, PathBuf::from("override.key"));
        } else {
            assert!(paths.private_key.is_absolute());
            assert!(paths.private_key.ends_with("file.key"));
        }
    }
}

/// Refuse misspelled policy values, invalid types and incomplete TLS settings rather than using defaults.
#[test]
fn invalid_mail_security_files_report_allowed_values() {
    for (field, value, expected) in [
        ("auth", "'requird'", "optional"),
        ("auth", "true", "auth"),
        ("tls", "'enabled'", "off"),
        ("tls", "42", "tls"),
        ("tls_mode", "'smtps'", "starttls"),
        ("tls_mode", "false", "tls_mode"),
        ("tls_private_key", "true", "tls_private_key"),
        ("tls_certificate", "true", "tls_certificate"),
    ] {
        let error = listener_sources(
            &format!("[smtp]\n{field} = {value}"),
            &[
                ("SANDPOST_SMTP_AUTH", "optional"),
                ("SANDPOST_SMTP_TLS", "off"),
            ],
        )
        .unwrap_err();
        assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
        assert!(error.to_string().contains(expected), "{error}");
    }
    for contents in [
        "[smtp]\ntls_private_key = 'mail.key'",
        "[smtp]\ntls = 'required'",
        "[smtp]\ntls_mode = 'implicit'",
    ] {
        assert!(listener_sources(contents, &[]).is_err(), "{contents}");
    }
    for (variable, value, field) in [
        ("SANDPOST_SMTP_AUTH", "requird", "auth = 'auto'"),
        ("SANDPOST_SMTP_TLS", "enabled", "tls = 'auto'"),
        ("SANDPOST_SMTP_TLS_MODE", "smtps", "tls_mode = 'starttls'"),
    ] {
        let error =
            listener_sources(&format!("[smtp]\n{field}"), &[(variable, value)]).unwrap_err();
        assert!(error.to_string().contains(variable), "{error}");
    }
}

/// File certificate/key paths resolve from the configuration directory and feed TLS loading and IMAP fallback.
#[test]
fn mail_security_file_key_loads_a_real_certificate() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate_path = directory.0.join("certificate.pem");
    let key_path = directory.0.join("private key.pem");
    std::fs::write(&certificate_path, certificate.cert.pem()).unwrap();
    std::fs::write(&key_path, certificate.signing_key.serialize_pem()).unwrap();
    let certificate_value = certificate_path.to_string_lossy().into_owned();
    for (file_certificate, private_key) in [
        (None, PathBuf::from("private key.pem")),
        (
            Some(PathBuf::from("certificate.pem")),
            PathBuf::from("private key.pem"),
        ),
        (Some(certificate_path.clone()), key_path.clone()),
    ] {
        let certificate_field = file_certificate
            .as_ref()
            .map(|path| format!("tls_certificate = '{}'\n", path.display()))
            .unwrap_or_default();
        // TOML literal strings also preserve Windows path separators.
        std::fs::write(
            &configuration_path,
            format!(
                "[smtp]\ntls = 'required'\n{certificate_field}tls_private_key = '{}'",
                private_key.display()
            ),
        )
        .unwrap();
        let configuration = Configuration::from_sources(
            |name| {
                (name == "SANDPOST_SMTP_TLS_CERTIFICATE" && file_certificate.is_none())
                    .then(|| certificate_value.clone())
            },
            Some(shared_file(configuration_path.clone())),
        )
        .unwrap();
        let paths = configuration.mail_certificate.as_ref().unwrap();
        assert_eq!(paths.certificate_chain, certificate_path);
        assert_eq!(paths.private_key, key_path);
        assert_eq!(configuration.imap.certificate.as_ref(), Some(paths));
        let encryption = crate::smtp_listener::MailEncryption::load(&configuration).unwrap();
        assert_eq!(
            encryption.describe("127.0.0.1:1025".parse().unwrap()),
            sandpost_application::MailListenerEncryption::StartTlsRequired
        );
        std::fs::remove_file(&key_path).unwrap();
        assert!(
            crate::smtp_listener::MailEncryption::load(&configuration)
                .err()
                .unwrap()
                .to_string()
                .contains("SMTP TLS certificate")
        );
        std::fs::write(&key_path, certificate.signing_key.serialize_pem()).unwrap();
    }
}

/// Resolve IMAP security, advertised host, and session limits from TOML before applying defaults.
#[test]
fn imap_file_security_policies_and_defaults() {
    let defaults = listener_sources("[imap]", &[]).unwrap().imap;
    assert_eq!(defaults.encryption, ImapEncryptionSetting::Automatic);
    assert_eq!(defaults.encryption_mode, ImapEncryptionMode::Implicit);
    assert!(defaults.certificate.is_none());
    assert!(defaults.advertised_host.is_none());
    assert_eq!(defaults.maximum_connection_count, 64);
    for (value, expected) in [
        ("auto", ImapEncryptionSetting::Automatic),
        ("required", ImapEncryptionSetting::Required),
        ("off", ImapEncryptionSetting::Disabled),
    ] {
        let configuration = listener_sources(&format!("[imap]\ntls = '{value}'\ntls_certificate = 'imap.crt'\ntls_private_key = 'imap.key'"), &[]).unwrap();
        assert_eq!(configuration.imap.encryption, expected);
    }
    for (value, expected) in [
        ("implicit", ImapEncryptionMode::Implicit),
        ("starttls", ImapEncryptionMode::StartTls),
    ] {
        let configuration =
            listener_sources(&format!("[imap]\ntls_mode = '{value}'"), &[]).unwrap();
        assert_eq!(configuration.imap.encryption_mode, expected);
    }
    let configuration = listener_sources(
        "[imap]\nhost = '  mail.example.test  '\nmax_connections = 8",
        &[],
    )
    .unwrap();
    assert_eq!(
        configuration.imap.advertised_host.as_deref(),
        Some("mail.example.test")
    );
    assert_eq!(configuration.imap.maximum_connection_count, 8);
    for count in [1, 10000] {
        assert_eq!(
            listener_sources(&format!("[imap]\nmax_connections = {count}"), &[])
                .unwrap()
                .imap
                .maximum_connection_count,
            count
        );
    }
    assert!(
        listener_sources("[imap]\nhost = '   '", &[])
            .unwrap()
            .imap
            .advertised_host
            .is_none()
    );
}

/// Each IMAP environment field overrides its TOML counterpart without replacing other file values.
#[test]
fn imap_environment_overrides_each_file_security_field() {
    let contents = "[imap]\ntls = 'required'\ntls_mode = 'starttls'\ntls_certificate = 'file.crt'\ntls_private_key = 'file.key'\nhost = 'file.example.test'\nmax_connections = 8";
    for (variable, value) in [
        ("SANDPOST_IMAP_TLS", "off"),
        ("SANDPOST_IMAP_TLS_MODE", "implicit"),
        ("SANDPOST_IMAP_TLS_CERTIFICATE", "environment.crt"),
        ("SANDPOST_IMAP_TLS_PRIVATE_KEY", "environment.key"),
        ("SANDPOST_IMAP_HOST", "environment.example.test"),
        ("SANDPOST_IMAP_MAX_CONNECTIONS", "16"),
    ] {
        let configuration = listener_sources(contents, &[(variable, value)])
            .unwrap()
            .imap;
        assert_eq!(
            configuration.encryption,
            if variable == "SANDPOST_IMAP_TLS" {
                ImapEncryptionSetting::Disabled
            } else {
                ImapEncryptionSetting::Required
            }
        );
        assert_eq!(
            configuration.encryption_mode,
            if variable == "SANDPOST_IMAP_TLS_MODE" {
                ImapEncryptionMode::Implicit
            } else {
                ImapEncryptionMode::StartTls
            }
        );
        assert_eq!(
            configuration.advertised_host.as_deref(),
            Some(if variable == "SANDPOST_IMAP_HOST" {
                "environment.example.test"
            } else {
                "file.example.test"
            })
        );
        assert_eq!(
            configuration.maximum_connection_count,
            if variable == "SANDPOST_IMAP_MAX_CONNECTIONS" {
                16
            } else {
                8
            }
        );
        let paths = configuration.certificate.unwrap();
        if variable == "SANDPOST_IMAP_TLS_CERTIFICATE" {
            assert_eq!(paths.certificate_chain, PathBuf::from("environment.crt"));
        } else {
            assert!(paths.certificate_chain.is_absolute());
            assert!(paths.certificate_chain.ends_with("file.crt"));
        }
        if variable == "SANDPOST_IMAP_TLS_PRIVATE_KEY" {
            assert_eq!(paths.private_key, PathBuf::from("environment.key"));
        } else {
            assert!(paths.private_key.is_absolute());
            assert!(paths.private_key.ends_with("file.key"));
        }
    }
    assert!(
        listener_sources(contents, &[("SANDPOST_IMAP_HOST", "")])
            .unwrap()
            .imap
            .advertised_host
            .is_none()
    );
}

/// Resolve complete dedicated pairs across sources, fall back to SMTP only when neither IMAP field exists.
#[test]
fn imap_file_certificate_pairing_and_mail_fallback() {
    let mail_environment = [
        ("SANDPOST_SMTP_TLS_CERTIFICATE", "smtp.crt"),
        ("SANDPOST_SMTP_TLS_PRIVATE_KEY", "smtp.key"),
    ];
    let configuration = listener_sources("[imap]\ntls = 'required'", &mail_environment).unwrap();
    assert_eq!(
        configuration.imap.certificate,
        configuration.mail_certificate
    );
    for (contents, variable, value) in [
        (
            "[imap]\ntls_certificate = 'file.crt'",
            "SANDPOST_IMAP_TLS_PRIVATE_KEY",
            "environment.key",
        ),
        (
            "[imap]\ntls_private_key = 'file.key'",
            "SANDPOST_IMAP_TLS_CERTIFICATE",
            "environment.crt",
        ),
    ] {
        assert!(
            listener_sources(contents, &[(variable, value)])
                .unwrap()
                .imap
                .certificate
                .is_some()
        );
        let error = listener_sources(contents, &mail_environment).unwrap_err();
        assert!(error.to_string().contains("together"), "{error}");
    }
    let dedicated = listener_sources(
        "[imap]\ntls_certificate = 'imap.crt'\ntls_private_key = 'imap.key'",
        &mail_environment,
    )
    .unwrap();
    assert_ne!(dedicated.imap.certificate, dedicated.mail_certificate);
    assert!(listener_sources("[imap]\ntls = 'required'", &[]).is_err());
    assert!(listener_sources("[imap]\nbind_address = '0.0.0.0'", &[]).is_err());
    assert!(listener_sources("[imap]\nbind_address = '0.0.0.0'\ntls = 'off'", &[]).is_ok());
}

/// File typos, invalid types, unsafe hosts and out-of-range limits fail without default fallback.
#[test]
fn invalid_imap_file_security_values() {
    for field in [
        "tls = 'enabled'",
        "tls = false",
        "tls_mode = 'smtps'",
        "tls_mode = 1",
        "tls_certificate = true",
        "tls_private_key = 42",
        "host = 42",
        "max_connections = -1",
        "max_connections = '64'",
        "max_connections = false",
    ] {
        let error = listener_sources(&format!("[imap]\n{field}"), &[]).unwrap_err();
        assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
    }
    for field in [
        "host = 'bad host'",
        "host = 'bad/host'",
        "max_connections = 0",
        "max_connections = 10001",
    ] {
        assert!(
            listener_sources(&format!("[imap]\n{field}"), &[]).is_err(),
            "{field}"
        );
    }
    for (variable, invalid_value, valid_field) in [
        ("SANDPOST_IMAP_TLS", "enabled", "tls = 'auto'"),
        ("SANDPOST_IMAP_TLS_MODE", "smtps", "tls_mode = 'implicit'"),
        (
            "SANDPOST_IMAP_HOST",
            "bad host",
            "host = 'mail.example.test'",
        ),
        ("SANDPOST_IMAP_MAX_CONNECTIONS", "0", "max_connections = 8"),
    ] {
        let error = listener_sources(
            &format!("[imap]\n{valid_field}"),
            &[(variable, invalid_value)],
        )
        .unwrap_err();
        assert!(error.to_string().contains(variable), "{error}");
    }
}

/// Real file certificates support relative and absolute paths and load through the existing TLS implementation.
#[test]
fn imap_file_certificate_paths_load_real_keys() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate_path = directory.0.join("imap cert.pem");
    let private_key_path = directory.0.join("imap key.pem");
    std::fs::write(&certificate_path, certificate.cert.pem()).unwrap();
    std::fs::write(&private_key_path, certificate.signing_key.serialize_pem()).unwrap();
    for (certificate_value, private_key_value) in [
        (
            PathBuf::from("imap cert.pem"),
            PathBuf::from("imap key.pem"),
        ),
        (certificate_path.clone(), private_key_path.clone()),
    ] {
        std::fs::write(
            &configuration_path,
            format!(
                "[imap]\ntls = 'required'\ntls_certificate = '{}'\ntls_private_key = '{}'",
                certificate_value.display(),
                private_key_value.display()
            ),
        )
        .unwrap();
        let configuration =
            Configuration::from_sources(|_| None, Some(shared_file(configuration_path.clone())))
                .unwrap();
        let paths = configuration.imap.certificate.unwrap();
        assert_eq!(paths.certificate_chain, certificate_path);
        assert_eq!(paths.private_key, private_key_path);
        assert!(
            sandpost_imap::TlsConfiguration::from_pem_files(
                &paths.certificate_chain,
                &paths.private_key
            )
            .is_ok()
        );
    }
}

/// Database paths resolve below environment overrides and against the independently selected data directory.
#[test]
fn storage_file_database_path_precedence() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    std::fs::write(
        &configuration_path,
        "[storage]\ndata_directory = 'mail data'\ndatabase_path = 'databases/capture.sqlite3'",
    )
    .unwrap();
    for (environment_directory, environment_database) in [
        (None, None),
        (Some("environment data"), None),
        (None, Some("override.sqlite3")),
        (Some("environment data"), Some("override.sqlite3")),
    ] {
        let configuration = Configuration::from_sources(
            |name| match name {
                "SANDPOST_DATA_DIRECTORY" => environment_directory.map(str::to_owned),
                "SANDPOST_DATABASE_PATH" => environment_database.map(str::to_owned),
                _ => None,
            },
            Some(shared_file(configuration_path.clone())),
        )
        .unwrap();
        let expected_directory = environment_directory
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.0.join("mail data"));
        assert_eq!(configuration.data_directory, expected_directory);
        assert_eq!(
            configuration.database_path,
            expected_directory.join(environment_database.unwrap_or("databases/capture.sqlite3"))
        );
        assert!(!configuration.database_path.exists());
    }
    // Omitting the data directory still anchors a relative database path to the platform default.
    let configuration =
        listener_sources("[storage]\ndatabase_path = 'custom.sqlite3'", &[]).unwrap();
    assert_eq!(
        configuration.database_path,
        default_data_directory().join("custom.sqlite3")
    );
}

/// Absolute TOML and environment database paths remain absolute regardless of the data directory.
#[test]
fn storage_absolute_database_paths_and_invalid_types() {
    let directory = ConfigurationDirectory::new();
    let configuration_path = directory.configuration_path();
    let absolute_database = directory.0.join("external database.sqlite3");
    let environment_database = directory.0.join("environment.sqlite3");
    std::fs::write(
        &configuration_path,
        format!(
            "[storage]\ndata_directory = 'mail'\ndatabase_path = '{}'",
            absolute_database.display()
        ),
    )
    .unwrap();
    for environment_directory in [None, Some("environment data")] {
        let configuration = Configuration::from_sources(
            |name| {
                (name == "SANDPOST_DATA_DIRECTORY")
                    .then(|| environment_directory.map(str::to_owned))
                    .flatten()
            },
            Some(shared_file(configuration_path.clone())),
        )
        .unwrap();
        assert_eq!(configuration.database_path, absolute_database);
        let configuration = Configuration::from_sources(
            |name| match name {
                "SANDPOST_DATA_DIRECTORY" => environment_directory.map(str::to_owned),
                "SANDPOST_DATABASE_PATH" => {
                    Some(environment_database.to_string_lossy().into_owned())
                }
                _ => None,
            },
            Some(shared_file(configuration_path.clone())),
        )
        .unwrap();
        assert_eq!(configuration.database_path, environment_database);
    }
    for value in ["42", "true", "[]", "{}"] {
        let error = listener_sources(
            &format!("[storage]\ndatabase_path = {value}"),
            &[("SANDPOST_DATABASE_PATH", "override.sqlite3")],
        )
        .unwrap_err();
        assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
        assert!(error.to_string().contains("database_path"), "{error}");
    }
}

/// Global TOML fields use their existing defaults and accept root-only limits and tracing directives.
#[test]
fn application_file_settings_and_defaults() {
    let defaults = listener_sources("[sandpost]", &[]).unwrap();
    assert_eq!(defaults.maximum_scope_depth, Some(2));
    assert_eq!(defaults.log_level, "info");
    for depth in [0, 4] {
        let configuration =
            listener_sources(&format!("[sandpost]\nmax_scope_depth = {depth}"), &[]).unwrap();
        assert_eq!(configuration.maximum_scope_depth, Some(depth));
        assert_eq!(configuration.log_level, "info");
    }
    for filter in [
        "off",
        "error",
        "warn",
        "info",
        "debug",
        "trace",
        "info,sandpost_application=debug",
    ] {
        let configuration =
            listener_sources(&format!("[sandpost]\nlog_level = '{filter}'"), &[]).unwrap();
        assert_eq!(configuration.log_level, filter);
        assert_eq!(configuration.maximum_scope_depth, Some(2));
        assert!(tracing_subscriber::EnvFilter::try_new(&configuration.log_level).is_ok());
    }
}

/// Environment overrides of global settings preserve the other field from TOML.
#[test]
fn application_environment_overrides_file_fields() {
    let contents = "[sandpost]\nmax_scope_depth = 4\nlog_level = 'debug'";
    for (values, expected_depth, expected_filter) in [
        (vec![], 4, "debug"),
        (vec![("SANDPOST_MAX_SCOPE_DEPTH", "0")], 0, "debug"),
        (vec![("SANDPOST_LOG_LEVEL", "warn")], 4, "warn"),
        (
            vec![
                ("SANDPOST_MAX_SCOPE_DEPTH", "8"),
                ("SANDPOST_LOG_LEVEL", "trace"),
            ],
            8,
            "trace",
        ),
    ] {
        let configuration = listener_sources(contents, &values).unwrap();
        assert_eq!(configuration.maximum_scope_depth, Some(expected_depth));
        assert_eq!(configuration.log_level, expected_filter);
    }
    for value in ["-1", "unlimited", ""] {
        let error = listener_sources(contents, &[("SANDPOST_MAX_SCOPE_DEPTH", value)]).unwrap_err();
        assert!(
            error.to_string().contains("SANDPOST_MAX_SCOPE_DEPTH"),
            "{error}"
        );
    }
}

/// Global file values reject unsupported types and misspelled keys without silent defaults.
#[test]
fn invalid_application_file_settings() {
    for field in [
        "max_scope_depth = -1",
        "max_scope_depth = '4'",
        "max_scope_depth = true",
        "log_level = 1",
        "log_level = false",
        "log_level = []",
        "log_levle = 'info'",
    ] {
        let error = listener_sources(&format!("[sandpost]\n{field}"), &[]).unwrap_err();
        assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
    }
    // Preserve the existing tracing parser's validation rather than limiting logs to level names.
    let configuration = listener_sources(
        "[sandpost]\nlog_level = 'sandpost_application=invalid'",
        &[],
    )
    .unwrap();
    assert!(tracing_subscriber::EnvFilter::try_new(&configuration.log_level).is_err());
}

/// Link checks refuse internal destinations unless TOML or the environment explicitly allows them.
#[test]
fn link_check_internal_addresses_require_an_explicit_opt_in() {
    assert!(
        !listener_sources("", &[])
            .unwrap()
            .link_check_allow_internal_addresses
    );
    let allowed = "[link_check]\nallow_internal_addresses = true";
    assert!(
        listener_sources(allowed, &[])
            .unwrap()
            .link_check_allow_internal_addresses
    );
    assert!(
        !listener_sources(
            allowed,
            &[("SANDPOST_LINK_CHECK_ALLOW_INTERNAL_ADDRESSES", "false")]
        )
        .unwrap()
        .link_check_allow_internal_addresses
    );
    assert!(
        listener_sources(
            "",
            &[("SANDPOST_LINK_CHECK_ALLOW_INTERNAL_ADDRESSES", "true")]
        )
        .unwrap()
        .link_check_allow_internal_addresses
    );
    let error = listener_sources(
        "",
        &[("SANDPOST_LINK_CHECK_ALLOW_INTERNAL_ADDRESSES", "yes")],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("SANDPOST_LINK_CHECK_ALLOW_INTERNAL_ADDRESSES"),
        "{error}"
    );
    for field in ["allow_internal_addresses = 'true'", "allow_internal = true"] {
        let error = listener_sources(&format!("[link_check]\n{field}"), &[]).unwrap_err();
        assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
    }
}

/// SMTP certificate overrides preserve the other TOML field and incomplete merged pairs fail startup.
#[test]
fn mail_file_certificate_environment_overrides() {
    let contents = "[smtp]\ntls_certificate = 'file.crt'\ntls_private_key = 'file.key'";
    for (variable, value) in [
        ("SANDPOST_SMTP_TLS_CERTIFICATE", "override.crt"),
        ("SANDPOST_SMTP_TLS_PRIVATE_KEY", "override.key"),
    ] {
        let configuration = listener_sources(contents, &[(variable, value)]).unwrap();
        let paths = configuration.mail_certificate.unwrap();
        if variable == "SANDPOST_SMTP_TLS_CERTIFICATE" {
            assert_eq!(paths.certificate_chain, PathBuf::from("override.crt"));
            assert!(paths.private_key.is_absolute());
            assert!(paths.private_key.ends_with("file.key"));
        } else {
            assert!(paths.certificate_chain.is_absolute());
            assert!(paths.certificate_chain.ends_with("file.crt"));
            assert_eq!(paths.private_key, PathBuf::from("override.key"));
        }
    }
    let configuration = listener_sources(
        "[smtp]\ntls_certificate = 'file.crt'",
        &[("SANDPOST_SMTP_TLS_PRIVATE_KEY", "environment.key")],
    )
    .unwrap();
    assert!(
        configuration
            .mail_certificate
            .unwrap()
            .certificate_chain
            .ends_with("file.crt")
    );
    let error = listener_sources("[smtp]\ntls_certificate = 'file.crt'", &[]).unwrap_err();
    assert!(error.to_string().contains("together"), "{error}");
}

/**
 * Persona limits default safely, follow TOML, and are overridden field by field by the
 * environment.
 */
#[test]
fn persona_settings_follow_toml_and_environment() {
    let defaults = listener_sources("", &[]).unwrap().personas;
    assert_eq!(defaults, crate::PersonaConfiguration::default());
    assert!(!defaults.allow_internal_addresses);
    let toml = "[personas]\nallow_internal_addresses = true\nmaximum_wait_seconds = 30\nexecution_timeout_seconds = 120\nconcurrent_executions = 4\nconcurrent_requests = 2";
    let configured = listener_sources(toml, &[]).unwrap().personas;
    assert_eq!(
        configured,
        crate::PersonaConfiguration {
            allow_internal_addresses: true,
            maximum_wait_seconds: 30,
            execution_timeout_seconds: 120,
            concurrent_executions: 4,
            concurrent_requests: 2,
        }
    );
    let overridden = listener_sources(
        toml,
        &[
            ("SANDPOST_PERSONAS_ALLOW_INTERNAL_ADDRESSES", "false"),
            ("SANDPOST_PERSONAS_CONCURRENT_REQUESTS", "16"),
        ],
    )
    .unwrap()
    .personas;
    assert!(!overridden.allow_internal_addresses);
    assert_eq!(overridden.concurrent_requests, 16);
    assert_eq!(overridden.maximum_wait_seconds, 30);
    let settings = configured.settings();
    assert_eq!(settings.maximum_wait, std::time::Duration::from_secs(30));
    assert_eq!(
        settings.execution_timeout,
        std::time::Duration::from_secs(120)
    );
    assert!(settings.allow_internal_addresses);
}

/**
 * Persona limits that could not work are refused before startup.
 */
#[test]
fn invalid_persona_settings_are_refused() {
    for (setting, value) in [
        ("SANDPOST_PERSONAS_MAXIMUM_WAIT_SECONDS", "0"),
        ("SANDPOST_PERSONAS_MAXIMUM_WAIT_SECONDS", "1351"),
        ("SANDPOST_PERSONAS_EXECUTION_TIMEOUT_SECONDS", "3"),
        ("SANDPOST_PERSONAS_CONCURRENT_EXECUTIONS", "0"),
        ("SANDPOST_PERSONAS_CONCURRENT_REQUESTS", "257"),
        ("SANDPOST_PERSONAS_ALLOW_INTERNAL_ADDRESSES", "yes"),
        ("SANDPOST_PERSONAS_CONCURRENT_EXECUTIONS", "many"),
    ] {
        let error = listener_sources("", &[(setting, value)]).unwrap_err();
        assert!(
            error.to_string().contains(setting),
            "{setting}={value}: {error}"
        );
    }
    for field in ["maximum_wait_seconds = '5'", "concurrency = 2"] {
        let error = listener_sources(&format!("[personas]\n{field}"), &[]).unwrap_err();
        assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
    }
}

/**
 * Outbound delivery defaults refuse internal destinations and keep the password key beside the
 * database; TOML and the environment override each field, and unusable values are refused.
 */
#[test]
fn outbound_settings_follow_toml_and_environment() {
    let defaults = listener_sources("", &[]).unwrap();
    let outbound = &defaults.outbound;
    assert!(!outbound.allow_internal_addresses);
    assert_eq!(outbound.concurrent_deliveries, 4);
    assert_eq!(outbound.maximum_attempts, 8);
    assert_eq!(outbound.retry_delay_seconds, 60);
    assert_eq!(
        outbound.secret_key_path,
        defaults.data_directory.join("outbound-secret.key")
    );
    let toml = "[outbound]\nallow_internal_addresses = true\nconcurrent_deliveries = 2\nmaximum_attempts = 3\nretry_delay_seconds = 10\nsecret_key_file = 'keys/outbound.key'";
    let configured = listener_sources(toml, &[]).unwrap();
    assert!(configured.outbound.allow_internal_addresses);
    assert_eq!(configured.outbound.maximum_attempts, 3);
    assert_eq!(
        configured.outbound.secret_key_path,
        configured.data_directory.join("keys/outbound.key")
    );
    let overridden = listener_sources(
        toml,
        &[
            ("SANDPOST_OUTBOUND_ALLOW_INTERNAL_ADDRESSES", "false"),
            ("SANDPOST_OUTBOUND_SECRET_KEY_FILE", "/secrets/outbound.key"),
        ],
    )
    .unwrap()
    .outbound;
    assert!(!overridden.allow_internal_addresses);
    assert_eq!(
        overridden.secret_key_path,
        std::path::PathBuf::from("/secrets/outbound.key")
    );
    let settings = configured.outbound.settings(None);
    assert_eq!(settings.retry_delay, std::time::Duration::from_secs(10));
    assert_eq!(settings.concurrent_deliveries, 2);
    assert!(settings.secret_key.is_none());
    for (setting, value) in [
        ("SANDPOST_OUTBOUND_CONCURRENT_DELIVERIES", "0"),
        ("SANDPOST_OUTBOUND_MAXIMUM_ATTEMPTS", "101"),
        ("SANDPOST_OUTBOUND_RETRY_DELAY_SECONDS", "0"),
        ("SANDPOST_OUTBOUND_ALLOW_INTERNAL_ADDRESSES", "yes"),
    ] {
        let error = listener_sources("", &[(setting, value)]).unwrap_err();
        assert!(
            error.to_string().contains(setting),
            "{setting}={value}: {error}"
        );
    }
    let error = listener_sources("[outbound]\nretry = 1", &[]).unwrap_err();
    assert!(matches!(error, ConfigurationError::File { .. }), "{error}");
}
