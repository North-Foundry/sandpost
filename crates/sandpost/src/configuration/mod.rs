//! Parse typed application settings from environment values.

use std::{net::SocketAddr, path::PathBuf};

/// When SMTP sessions must authenticate with an SMTP access before submitting mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailAuthenticationSetting {
    /// Optional on loopback listeners (local development), required on every other address.
    Automatic,
    /// Always required, including on loopback.
    Required,
    /// Never required; unauthenticated mail is accepted on any address. An explicit opt-in.
    Optional,
}

/// When SMTP sessions must be encrypted with TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailEncryptionSetting {
    /// With a certificate: required on non-loopback listeners, offered on loopback. Without a
    /// certificate: no TLS.
    Automatic,
    /// Always required; a certificate must be configured.
    Required,
    /// Never offered, even if a certificate is configured.
    Disabled,
}

/// How TLS starts on the SMTP listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailEncryptionMode {
    /// Clients connect in clear text and upgrade with STARTTLS (ports 25 and 587).
    StartTls,
    /// Every connection starts with a TLS handshake (port 465).
    Implicit,
}

/// When IMAP sessions must be encrypted with TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImapEncryptionSetting {
    /// With a certificate: implicit TLS, or STARTTLS required on non-loopback addresses and
    /// offered on loopback. Without a certificate: clear text, allowed only on loopback.
    Automatic,
    /// Always required; a certificate must be configured.
    Required,
    /// Never offered; passwords travel in clear text on any address. An explicit opt-in.
    Disabled,
}

/// How TLS starts on the IMAP listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImapEncryptionMode {
    /// Every connection starts with a TLS handshake (IMAPS, conventionally port 993).
    Implicit,
    /// Clients connect in clear text and upgrade with STARTTLS (conventionally port 143).
    StartTls,
}

/// The IMAP service settings, from the `SANDPOST_IMAP_*` variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapConfiguration {
    /// Listener address; `None` when `SANDPOST_IMAP_LISTEN=off` disables IMAP.
    pub listen_address: Option<SocketAddr>,
    pub encryption: ImapEncryptionSetting,
    pub encryption_mode: ImapEncryptionMode,
    /// From `SANDPOST_IMAP_TLS_CERTIFICATE`/`_PRIVATE_KEY`, else the SMTP certificate pair.
    pub certificate: Option<MailCertificatePaths>,
    /// Host name shown to users in connection instructions, from `SANDPOST_IMAP_HOST`.
    pub advertised_host: Option<String>,
    /// Most concurrent IMAP sessions, from `SANDPOST_IMAP_MAX_CONNECTIONS`.
    pub maximum_connection_count: usize,
}

