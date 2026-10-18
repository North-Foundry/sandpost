//! Load shared file settings and environment overrides with platform data defaults.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

mod file;

/// Environment variable naming the configuration file to read.
const CONFIGURATION_PATH_SETTING: &str = "SANDPOST_CONFIG";
/// Shared home configuration file, used when the variable is absent.
const SHARED_CONFIGURATION_FILE: &str = ".sandpost/config.toml";

/// When SMTP sessions must authenticate with an SMTP access before submitting mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailAuthenticationSetting {
    /// Optional on loopback listeners (local development), required on every other address.
    #[serde(rename = "auto")]
    Automatic,
    /// Always required, including on loopback.
    Required,
    /// Never required; unauthenticated mail is accepted on any address. An explicit opt-in.
    Optional,
}

/// When SMTP sessions must be encrypted with TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailEncryptionSetting {
    /// With a certificate: required on non-loopback listeners, offered on loopback. Without a
    /// certificate: no TLS.
    #[serde(rename = "auto")]
    Automatic,
    /// Always required; a certificate must be configured.
    Required,
    /// Never offered, even if a certificate is configured.
    #[serde(rename = "off")]
    Disabled,
}

/// How TLS starts on the SMTP listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailEncryptionMode {
    /// Clients connect in clear text and upgrade with STARTTLS (ports 25 and 587).
    StartTls,
    /// Every connection starts with a TLS handshake (port 465).
    Implicit,
}

/// When IMAP sessions must be encrypted with TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImapEncryptionSetting {
    /// With a certificate: implicit TLS, or STARTTLS required on non-loopback addresses and
    /// offered on loopback. Without a certificate: clear text, allowed only on loopback.
    #[serde(rename = "auto")]
    Automatic,
    /// Always required; a certificate must be configured.
    Required,
    /// Never offered; passwords travel in clear text on any address. An explicit opt-in.
    #[serde(rename = "off")]
    Disabled,
}

/// How TLS starts on the IMAP listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImapEncryptionMode {
    /// Every connection starts with a TLS handshake (IMAPS, conventionally port 993).
    Implicit,
    /// Clients connect in clear text and upgrade with STARTTLS (conventionally port 143).
    StartTls,
}

/// Resolved IMAP service settings from TOML, environment overrides, and defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapConfiguration {
    /// Listener address; `None` when `imap.enabled` or `SANDPOST_IMAP_ENABLED` is false.
    pub listen_address: Option<SocketAddr>,
    pub encryption: ImapEncryptionSetting,
    pub encryption_mode: ImapEncryptionMode,
    /// Resolved IMAP TOML/environment certificate pair, else the resolved SMTP pair.
    pub certificate: Option<MailCertificatePaths>,
    /// Host name shown to users, from `imap.host` or `SANDPOST_IMAP_HOST`.
    pub advertised_host: Option<String>,
    /// Most concurrent IMAP sessions, from `imap.max_connections` or its environment override.
    pub maximum_connection_count: usize,
}

/// PEM files holding the SMTP server's certificate chain and private key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailCertificatePaths {
    pub certificate_chain: PathBuf,
    pub private_key: PathBuf,
}

