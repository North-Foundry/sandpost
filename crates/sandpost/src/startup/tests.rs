use super::*;

/// Read an aligned row without depending on column spacing.
fn row_value<'output>(output: &'output str, label: &str) -> &'output str {
    output
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_prefix(label)
                .filter(|value| value.starts_with(char::is_whitespace))
                .map(str::trim)
        })
        .unwrap_or_else(|| panic!("missing row: {label}"))
}

/// Render configured defaults without duplicating their values in the renderer.
#[test]
fn default_configuration_reports_current_transport_and_storage_capabilities() {
    let configuration = Configuration::from_values(|setting_name| {
        (setting_name == "HOME").then(|| "/home/test".into())
    })
    .unwrap();
    let output = StartupSummary {
        mail_listen_address: Some("127.0.0.1:1025".parse().unwrap()),
        mail_authentication_required: false,
        mail_encryption: MailListenerEncryption::None,
        web_listen_address: configuration.web_listen_address,
        configuration: &configuration,
        imap: None,
        bootstrap_secret: None,
    }
    .to_string();
    assert!(output.contains("Sandpost"));
    assert_eq!(row_value(&output, "Host"), "127.0.0.1");
    assert_eq!(row_value(&output, "Port"), "1025");
    assert_eq!(row_value(&output, "Encryption"), "none");
    assert_eq!(
        row_value(&output, "Auth"),
        "optional (unauthenticated local mail accepted)"
    );
    assert_eq!(
        row_value(&output, "API"),
        "http://127.0.0.1:8025/api/v1/messages"
    );
    assert_eq!(
        row_value(&output, "Database"),
        "SQLite · /home/test/.sandpost/sandpost.sqlite3"
    );
    assert_eq!(row_value(&output, "Data"), "/home/test/.sandpost");
    assert_eq!(
        row_value(&output, "Attachments"),
        "in original messages (SQLite)"
    );
    assert_eq!(output.matches("Ready to receive mail.").count(), 1);
    assert!(!output.contains('\u{1b}'));
    assert_eq!(
        row_value(&output, "Status"),
        "disabled (SANDPOST_IMAP_LISTEN=off)"
    );
    assert!(!output.contains("Inbox"));
}

/// Report the IMAP service's host, port, security, and bound address.
#[test]
fn imap_section_reports_connection_details() {
    let configuration = Configuration::from_values(|_| None).unwrap();
    let imap = ImapServerDetails {
        enabled: true,
        host: Some("mail.example.test".into()),
        port: Some(993),
        security: ImapSecurity::Implicit,
        listen_address: Some("0.0.0.0:993".parse().unwrap()),
    };
    let output = StartupSummary {
        mail_listen_address: None,
        mail_authentication_required: true,
        mail_encryption: MailListenerEncryption::None,
        web_listen_address: configuration.web_listen_address,
        configuration: &configuration,
        imap: Some(&imap),
        bootstrap_secret: None,
    }
    .to_string();
    let imap_section = output.split("\nIMAP\n").nth(1).unwrap();
    assert!(imap_section.contains("mail.example.test"), "{output}");
    assert_eq!(row_value(imap_section, "Port"), "993");
    assert_eq!(row_value(imap_section, "Security"), "implicit TLS (IMAPS)");
    assert_eq!(row_value(imap_section, "Listening"), "0.0.0.0:993");
}

/// Preserve custom ports, database settings, and specific network interface addresses.
#[test]
fn environment_overrides_are_reflected_without_printing_unrelated_settings() {
    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_SMTP_LISTEN" => Some("192.0.2.10:2525".into()),
        "SANDPOST_HTTP_LISTEN" => Some("192.0.2.11:8080".into()),
        "SANDPOST_DATA_DIR" => Some("/srv/mail-data".into()),
        "SANDPOST_DATABASE_PATH" => Some("/srv/custom/capture.sqlite3".into()),
        "SANDPOST_LOG_LEVEL" => Some("private-filter-value".into()),
        _ => None,
    })
    .unwrap();
    let output = StartupSummary {
        mail_listen_address: configuration.mail_listen_address,
        mail_authentication_required: true,
        mail_encryption: MailListenerEncryption::None,
        web_listen_address: configuration.web_listen_address,
        configuration: &configuration,
        imap: None,
        bootstrap_secret: None,
    }
    .to_string();
    assert_eq!(row_value(&output, "Host"), "192.0.2.10");
    assert_eq!(row_value(&output, "Port"), "2525");
    assert_eq!(
        row_value(&output, "Auth"),
        "required (SMTP access credentials, PLAIN/LOGIN)"
    );
    assert_eq!(
        row_value(&output, "API"),
        "http://192.0.2.11:8080/api/v1/messages"
    );
    assert_eq!(
        row_value(&output, "Database"),
        "SQLite · /srv/custom/capture.sqlite3"
    );
    assert!(!output.contains("private-filter-value"));
    assert!(output.contains("/srv/mail-data"));
}

/// Use actual bound ports, including those assigned for configured port zero.
#[test]
fn bound_listener_addresses_take_precedence_over_requested_addresses() {
    let configuration = Configuration::from_values(|setting_name| match setting_name {
        "SANDPOST_SMTP_LISTEN" | "SANDPOST_HTTP_LISTEN" => Some("127.0.0.1:0".into()),
        _ => None,
    })
    .unwrap();
    let output = StartupSummary {
        configuration: &configuration,
        imap: None,
        bootstrap_secret: None,
        mail_listen_address: Some("127.0.0.1:34567".parse().unwrap()),
        mail_authentication_required: false,
        mail_encryption: MailListenerEncryption::None,
        web_listen_address: "127.0.0.1:45678".parse().unwrap(),
    }
    .to_string();
    assert_eq!(row_value(&output, "Port"), "34567");
    assert_eq!(
        row_value(&output, "API"),
        "http://127.0.0.1:45678/api/v1/messages"
    );
    assert!(output.contains("127.0.0.1:34567"));
    assert!(!output.contains("127.0.0.1:0"));
}

