mod burst;
mod http;
mod mail;
mod progress;
mod server;
mod smoke;
mod smtp;

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: sandpost-test-runner <command>\n\nCommands:\n  smoke-test      Verify a real Sandpost binary with isolated data\n  mail-burst      Send synthetic SMTP traffic to an existing server\n\nUse <command> --help for options.";
const SMOKE_USAGE: &str = "Usage: sandpost-test-runner smoke-test [--binary PATH] [--frontend DIRECTORY]\n\nRun the real Sandpost binary through SMTP, HTTP/SSE, IMAP, search and restart checks.\nPrint each phase, its result and duration to stderr, including the active phase on failure.\nBuild the server first with cargo build -p sandpost. --frontend verifies embedded assets.";

/// Dispatch smoke checks or synthetic traffic, preserving unsuccessful and interrupted exit statuses.
#[tokio::main]
async fn main() -> Result<ExitCode> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    match arguments.first().and_then(|argument| argument.to_str()) {
        Some("--help" | "-h") => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        Some("smoke-test") => {
            run_smoke(&arguments[1..]).await?;
            Ok(ExitCode::SUCCESS)
        }
        Some("mail-burst") => burst::run(&arguments[1..]).await,
        _ => Err(USAGE.into()),
    }
}

/// Parse smoke-specific paths without imposing server-build requirements on traffic generation.
async fn run_smoke(arguments: &[OsString]) -> Result<()> {
    let mut binary = None;
    let mut frontend = None;
    let mut remaining = arguments.iter();
    while let Some(argument) = remaining.next() {
        match argument.to_str() {
            Some("--help" | "-h") => {
                println!("{SMOKE_USAGE}");
                return Ok(());
            }
            Some("--binary") => {
                binary = Some(PathBuf::from(
                    remaining.next().ok_or("--binary requires a path")?,
                ))
            }
            Some("--frontend") => {
                frontend = Some(PathBuf::from(
                    remaining.next().ok_or("--frontend requires a directory")?,
                ))
            }
            _ => {
                return Err(format!(
                    "unknown argument: {}\n{SMOKE_USAGE}",
                    argument.to_string_lossy()
                )
                .into());
            }
        }
    }
    // Resolve beside the runner so custom target directories and release builds work too.
    let binary = binary.unwrap_or(
        std::env::current_exe()?
            .with_file_name(format!("sandpost{}", std::env::consts::EXE_SUFFIX)),
    );
    if !binary.is_file() {
        return Err(format!(
            "server binary does not exist: {}; build first with cargo build -p sandpost",
            binary.display()
        )
        .into());
    }
    let binary = binary.canonicalize()?;
    let frontend = frontend
        .map(|directory| directory.canonicalize())
        .transpose()?;
    if frontend
        .as_ref()
        .is_some_and(|directory| !directory.join("index.html").is_file())
    {
        return Err("--frontend must contain a built index.html".into());
    }
    smoke::run(&binary, frontend.as_deref()).await
}
