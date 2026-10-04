//! Render a plain-text startup summary from resolved settings and bound listeners.

use crate::configuration::Configuration;
use std::{
    fmt::{self, Display, Formatter},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

/// Operator-facing state after storage initialization and successful listener binds.
pub(crate) struct StartupSummary<'configuration> {
    pub configuration: &'configuration Configuration,
    pub mail_listen_address: SocketAddr,
    pub web_listen_address: SocketAddr,
}

impl Display for StartupSummary<'_> {
    /// Render only connection and storage information, independently of tracing filters.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let mail_connection_address = connection_address(self.mail_listen_address);
        let web_connection_address = connection_address(self.web_listen_address);
        // SocketAddr brackets IPv6; escape a scoped address's zone delimiter in URLs.
        let web_url = format!("http://{web_connection_address}").replace('%', "%25");

        writeln!(
            formatter,
            "\nSandPost\n  Local email capture for development"
        )?;
        write_section(
            formatter,
            "SMTP",
            &[
                ("Host", connection_host(mail_connection_address)),
                ("Port", mail_connection_address.port().to_string()),
                // The implemented SMTP transport has neither TLS nor AUTH settings.
                ("Encryption", "none".into()),
                ("Auth", "disabled (no credentials required)".into()),
                ("Listening", self.mail_listen_address.to_string()),
            ],
        )?;
        write_section(
            formatter,
            "HTTP",
            &[
                // Link to an implemented resource; the API prefix itself has no route.
                ("API", format!("{web_url}/api/v1/messages")),
                ("Listening", self.web_listen_address.to_string()),
            ],
        )?;
        write_section(
            formatter,
            "Storage",
            &[
                (
                    "Database",
                    format!("SQLite · {}", self.configuration.database_path.display()),
                ),
                ("Attachments", "in original messages (SQLite)".into()),
            ],
        )?;
        writeln!(formatter, "\nReady to receive mail.\n")
    }
}

/// Replace wildcard binds with matching-family loopback addresses for local clients.
fn connection_address(listen_address: SocketAddr) -> SocketAddr {
    match listen_address.ip() {
        IpAddr::V4(address) if address.is_unspecified() => {
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), listen_address.port())
        }
        IpAddr::V6(address) if address.is_unspecified() => {
            SocketAddr::new(Ipv6Addr::LOCALHOST.into(), listen_address.port())
        }
        _ => listen_address,
    }
}

/// Format an SMTP hostname without URL brackets, retaining an IPv6 scope if present.
fn connection_host(connection_address: SocketAddr) -> String {
    match connection_address {
        SocketAddr::V6(address) if address.scope_id() != 0 => {
            format!("{}%{}", address.ip(), address.scope_id())
        }
        _ => connection_address.ip().to_string(),
    }
}

/// Write aligned section rows, escaping controls so configured paths remain plain text.
fn write_section(
    formatter: &mut Formatter<'_>,
    title: &str,
    rows: &[(&str, String)],
) -> fmt::Result {
    let label_width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    writeln!(formatter, "\n{title}")?;
    for (label, value) in rows {
        write!(formatter, "  {label:label_width$}  ")?;
        for character in value.chars() {
            if character.is_control() {
                write!(formatter, "{}", character.escape_default())?;
            } else {
                write!(formatter, "{character}")?;
            }
        }
        writeln!(formatter)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
        let configuration = Configuration::from_values(|_| None).unwrap();
        let output = StartupSummary {
            mail_listen_address: configuration.mail_listen_address,
            web_listen_address: configuration.web_listen_address,
            configuration: &configuration,
        }
        .to_string();
        assert!(output.contains("SandPost"));
        assert_eq!(row_value(&output, "Host"), "127.0.0.1");
        assert_eq!(row_value(&output, "Port"), "1025");
        assert_eq!(row_value(&output, "Encryption"), "none");
        assert_eq!(
            row_value(&output, "Auth"),
            "disabled (no credentials required)"
        );
        assert_eq!(
            row_value(&output, "API"),
            "http://127.0.0.1:8025/api/v1/messages"
        );
        assert_eq!(
            row_value(&output, "Database"),
            "SQLite · data/sandpost.sqlite3"
        );
        assert_eq!(
            row_value(&output, "Attachments"),
            "in original messages (SQLite)"
        );
        assert_eq!(output.matches("Ready to receive mail.").count(), 1);
        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains("IMAP"));
        assert!(!output.contains("Inbox"));
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
            web_listen_address: configuration.web_listen_address,
            configuration: &configuration,
        }
        .to_string();
        assert_eq!(row_value(&output, "Host"), "192.0.2.10");
        assert_eq!(row_value(&output, "Port"), "2525");
        assert_eq!(
            row_value(&output, "API"),
            "http://192.0.2.11:8080/api/v1/messages"
        );
        assert_eq!(
            row_value(&output, "Database"),
            "SQLite · /srv/custom/capture.sqlite3"
        );
        assert!(!output.contains("private-filter-value"));
        assert!(!output.contains("/srv/mail-data"));
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
            mail_listen_address: "127.0.0.1:34567".parse().unwrap(),
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
                mail_listen_address: listen_address.parse().unwrap(),
                web_listen_address: listen_address.parse().unwrap(),
            }
            .to_string();
            assert_eq!(row_value(&output, "Host"), expected_host);
            assert_eq!(row_value(&output, "API"), expected_url);
            assert!(output.contains(listen_address));
        }
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
            web_listen_address: configuration.web_listen_address,
            configuration: &configuration,
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
}