/// Distinguish wildcard listeners from local client addresses for IPv4 and IPv6.
#[test]
fn wildcard_and_specific_ipv6_addresses_have_usable_client_representations() {
    let configuration = Configuration::from_values(|_| None).unwrap();
    for (listen_address, expected_host, expected_url) in [
        (
            "0.0.0.0:2525",
            "127.0.0.1",
            "http://127.0.0.1:2525/api/v1/messages",
        ),
        ("[::]:2525", "::1", "http://[::1]:2525/api/v1/messages"),
        ("[::1]:2525", "::1", "http://[::1]:2525/api/v1/messages"),
        (
            "[2001:db8::2]:2525",
            "2001:db8::2",
            "http://[2001:db8::2]:2525/api/v1/messages",
        ),
        (
            "[fe80::2%3]:2525",
            "fe80::2%3",
            "http://[fe80::2%253]:2525/api/v1/messages",
        ),
    ] {
        let output = StartupSummary {
            configuration: &configuration,
            imap: None,
            bootstrap_secret: None,
            mail_listen_address: Some(listen_address.parse().unwrap()),
            mail_authentication_required: true,
            mail_encryption: MailListenerEncryption::None,
            web_listen_address: listen_address.parse().unwrap(),
        }
        .to_string();
        assert_eq!(row_value(&output, "Host"), expected_host);
        assert_eq!(row_value(&output, "API"), expected_url);
        assert!(output.contains(listen_address));
    }
}

/// Make the ephemeral startup credential and local setup URL easy to find in container logs.
#[test]
fn bootstrap_summary_prints_secret_once_and_labels_local_setup_url() {
    let configuration = Configuration::from_values(|name| match name {
        "SANDPOST_HTTP_LISTEN" => Some("0.0.0.0:8025".into()),
        "SANDPOST_DATA_DIR" => Some("/data".into()),
        _ => None,
    })
    .unwrap();
    let output = StartupSummary {
        configuration: &configuration,
        mail_listen_address: configuration.mail_listen_address,
        mail_authentication_required: true,
        mail_encryption: MailListenerEncryption::None,
        web_listen_address: configuration.web_listen_address,
        imap: None,
        bootstrap_secret: Some("SP-STARTUP-SECRET"),
    }
    .to_string();
    assert_eq!(output.matches("SP-STARTUP-SECRET").count(), 1);
    assert!(output.contains("INSTANCE NOT INITIALIZED"));
    assert!(output.contains("Local setup URL: http://127.0.0.1:8025/setup"));
    assert_eq!(row_value(&output, "Data"), "/data");
    assert!(output.contains("Listening  0.0.0.0:8025"));
}

/// Point the operator at the admin interface when the primary SMTP listener did not start.
#[test]
fn missing_primary_listener_is_reported_without_a_port() {
    let configuration = Configuration::from_values(|_| None).unwrap();
    let output = StartupSummary {
        configuration: &configuration,
        mail_listen_address: None,
        mail_authentication_required: false,
        mail_encryption: MailListenerEncryption::None,
        web_listen_address: configuration.web_listen_address,
        imap: None,
        bootstrap_secret: None,
    }
    .to_string();
    assert!(row_value(&output, "Status").starts_with("not listening"));
    assert!(!output.contains("Port"));
    assert_eq!(row_value(&output, "Encryption"), "none");
}

/// Prevent configured filenames from injecting terminal styling or false readiness lines.
#[test]
fn configured_path_controls_are_escaped_in_plain_text_output() {
    let configuration = Configuration::from_values(|setting_name| {
        (setting_name == "SANDPOST_DATABASE_PATH")
            .then(|| "data/\u{1b}[31m\nReady to receive mail.\t.sqlite3".into())
    })
    .unwrap();
    let output = StartupSummary {
        mail_listen_address: configuration.mail_listen_address,
        mail_authentication_required: true,
        mail_encryption: MailListenerEncryption::None,
        web_listen_address: configuration.web_listen_address,
        configuration: &configuration,
        imap: None,
        bootstrap_secret: None,
    }
    .to_string();
    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\t'));
    assert!(row_value(&output, "Database").contains("\\u{1b}[31m\\n"));
    assert_eq!(
        output
            .lines()
            .filter(|line| *line == "Ready to receive mail.")
            .count(),
        1
    );
}

/// Each transport protection is named in the Encryption row.
#[test]
fn encryption_row_names_the_transport_protection() {
    let configuration = Configuration::from_values(|_| None).unwrap();
    for (encryption, expected) in [
        (
            MailListenerEncryption::StartTlsOptional,
            "STARTTLS (optional)",
        ),
        (
            MailListenerEncryption::StartTlsRequired,
            "STARTTLS (required)",
        ),
        (MailListenerEncryption::Implicit, "implicit TLS"),
    ] {
        let output = StartupSummary {
            mail_listen_address: Some("0.0.0.0:587".parse().unwrap()),
            mail_authentication_required: true,
            mail_encryption: encryption,
            web_listen_address: configuration.web_listen_address,
            configuration: &configuration,
            imap: None,
            bootstrap_secret: None,
        }
        .to_string();
        assert_eq!(row_value(&output, "Encryption"), expected);
    }
}