/// Resolved runtime settings shared by the CLI and embedding hosts; validated before startup.
#[derive(Debug, Clone)]
pub struct Configuration {
    pub web_listen_address: SocketAddr,
    /// Optional frontend origin from the CLI override or `[frontend].proxy_url`.
    pub frontend_proxy_url: Option<String>,
    /// SMTP listener address from environment, TOML, or the 127.0.0.1:1025 default.
    pub mail_listen_address: SocketAddr,
    /// SMTP AUTH requirement, from `smtp.auth` or `SANDPOST_SMTP_AUTH`.
    pub mail_authentication: MailAuthenticationSetting,
    /// SMTP TLS requirement, from `smtp.tls` or `SANDPOST_SMTP_TLS`.
    pub mail_encryption: MailEncryptionSetting,
    /// How TLS starts, from `smtp.tls_mode` or `SANDPOST_SMTP_TLS_MODE`.
    pub mail_encryption_mode: MailEncryptionMode,
    /// Resolved `smtp.tls_certificate`/`smtp.tls_private_key` pair with independent environment overrides.
    pub mail_certificate: Option<MailCertificatePaths>,
    /// The single IMAP service shared by every user's IMAP accounts.
    pub imap: ImapConfiguration,
    pub data_directory: PathBuf,
    pub database_path: PathBuf,
    /// Maximum scope depth, counting roots as zero; defaults to two.
    pub maximum_scope_depth: Option<usize>,
    pub log_level: String,
    /// The configuration file that was read; absent while defaults and environment alone apply.
    pub configuration_path: Option<PathBuf>,
}