/// PEM files holding the SMTP server's certificate chain and private key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailCertificatePaths {
    pub certificate_chain: PathBuf,
    pub private_key: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Configuration {
    pub web_listen_address: SocketAddr,
    /// Environment override for the global SMTP server's persisted listener address.
    pub mail_listen_address: Option<SocketAddr>,
    /// SMTP AUTH requirement, from `SANDPOST_SMTP_AUTH` (`auto`, `required`, or `optional`).
    pub mail_authentication: MailAuthenticationSetting,
    /// SMTP TLS requirement, from `SANDPOST_SMTP_TLS` (`auto`, `required`, or `off`).
    pub mail_encryption: MailEncryptionSetting,
    /// How TLS starts, from `SANDPOST_SMTP_TLS_MODE` (`starttls` or `implicit`).
    pub mail_encryption_mode: MailEncryptionMode,
    /// From `SANDPOST_SMTP_TLS_CERTIFICATE` and `SANDPOST_SMTP_TLS_PRIVATE_KEY`, set together.
    pub mail_certificate: Option<MailCertificatePaths>,
    /// The single IMAP service shared by every user's IMAP accounts.
    pub imap: ImapConfiguration,
    pub data_directory: PathBuf,
    pub database_path: PathBuf,
    pub maximum_scope_depth: Option<usize>,
    pub log_level: String,
    pub frontend_directory: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    #[error("invalid {setting_name}: {reason}")]
    Invalid {
        setting_name: &'static str,
        reason: String,
    },
}
impl Configuration {
    /// Validate settings after parsing or editing their typed representation.
    pub(crate) fn validate(&self) -> Result<(), ConfigurationError> {
        let invalid = |setting_name, reason: &str| ConfigurationError::Invalid {
            setting_name,
            reason: reason.into(),
        };
        let smtp_needs_certificate = self.mail_encryption == MailEncryptionSetting::Required
            || (self.mail_encryption != MailEncryptionSetting::Disabled
                && self.mail_encryption_mode == MailEncryptionMode::Implicit);
        if smtp_needs_certificate && self.mail_certificate.is_none() {
            return Err(invalid(
                "SANDPOST_SMTP_TLS",
                "required or implicit TLS needs SANDPOST_SMTP_TLS_CERTIFICATE and SANDPOST_SMTP_TLS_PRIVATE_KEY",
            ));
        }
        if self.imap.encryption == ImapEncryptionSetting::Required
            && self.imap.certificate.is_none()
        {
            return Err(invalid(
                "SANDPOST_IMAP_TLS",
                "required TLS needs SANDPOST_IMAP_TLS_CERTIFICATE and SANDPOST_IMAP_TLS_PRIVATE_KEY (or the SMTP certificate)",
            ));
        }
        if let Some(address) = self.imap.listen_address
            && self.imap.encryption == ImapEncryptionSetting::Automatic
            && self.imap.certificate.is_none()
            && !address.ip().to_canonical().is_loopback()
        {
            return Err(invalid(
                "SANDPOST_IMAP_LISTEN",
                "IMAP on a non-loopback address needs a TLS certificate (SANDPOST_IMAP_TLS_CERTIFICATE and SANDPOST_IMAP_TLS_PRIVATE_KEY); set SANDPOST_IMAP_TLS=off to accept passwords in clear text",
            ));
        }
        if !(1..=10_000).contains(&self.imap.maximum_connection_count) {
            return Err(invalid(
                "SANDPOST_IMAP_MAX_CONNECTIONS",
                "expected a number from 1 to 10000",
            ));
        }
        if let Some(host) = &self.imap.advertised_host
            && (host.len() > 253
                || host.chars().any(|character| {
                    character.is_whitespace() || character.is_control() || character == '/'
                }))
        {
            return Err(invalid(
                "SANDPOST_IMAP_HOST",
                "expected a host name or address",
            ));
        }
        Ok(())
    }

