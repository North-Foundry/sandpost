//! Strictly typed TOML listener, frontend, and storage settings before runtime overrides.

use super::{
    CONFIGURATION_PATH_SETTING, ConfigurationError, ImapEncryptionMode, ImapEncryptionSetting,
    MailAuthenticationSetting, MailEncryptionMode, MailEncryptionSetting,
};
use serde::Deserialize;
use std::{
    io::ErrorKind,
    net::IpAddr,
    path::{Path, PathBuf},
};

/// Parsed configuration file contents.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileConfiguration {
    pub sandpost: ApplicationConfiguration,
    pub storage: StorageConfiguration,
    pub http: ListenerConfiguration,
    pub smtp: MailConfiguration,
    pub imap: ImapConfiguration,
    pub frontend: FrontendConfiguration,
    pub link_check: LinkCheckConfiguration,
}

/// Outbound link verification policy; omission keeps internal destinations refused.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LinkCheckConfiguration {
    pub allow_internal_addresses: Option<bool>,
}

/// Optional external frontend origin; omission retains embedded frontend serving.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FrontendConfiguration {
    pub proxy_url: Option<String>,
}

/// Global application settings, independently overridden by environment fields.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ApplicationConfiguration {
    pub max_scope_depth: Option<usize>,
    pub log_level: Option<String>,
}

/// Independently optional listener components; omission preserves lower-priority defaults.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ListenerConfiguration {
    pub bind_address: Option<IpAddr>,
    pub port: Option<u16>,
}

/// SMTP listener components and security policy, independently overridden by environment fields.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct MailConfiguration {
    pub bind_address: Option<IpAddr>,
    pub port: Option<u16>,
    pub auth: Option<MailAuthenticationSetting>,
    pub tls: Option<MailEncryptionSetting>,
    pub tls_mode: Option<MailEncryptionMode>,
    pub tls_certificate: Option<PathBuf>,
    pub tls_private_key: Option<PathBuf>,
}

/// IMAP may be disabled without changing any persisted accounts or mailbox state.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ImapConfiguration {
    pub enabled: Option<bool>,
    pub bind_address: Option<IpAddr>,
    pub port: Option<u16>,
    pub tls: Option<ImapEncryptionSetting>,
    pub tls_mode: Option<ImapEncryptionMode>,
    pub tls_certificate: Option<PathBuf>,
    pub tls_private_key: Option<PathBuf>,
    pub host: Option<String>,
    pub max_connections: Option<usize>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StorageConfiguration {
    data_directory: Option<String>,
    pub database_path: Option<PathBuf>,
}

impl FileConfiguration {
    /// Read the file without creating it; a missing shared file selects defaults, while a missing
    /// explicit file is refused so a mistyped path cannot silently fall back.
    /// Relative certificate/key paths are anchored to the configuration file's directory.
    pub(super) fn read(
        configuration_path: &Path,
        required: bool,
    ) -> Result<Option<Self>, ConfigurationError> {
        let contents = match std::fs::read_to_string(configuration_path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound && !required => return Ok(None),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(invalid(
                    configuration_path,
                    format!(
                        "not found; {CONFIGURATION_PATH_SETTING} must name an existing configuration file"
                    ),
                ));
            }
            Err(error) => return Err(invalid(configuration_path, error.to_string())),
        };
        let mut configuration: Self = toml::from_str(&contents)
            .map_err(|error| invalid(configuration_path, error.to_string()))?;
        for certificate_path in [
            &mut configuration.smtp.tls_certificate,
            &mut configuration.smtp.tls_private_key,
            &mut configuration.imap.tls_certificate,
            &mut configuration.imap.tls_private_key,
        ]
        .into_iter()
        .flatten()
        {
            if certificate_path.is_relative() {
                *certificate_path = configuration_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&*certificate_path);
            }
        }
        Ok(Some(configuration))
    }

    /// The configured data directory; blank strings use the system default and relative paths are
    /// anchored to the configuration directory, independently of the launcher.
    pub(super) fn data_directory(&self, configuration_path: &Path) -> Option<PathBuf> {
        let directory = self.storage.data_directory.as_deref()?;
        if directory.trim().is_empty() {
            return None;
        }
        let directory = PathBuf::from(directory);
        if directory.is_absolute() {
            return Some(directory);
        }
        Some(
            configuration_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(directory),
        )
    }
}

/// Build a configuration-file error naming the file and the reason it is unusable.
fn invalid(configuration_path: &Path, reason: String) -> ConfigurationError {
    ConfigurationError::File {
        path: configuration_path.to_owned(),
        reason,
    }
}
