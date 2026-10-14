//! Render a plain-text startup summary from resolved settings and bound listeners.

use crate::configuration::Configuration;
use sandpost_application::{ImapSecurity, ImapServerDetails, MailListenerEncryption};
use std::{
    fmt::{self, Display, Formatter},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

/// Operator-facing state after storage initialization and successful listener binds.
pub struct StartupSummary<'configuration> {
    pub configuration: &'configuration Configuration,
    /// The global SMTP server's bound address, or `None` when its listener did not start.
    pub mail_listen_address: Option<SocketAddr>,
    /// Whether SMTP sessions must authenticate with an SMTP access.
    pub mail_authentication_required: bool,
    /// How SMTP sessions are protected in transit.
    pub mail_encryption: MailListenerEncryption,
    pub web_listen_address: SocketAddr,
    /// The IMAP service's connection details, or `None` when IMAP is disabled.
    pub imap: Option<&'configuration ImapServerDetails>,
    pub bootstrap_secret: Option<&'configuration str>,
}

impl Display for StartupSummary<'_> {
    /// Render only connection and storage information, independently of tracing filters.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let web_connection_address = connection_address(self.web_listen_address);
        // SocketAddr brackets IPv6; escape a scoped address's zone delimiter in URLs.
        let web_url = format!("http://{web_connection_address}").replace('%', "%25");

        writeln!(
            formatter,
            "\nSandpost\n  Local email capture for development"
        )?;
        let mut mail_rows = match self.mail_listen_address {
            Some(mail_listen_address) => {
                let mail_connection_address = connection_address(mail_listen_address);
                vec![
                    ("Host", connection_host(mail_connection_address)),
                    ("Port", mail_connection_address.port().to_string()),
                ]
            }
            None => vec![(
                "Status",
                "not listening; check SMTP Access in the admin interface".into(),
            )],
        };
        mail_rows.push((
            "Encryption",
            match self.mail_encryption {
                MailListenerEncryption::None => "none",
                MailListenerEncryption::StartTlsOptional => "STARTTLS (optional)",
                MailListenerEncryption::StartTlsRequired => "STARTTLS (required)",
                MailListenerEncryption::Implicit => "implicit TLS",
            }
            .into(),
        ));
        mail_rows.push((
            "Auth",
            if self.mail_authentication_required {
                "required (SMTP access credentials, PLAIN/LOGIN)".into()
            } else {
                "optional (unauthenticated local mail accepted)".into()
            },
        ));
        if let Some(mail_listen_address) = self.mail_listen_address {
            mail_rows.push(("Listening", mail_listen_address.to_string()));
        }
        write_section(formatter, "SMTP", &mail_rows)?;
        let imap_rows = match self.imap {
            Some(imap) => {
                let mut rows = Vec::new();
                if let Some(host) = &imap.host {
                    rows.push(("Host", host.clone()));
                }
                if let Some(port) = imap.port {
                    rows.push(("Port", port.to_string()));
                }
                rows.push((
                    "Security",
                    match imap.security {
                        ImapSecurity::None => "none (passwords in clear text)",
                        ImapSecurity::StartTlsOptional => "STARTTLS (optional)",
                        ImapSecurity::StartTlsRequired => "STARTTLS (required)",
                        ImapSecurity::Implicit => "implicit TLS (IMAPS)",
                    }
                    .into(),
                ));
                rows.push((
                    "Accounts",
                    "personal IMAP accounts are created in the web interface".into(),
                ));
                if let Some(address) = imap.listen_address {
                    rows.push(("Listening", address.to_string()));
                }
                rows
            }
            None => vec![("Status", "disabled (SANDPOST_IMAP_LISTEN=off)".into())],
        };
        write_section(formatter, "IMAP", &imap_rows)?;
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
                    "Data",
                    self.configuration.data_directory.display().to_string(),
                ),
                (
                    "Database",
                    format!("SQLite · {}", self.configuration.database_path.display()),
                ),
                ("Attachments", "in original messages (SQLite)".into()),
            ],
        )?;
        if let Some(secret) = self.bootstrap_secret {
            writeln!(
                formatter,
                "\nINSTANCE NOT INITIALIZED\n\nComplete the initial setup using the web interface."
            )?;
            writeln!(
                formatter,
                "Local setup URL: {web_url}/setup\n\nBootstrap secret:\n{secret}"
            )?;
        }
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
mod tests;