    /// Build application configuration from the process environment.
    pub fn from_environment() -> Result<Self, ConfigurationError> {
        Self::from_values(|setting_name| std::env::var(setting_name).ok())
    }
    /// Parse settings from a supplied lookup closure, using local defaults when absent.
    pub(crate) fn from_values(
        environment_value: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigurationError> {
        let parse_socket_address = |setting_name, default_address: &str| {
            let setting_value =
                environment_value(setting_name).unwrap_or_else(|| default_address.into());
            setting_value
                .parse()
                .map_err(
                    |error: std::net::AddrParseError| ConfigurationError::Invalid {
                        setting_name,
                        reason: error.to_string(),
                    },
                )
        };
        let data_directory = environment_value("SANDPOST_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                environment_value("HOME")
                    .filter(|value| !value.is_empty())
                    .or_else(|| environment_value("USERPROFILE").filter(|value| !value.is_empty()))
                    .map(|home_directory| PathBuf::from(home_directory).join(".sandpost"))
                    .unwrap_or_else(|| PathBuf::from("data"))
            });
        let database_path = environment_value("SANDPOST_DATABASE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_directory.join("sandpost.sqlite3"));
        let maximum_scope_depth = environment_value("SANDPOST_MAX_SCOPE_DEPTH")
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|error| ConfigurationError::Invalid {
                        setting_name: "SANDPOST_MAX_SCOPE_DEPTH",
                        reason: error.to_string(),
                    })
            })
            .transpose()?;
        for setting_name in ["SANDPOST_SMTP_ENDPOINT", "SANDPOST_SMTP_ENDPOINTS"] {
            if environment_value(setting_name).is_some() {
                return Err(ConfigurationError::Invalid {
                    setting_name,
                    reason: "SMTP endpoints were removed: all mail reaches one pool through the global SMTP server; remove this setting".into(),
                });
            }
        }
        let mail_authentication = match environment_value("SANDPOST_SMTP_AUTH")
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("auto") => MailAuthenticationSetting::Automatic,
            Some("required") => MailAuthenticationSetting::Required,
            Some("optional") => MailAuthenticationSetting::Optional,
            Some(other) => {
                return Err(ConfigurationError::Invalid {
                    setting_name: "SANDPOST_SMTP_AUTH",
                    reason: format!("expected auto, required, or optional, got {other:?}"),
                });
            }
        };
        let normalized_value = |setting_name| {
            environment_value(setting_name).map(|value| value.trim().to_ascii_lowercase())
        };
        let mail_encryption = match normalized_value("SANDPOST_SMTP_TLS").as_deref() {
            None | Some("auto") => MailEncryptionSetting::Automatic,
            Some("required") => MailEncryptionSetting::Required,
            Some("off") => MailEncryptionSetting::Disabled,
            Some(other) => {
                return Err(ConfigurationError::Invalid {
                    setting_name: "SANDPOST_SMTP_TLS",
                    reason: format!("expected auto, required, or off, got {other:?}"),
                });
            }
        };
        let mail_encryption_mode = match normalized_value("SANDPOST_SMTP_TLS_MODE").as_deref() {
            None | Some("starttls") => MailEncryptionMode::StartTls,
            Some("implicit") => MailEncryptionMode::Implicit,
            Some(other) => {
                return Err(ConfigurationError::Invalid {
                    setting_name: "SANDPOST_SMTP_TLS_MODE",
                    reason: format!("expected starttls or implicit, got {other:?}"),
                });
            }
        };
        let mail_certificate = match (
            environment_value("SANDPOST_SMTP_TLS_CERTIFICATE"),
            environment_value("SANDPOST_SMTP_TLS_PRIVATE_KEY"),
        ) {
            (Some(certificate_chain), Some(private_key)) => Some(MailCertificatePaths {
                certificate_chain: certificate_chain.into(),
                private_key: private_key.into(),
            }),
            (None, None) => None,
            (Some(_), None) | (None, Some(_)) => {
                return Err(ConfigurationError::Invalid {
                    setting_name: "SANDPOST_SMTP_TLS_CERTIFICATE",
                    reason: "set SANDPOST_SMTP_TLS_CERTIFICATE and SANDPOST_SMTP_TLS_PRIVATE_KEY together".into(),
                });
            }
        };
        let imap = imap_configuration(&environment_value, mail_certificate.as_ref())?;
        let configuration = Self {
            imap,
            web_listen_address: parse_socket_address("SANDPOST_HTTP_LISTEN", "127.0.0.1:8025")?,
            mail_listen_address: environment_value("SANDPOST_SMTP_LISTEN")
                .map(|value| {
                    value.parse().map_err(|error: std::net::AddrParseError| {
                        ConfigurationError::Invalid {
                            setting_name: "SANDPOST_SMTP_LISTEN",
                            reason: error.to_string(),
                        }
                    })
                })
                .transpose()?,
            mail_authentication,
            mail_encryption,
            mail_encryption_mode,
            mail_certificate,
            data_directory,
            database_path,
            maximum_scope_depth,
            log_level: environment_value("SANDPOST_LOG_LEVEL").unwrap_or_else(|| "info".into()),
            frontend_directory: environment_value("SANDPOST_FRONTEND_DIR").map(PathBuf::from),
        };
        configuration.validate()?;
        Ok(configuration)
    }
}

