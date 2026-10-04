use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug)]
pub struct Config {
    pub http_listen: SocketAddr,
    pub smtp_listen: SocketAddr,
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub max_scope_depth: Option<usize>,
    pub log_level: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid {name}: {reason}")]
    Invalid { name: &'static str, reason: String },
}
impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(|name| std::env::var(name).ok())
    }
    fn from_values(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let parse_addr = |name, default: &str| {
            let value = get(name).unwrap_or_else(|| default.into());
            value
                .parse()
                .map_err(|error: std::net::AddrParseError| ConfigError::Invalid {
                    name,
                    reason: error.to_string(),
                })
        };
        let data_dir = PathBuf::from(get("SANDPOST_DATA_DIR").unwrap_or_else(|| "data".into()));
        let database_path = get("SANDPOST_DATABASE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("sandpost.sqlite3"));
        let max_scope_depth = get("SANDPOST_MAX_SCOPE_DEPTH")
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|error| ConfigError::Invalid {
                        name: "SANDPOST_MAX_SCOPE_DEPTH",
                        reason: error.to_string(),
                    })
            })
            .transpose()?;
        Ok(Self {
            http_listen: parse_addr("SANDPOST_HTTP_LISTEN", "127.0.0.1:8025")?,
            smtp_listen: parse_addr("SANDPOST_SMTP_LISTEN", "127.0.0.1:1025")?,
            data_dir,
            database_path,
            max_scope_depth,
            log_level: get("SANDPOST_LOG_LEVEL").unwrap_or_else(|| "info".into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_overrides_and_invalid_values() {
        let config = Config::from_values(|_| None).unwrap();
        assert_eq!(config.http_listen.to_string(), "127.0.0.1:8025");
        assert_eq!(config.database_path, PathBuf::from("data/sandpost.sqlite3"));
        assert!(config.max_scope_depth.is_none());
        let config = Config::from_values(|name| match name {
            "SANDPOST_DATA_DIR" => Some("/tmp/test".into()),
            "SANDPOST_MAX_SCOPE_DEPTH" => Some("0".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.max_scope_depth, Some(0));
        assert_eq!(
            config.database_path,
            PathBuf::from("/tmp/test/sandpost.sqlite3")
        );
        assert!(
            Config::from_values(|name| (name == "SANDPOST_HTTP_LISTEN").then(|| "invalid".into()))
                .is_err()
        );
    }
}