/// A configuration file or setting that cannot be used, named for the operator.
#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    #[error("invalid configuration file {}: {reason}", path.display())]
    File { path: PathBuf, reason: String },
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
        if let Some(url) = &self.frontend_proxy_url {
            let proxy = crate::frontend_proxy::FrontendProxy::from_url(url).map_err(|error| {
                invalid(
                    "frontend.proxy_url / --frontend-proxy-url",
                    &error.to_string(),
                )
            })?;
            proxy
                .validate_listener(self.web_listen_address)
                .map_err(|error| {
                    invalid(
                        "frontend.proxy_url / --frontend-proxy-url",
                        &error.to_string(),
                    )
                })?;
        }
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
                "SANDPOST_IMAP_BIND_ADDRESS",
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
        self.validate_listener_conflicts()
    }

    /// Reject definite conflicts among operator-configured listeners before touching storage.
    /// Port zero requests independent ephemeral ports. Cross-family wildcard overlap depends
    /// on the platform's dual-stack policy and remains subject to the actual bind result.
    fn validate_listener_conflicts(&self) -> Result<(), ConfigurationError> {
        let listeners = [
            ("HTTP", Some(self.web_listen_address)),
            ("SMTP", Some(self.mail_listen_address)),
            ("IMAP", self.imap.listen_address),
        ];
        for (index, (name, address)) in listeners.iter().enumerate() {
            let Some(address) = address else { continue };
            for (other_name, other_address) in &listeners[index + 1..] {
                let Some(other_address) = other_address else {
                    continue;
                };
                let canonical_address = address.ip().to_canonical();
                let other_canonical_address = other_address.ip().to_canonical();
                if address.port() != 0
                    && address.port() == other_address.port()
                    && canonical_address.is_ipv4() == other_canonical_address.is_ipv4()
                    && (canonical_address == other_canonical_address
                        || canonical_address.is_unspecified()
                        || other_canonical_address.is_unspecified())
                {
                    return Err(ConfigurationError::Invalid {
                        setting_name: "listener addresses",
                        reason: format!(
                            "{name} {address} conflicts with {other_name} {other_address}; use different ports or non-overlapping bind addresses"
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Read the configured file, then apply environment overrides and platform defaults.
    ///
    /// `SANDPOST_CONFIG` names an explicit file that must exist; without it the shared home file
    /// is read when present.
    pub fn from_environment() -> Result<Self, ConfigurationError> {
        Self::from_environment_with_frontend_proxy_url(None)
    }

    /// Resolve CLI frontend selection above the TOML value before validating the configuration.
    pub fn from_environment_with_frontend_proxy_url(
        frontend_proxy_url: Option<&str>,
    ) -> Result<Self, ConfigurationError> {
        reject_legacy_listeners(&|name| {
            std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
        })?;
        let environment_value = |setting_name: &str| std::env::var(setting_name).ok();
        let configuration_file = configuration_file(&environment_value)?;
        Self::from_sources_with_frontend_proxy_url(
            environment_value,
            configuration_file,
            frontend_proxy_url,
        )
    }

    /// Resolve the configuration file from a supplied lookup, which keeps tests off the process environment.
    #[cfg(test)]
    pub(crate) fn from_environment_values(
        environment_value: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigurationError> {
        let configuration_file = configuration_file(&environment_value)?;
        Self::from_sources(environment_value, configuration_file)
    }

    /// Merge an optional file's settings below environment values without changing the process.
    #[cfg(test)]
    fn from_sources(
        environment_value: impl Fn(&str) -> Option<String>,
        configuration_file: Option<ConfigurationFile>,
    ) -> Result<Self, ConfigurationError> {
        Self::from_sources_with_frontend_proxy_url(environment_value, configuration_file, None)
    }

    /// Merge the frontend CLI override before checking the effective file and environment settings.
    fn from_sources_with_frontend_proxy_url(
        environment_value: impl Fn(&str) -> Option<String>,
        configuration_file: Option<ConfigurationFile>,
        frontend_proxy_url: Option<&str>,
    ) -> Result<Self, ConfigurationError> {
        reject_legacy_listeners(&environment_value)?;
        let file = configuration_file
            .as_ref()
            .map(|configuration_file| {
                file::FileConfiguration::read(&configuration_file.path, configuration_file.required)
            })
            .transpose()?
            .flatten();
        let mut configuration = Self::from_settings(
            &environment_value,
            file.as_ref(),
            configuration_file.as_ref().map(|file| file.path.as_path()),
            frontend_proxy_url,
        )?;
        if file.is_some() {
            configuration.configuration_path = configuration_file.map(|file| file.path);
        }
        Ok(configuration)
    }
    /// Parse settings from a supplied lookup closure, using local defaults when absent.
    #[cfg(test)]
    pub(crate) fn from_values(
        environment_value: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigurationError> {
        Self::from_sources(environment_value, None)
    }

    /// Resolve individual typed file fields below environment values, then validate the result.
    fn from_settings(
        environment_value: &impl Fn(&str) -> Option<String>,
        file: Option<&file::FileConfiguration>,
        configuration_path: Option<&std::path::Path>,
        frontend_proxy_url: Option<&str>,
    ) -> Result<Self, ConfigurationError> {
        let defaults = file::FileConfiguration::default();
        let file = file.unwrap_or(&defaults);
        // Blank storage paths are treated as unset, matching blank file values.
        let path_value = |setting_name| {
            environment_value(setting_name)
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        };
        let data_directory = path_value("SANDPOST_DATA_DIRECTORY")
            .or_else(|| configuration_path.and_then(|path| file.data_directory(path)))
            .unwrap_or_else(default_data_directory);
        let database_path = data_directory.join(
            path_value("SANDPOST_DATABASE_PATH")
                .or_else(|| file.storage.database_path.clone())
                .unwrap_or_else(|| "sandpost.sqlite3".into()),
        );
        let maximum_scope_depth = environment_value("SANDPOST_MAX_SCOPE_DEPTH")
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|error| ConfigurationError::Invalid {
                        setting_name: "SANDPOST_MAX_SCOPE_DEPTH",
                        reason: error.to_string(),
                    })
            })
            .transpose()?
            .or(file.sandpost.max_scope_depth)
            .or(Some(2));
        for setting_name in ["SANDPOST_SMTP_ENDPOINT", "SANDPOST_SMTP_ENDPOINTS"] {
            if environment_value(setting_name).is_some() {
                return Err(ConfigurationError::Invalid {
                    setting_name,
                    reason: "SMTP endpoints were removed: all mail reaches one pool through the global SMTP server; remove this setting".into(),
                });
            }
        }
        let mail_authentication = environment_choice(
            environment_value,
            "SANDPOST_SMTP_AUTH",
            &[
                ("auto", MailAuthenticationSetting::Automatic),
                ("required", MailAuthenticationSetting::Required),
                ("optional", MailAuthenticationSetting::Optional),
            ],
            file.smtp
                .auth
                .unwrap_or(MailAuthenticationSetting::Automatic),
        )?;
        let mail_encryption = environment_choice(
            environment_value,
            "SANDPOST_SMTP_TLS",
            &[
                ("auto", MailEncryptionSetting::Automatic),
                ("required", MailEncryptionSetting::Required),
                ("off", MailEncryptionSetting::Disabled),
            ],
            file.smtp.tls.unwrap_or(MailEncryptionSetting::Automatic),
        )?;
        let mail_encryption_mode = environment_choice(
            environment_value,
            "SANDPOST_SMTP_TLS_MODE",
            &[
                ("starttls", MailEncryptionMode::StartTls),
                ("implicit", MailEncryptionMode::Implicit),
            ],
            file.smtp.tls_mode.unwrap_or(MailEncryptionMode::StartTls),
        )?;
        let mail_certificate = certificate_paths(
            environment_value,
            "smtp",
            [
                "SANDPOST_SMTP_TLS_CERTIFICATE",
                "SANDPOST_SMTP_TLS_PRIVATE_KEY",
            ],
            [&file.smtp.tls_certificate, &file.smtp.tls_private_key],
        )?;
        let imap = imap_configuration(environment_value, &file.imap, mail_certificate.as_ref())?;
        let web_listen_address = listener_address(
            environment_value,
            "SANDPOST_HTTP_BIND_ADDRESS",
            "SANDPOST_HTTP_PORT",
            file.http.bind_address,
            file.http.port,
            8025,
        )?;
        let mail_listen_address = listener_address(
            environment_value,
            "SANDPOST_SMTP_BIND_ADDRESS",
            "SANDPOST_SMTP_PORT",
            file.smtp.bind_address,
            file.smtp.port,
            1025,
        )?;
        let configuration = Self {
            imap,
            web_listen_address,
            frontend_proxy_url: frontend_proxy_url
                .map(str::to_owned)
                .or_else(|| file.frontend.proxy_url.clone()),
            mail_listen_address,
            mail_authentication,
            mail_encryption,
            mail_encryption_mode,
            mail_certificate,
            data_directory,
            database_path,
            maximum_scope_depth,
            log_level: environment_value("SANDPOST_LOG_LEVEL")
                .or_else(|| file.sandpost.log_level.clone())
                .unwrap_or_else(|| "info".into()),
            configuration_path: None,
        };
        configuration.validate()?;
        Ok(configuration)
    }
}

/// Refuse removed combined listener settings even when a replacement setting is also present.
fn reject_legacy_listeners(
    environment_value: &impl Fn(&str) -> Option<String>,
) -> Result<(), ConfigurationError> {
    for (setting_name, replacement) in [
        (
            "SANDPOST_HTTP_LISTEN",
            "SANDPOST_HTTP_BIND_ADDRESS and SANDPOST_HTTP_PORT",
        ),
        (
            "SANDPOST_SMTP_LISTEN",
            "SANDPOST_SMTP_BIND_ADDRESS and SANDPOST_SMTP_PORT",
        ),
        (
            "SANDPOST_IMAP_LISTEN",
            "SANDPOST_IMAP_BIND_ADDRESS and SANDPOST_IMAP_PORT; use SANDPOST_IMAP_ENABLED=false to disable IMAP",
        ),
    ] {
        if environment_value(setting_name).is_some() {
            return Err(ConfigurationError::Invalid {
                setting_name,
                reason: format!(
                    "{setting_name} is no longer supported. Use {replacement} instead."
                ),
            });
        }
    }
    Ok(())
}

/// Parse one environment field independently, retaining the resolved file/default value if absent.
fn environment_setting<Value: std::str::FromStr>(
    environment_value: &impl Fn(&str) -> Option<String>,
    setting_name: &'static str,
    fallback: Value,
    expected: &str,
) -> Result<Value, ConfigurationError> {
    match environment_value(setting_name) {
        Some(value) => value.parse().map_err(|_| ConfigurationError::Invalid {
            setting_name,
            reason: format!("expected {expected}, got {value:?}"),
        }),
        None => Ok(fallback),
    }
}

/// Parse a trimmed, case-insensitive enumerated environment setting, else keep the fallback.
fn environment_choice<Value: Copy>(
    environment_value: &impl Fn(&str) -> Option<String>,
    setting_name: &'static str,
    choices: &[(&str, Value)],
    fallback: Value,
) -> Result<Value, ConfigurationError> {
    let Some(value) = environment_value(setting_name) else {
        return Ok(fallback);
    };
    let value = value.trim().to_ascii_lowercase();
    if let Some((_, choice)) = choices.iter().find(|(name, _)| *name == value) {
        return Ok(*choice);
    }
    let names: Vec<&str> = choices.iter().map(|(name, _)| *name).collect();
    let expected = match names.as_slice() {
        [first, second] => format!("{first} or {second}"),
        [leading @ .., last] => format!("{}, or {last}", leading.join(", ")),
        [] => String::new(),
    };
    Err(ConfigurationError::Invalid {
        setting_name,
        reason: format!("expected {expected}, got {value:?}"),
    })
}

/// Resolve a certificate chain/private key pair whose environment fields override file fields
/// independently; `None` when neither is set, and an error when only one is.
fn certificate_paths(
    environment_value: &impl Fn(&str) -> Option<String>,
    file_section: &str,
    [certificate_setting, private_key_setting]: [&'static str; 2],
    [file_certificate, file_private_key]: [&Option<PathBuf>; 2],
) -> Result<Option<MailCertificatePaths>, ConfigurationError> {
    match (
        environment_value(certificate_setting)
            .map(PathBuf::from)
            .or_else(|| file_certificate.clone()),
        environment_value(private_key_setting)
            .map(PathBuf::from)
            .or_else(|| file_private_key.clone()),
    ) {
        (Some(certificate_chain), Some(private_key)) => Ok(Some(MailCertificatePaths {
            certificate_chain,
            private_key,
        })),
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(ConfigurationError::Invalid {
            setting_name: certificate_setting,
            reason: format!(
                "set {file_section}.tls_certificate and {file_section}.tls_private_key together (or their {certificate_setting} and {private_key_setting} overrides)"
            ),
        }),
    }
}

/// Compose an IPv4/IPv6 socket only after resolving the address and port independently.
fn listener_address(
    environment_value: &impl Fn(&str) -> Option<String>,
    address_setting: &'static str,
    port_setting: &'static str,
    file_address: Option<IpAddr>,
    file_port: Option<u16>,
    default_port: u16,
) -> Result<SocketAddr, ConfigurationError> {
    let address = environment_setting(
        environment_value,
        address_setting,
        file_address.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        "an IPv4 or IPv6 address",
    )?;
    let port = environment_setting(
        environment_value,
        port_setting,
        file_port.unwrap_or(default_port),
        "a TCP port from 0 to 65535 (0 requests an ephemeral port)",
    )?;
    Ok(SocketAddr::new(address, port))
}

/// The configuration file to read, and whether its absence stops startup.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigurationFile {
    path: PathBuf,
    /// An explicit `SANDPOST_CONFIG` path must exist; the shared home file may be absent.
    required: bool,
}

impl ConfigurationFile {
    /// An explicit file named by `SANDPOST_CONFIG`; a missing file stops startup.
    fn explicit(path: PathBuf) -> Self {
        Self {
            path,
            required: true,
        }
    }

    /// The shared home file, read only when it exists.
    fn shared(path: PathBuf) -> Self {
        Self {
            path,
            required: false,
        }
    }
}

/// Resolve the configuration file from `SANDPOST_CONFIG`, defaulting to the shared home file.
fn configuration_file(
    environment_value: &impl Fn(&str) -> Option<String>,
) -> Result<Option<ConfigurationFile>, ConfigurationError> {
    let Some(setting_value) = environment_value(CONFIGURATION_PATH_SETTING) else {
        return Ok(dirs::home_dir().map(|home_directory| {
            ConfigurationFile::shared(home_directory.join(SHARED_CONFIGURATION_FILE))
        }));
    };
    let path = PathBuf::from(setting_value.trim());
    if path.as_os_str().is_empty() {
        return Err(ConfigurationError::Invalid {
            setting_name: CONFIGURATION_PATH_SETTING,
            reason: "expected a file path".into(),
        });
    }
    Ok(Some(ConfigurationFile::explicit(path)))
}

/// Use native Application Support/RoamingAppData or XDG data storage, with a relative fallback.
fn default_data_directory() -> PathBuf {
    let application_directory = if cfg!(any(target_os = "macos", target_os = "windows")) {
        "Sandpost"
    } else {
        "sandpost"
    };
    dirs::data_dir()
        .map(|directory| directory.join(application_directory))
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// Default maximum of concurrent IMAP sessions.
const DEFAULT_IMAP_CONNECTIONS: usize = 64;

/// Resolve IMAP TOML and `SANDPOST_IMAP_*` settings, refusing clear-text password logins on network
/// addresses unless explicitly allowed.
fn imap_configuration(
    environment_value: &impl Fn(&str) -> Option<String>,
    file: &file::ImapConfiguration,
    mail_certificate: Option<&MailCertificatePaths>,
) -> Result<ImapConfiguration, ConfigurationError> {
    let enabled = environment_setting(
        environment_value,
        "SANDPOST_IMAP_ENABLED",
        file.enabled.unwrap_or(true),
        "true or false",
    )?;
    // Validate components even while disabled, but never bind a socket or change persisted state.
    let address = listener_address(
        environment_value,
        "SANDPOST_IMAP_BIND_ADDRESS",
        "SANDPOST_IMAP_PORT",
        file.bind_address,
        file.port,
        1143,
    )?;
    let listen_address = enabled.then_some(address);
    let encryption = environment_choice(
        environment_value,
        "SANDPOST_IMAP_TLS",
        &[
            ("auto", ImapEncryptionSetting::Automatic),
            ("required", ImapEncryptionSetting::Required),
            ("off", ImapEncryptionSetting::Disabled),
        ],
        file.tls.unwrap_or(ImapEncryptionSetting::Automatic),
    )?;
    let encryption_mode = environment_choice(
        environment_value,
        "SANDPOST_IMAP_TLS_MODE",
        &[
            ("implicit", ImapEncryptionMode::Implicit),
            ("starttls", ImapEncryptionMode::StartTls),
        ],
        file.tls_mode.unwrap_or(ImapEncryptionMode::Implicit),
    )?;
    let certificate = certificate_paths(
        environment_value,
        "imap",
        [
            "SANDPOST_IMAP_TLS_CERTIFICATE",
            "SANDPOST_IMAP_TLS_PRIVATE_KEY",
        ],
        [&file.tls_certificate, &file.tls_private_key],
    )?
    .or_else(|| mail_certificate.cloned());
    let maximum_connection_count = match environment_value("SANDPOST_IMAP_MAX_CONNECTIONS") {
        Some(value) => value
            .trim()
            .parse::<usize>()
            .map_err(|_| ConfigurationError::Invalid {
                setting_name: "SANDPOST_IMAP_MAX_CONNECTIONS",
                reason: "expected a number from 1 to 10000".into(),
            })?,
        None => file.max_connections.unwrap_or(DEFAULT_IMAP_CONNECTIONS),
    };
    let advertised_host = environment_value("SANDPOST_IMAP_HOST")
        .or_else(|| file.host.clone())
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
