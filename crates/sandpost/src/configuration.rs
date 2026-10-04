//! Parse typed application settings from environment values.

use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug)]
pub struct Configuration {
    pub web_listen_address: SocketAddr,
    pub mail_listen_address: SocketAddr,
    pub data_directory: PathBuf,
    pub database_path: PathBuf,
    pub maximum_scope_depth: Option<usize>,
    pub log_level: String,
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
        let data_directory =
            PathBuf::from(environment_value("SANDPOST_DATA_DIR").unwrap_or_else(|| "data".into()));
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
        Ok(Self {
            web_listen_address: parse_socket_address("SANDPOST_HTTP_LISTEN", "127.0.0.1:8025")?,
            mail_listen_address: parse_socket_address("SANDPOST_SMTP_LISTEN", "127.0.0.1:1025")?,
            data_directory,
            database_path,
            maximum_scope_depth,
            log_level: environment_value("SANDPOST_LOG_LEVEL").unwrap_or_else(|| "info".into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let configuration = Configuration::from_values(|setting_name| match setting_name {
            "SANDPOST_DATA_DIR" => Some("/tmp/test".into()),
            "SANDPOST_MAX_SCOPE_DEPTH" => Some("0".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(configuration.maximum_scope_depth, Some(0));
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
    }
}
