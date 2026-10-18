//! Select offline maintenance or a frontend proxy origin before startup.

use sandpost::{MaintenanceCommand, RuntimeError};
#[cfg(debug_assertions)]
use std::{path::PathBuf, time::Duration};

/// The selected maintenance command or frontend proxy origin for serving.
pub(super) struct CommandLine {
    pub maintenance: Option<MaintenanceCommand>,
    pub frontend_proxy_url: Option<String>,
    #[cfg(debug_assertions)]
    pub frontend_proxy_file: Option<PathBuf>,
}

/// Parse serving options independently of offline maintenance commands in every build profile.
pub(super) fn parse(arguments: &[String]) -> Result<CommandLine, RuntimeError> {
    #[cfg(debug_assertions)]
    let usage = "usage: sandpost [--frontend-proxy-url URL] [--frontend-proxy-file PATH]";
    #[cfg(not(debug_assertions))]
    let usage = "usage: sandpost [--frontend-proxy-url URL]";
    if arguments
        .first()
        .is_some_and(|argument| argument.starts_with("--frontend-proxy-"))
    {
        let mut frontend_proxy_url = None;
        #[cfg(debug_assertions)]
        let mut frontend_proxy_file = None;
        let mut index = 0;
        while index < arguments.len() {
            let flag = arguments[index].as_str();
            let Some(value) = arguments.get(index + 1) else {
                return Err(std::io::Error::other(usage).into());
            };
            if value.is_empty() || value.starts_with("--") {
                return Err(std::io::Error::other(usage).into());
            }
            match flag {
                "--frontend-proxy-url" if frontend_proxy_url.is_none() => {
                    frontend_proxy_url = Some(value.clone());
                }
                #[cfg(debug_assertions)]
                "--frontend-proxy-file" if frontend_proxy_file.is_none() => {
                    frontend_proxy_file = Some(PathBuf::from(value));
                }
                _ => return Err(std::io::Error::other(usage).into()),
            }
            index += 2;
        }
        return Ok(CommandLine {
            maintenance: None,
            frontend_proxy_url,
            #[cfg(debug_assertions)]
            frontend_proxy_file,
        });
    }
    Ok(CommandLine {
        maintenance: MaintenanceCommand::parse(arguments)?,
        frontend_proxy_url: None,
        #[cfg(debug_assertions)]
        frontend_proxy_file: None,
    })
}

#[cfg(debug_assertions)]
/// Read a Vite origin file, polling briefly while the frontend publishes it.
pub(super) async fn read_frontend_proxy_file(path: PathBuf) -> Result<String, RuntimeError> {
    tokio::time::timeout(Duration::from_secs(30), async move {
        loop {
            let read_path = path.clone();
            match tokio::task::spawn_blocking(move || std::fs::read_to_string(read_path)).await? {
                Ok(contents) => return Ok(contents.trim().to_owned()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    })
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "frontend proxy file was not published within 30 seconds",
        )
    })?
}

#[cfg(test)]
mod tests;
