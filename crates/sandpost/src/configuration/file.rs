//! The TOML configuration file supports only an optional storage data directory today.

use super::{CONFIGURATION_PATH_SETTING, ConfigurationError};
use serde::Deserialize;
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// Parsed configuration file contents.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileConfiguration {
    storage: StorageConfiguration,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StorageConfiguration {
    data_directory: Option<String>,
}

impl FileConfiguration {
    /// Read the file without creating it; a missing shared file selects defaults, while a missing
    /// explicit file is refused so a mistyped path cannot silently fall back.
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
        toml::from_str(&contents)
            .map(Some)
            .map_err(|error| invalid(configuration_path, error.to_string()))
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