/// Default IMAP listener: loopback on an unprivileged port, like the SMTP default.
const DEFAULT_IMAP_LISTEN: &str = "127.0.0.1:1143";
/// Default maximum of concurrent IMAP sessions.
const DEFAULT_IMAP_CONNECTIONS: usize = 64;

/// Parse the `SANDPOST_IMAP_*` settings, refusing password logins in clear text on network
/// addresses unless explicitly allowed.
fn imap_configuration(
    environment_value: &impl Fn(&str) -> Option<String>,
    mail_certificate: Option<&MailCertificatePaths>,
) -> Result<ImapConfiguration, ConfigurationError> {
    let invalid = |setting_name: &'static str, reason: String| ConfigurationError::Invalid {
        setting_name,
        reason,
    };
    let listen = environment_value("SANDPOST_IMAP_LISTEN")
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|| DEFAULT_IMAP_LISTEN.to_owned());
    let listen_address = if listen.eq_ignore_ascii_case("off") {
        None
    } else {
        Some(
            listen
                .parse::<SocketAddr>()
                .map_err(|error| invalid("SANDPOST_IMAP_LISTEN", error.to_string()))?,
        )
    };
    let normalized = |setting_name| {
        environment_value(setting_name).map(|value| value.trim().to_ascii_lowercase())
    };
    let encryption = match normalized("SANDPOST_IMAP_TLS").as_deref() {
        None | Some("auto") => ImapEncryptionSetting::Automatic,
        Some("required") => ImapEncryptionSetting::Required,
        Some("off") => ImapEncryptionSetting::Disabled,
        Some(other) => {
            return Err(invalid(
                "SANDPOST_IMAP_TLS",
                format!("expected auto, required, or off, got {other:?}"),
            ));
        }
    };
    let encryption_mode = match normalized("SANDPOST_IMAP_TLS_MODE").as_deref() {
        None | Some("implicit") => ImapEncryptionMode::Implicit,
        Some("starttls") => ImapEncryptionMode::StartTls,
        Some(other) => {
            return Err(invalid(
                "SANDPOST_IMAP_TLS_MODE",
                format!("expected implicit or starttls, got {other:?}"),
            ));
        }
    };
    let certificate = match (
        environment_value("SANDPOST_IMAP_TLS_CERTIFICATE"),
        environment_value("SANDPOST_IMAP_TLS_PRIVATE_KEY"),
    ) {
        (Some(certificate_chain), Some(private_key)) => Some(MailCertificatePaths {
            certificate_chain: certificate_chain.into(),
            private_key: private_key.into(),
        }),
        (None, None) => mail_certificate.cloned(),
        _ => {
            return Err(invalid(
                "SANDPOST_IMAP_TLS_CERTIFICATE",
                "set SANDPOST_IMAP_TLS_CERTIFICATE and SANDPOST_IMAP_TLS_PRIVATE_KEY together"
                    .into(),
            ));
        }
    };
    let maximum_connection_count = match environment_value("SANDPOST_IMAP_MAX_CONNECTIONS") {
        Some(value) => value.trim().parse::<usize>().ok().ok_or_else(|| {
            invalid(
                "SANDPOST_IMAP_MAX_CONNECTIONS",
                "expected a number from 1 to 10000".into(),
            )
        })?,
        None => DEFAULT_IMAP_CONNECTIONS,
    };
    let advertised_host = environment_value("SANDPOST_IMAP_HOST")
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty());
    Ok(ImapConfiguration {
        listen_address,
        encryption,
        encryption_mode,
        certificate,
        advertised_host,
        maximum_connection_count,
    })
}

#[cfg(test)]
mod tests;
