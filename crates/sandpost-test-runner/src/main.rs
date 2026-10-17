mod http;
mod mail;
mod progress;
mod server;
mod smoke;

use std::path::PathBuf;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: sandpost-test-runner smoke-test [--binary PATH] [--frontend DIRECTORY]\n\nRun the real Sandpost binary through SMTP, HTTP/SSE, IMAP, search and restart checks.\nPrint each phase, its result and duration to stderr, including the active phase on failure.\nBuild the server first with cargo build -p sandpost. --frontend verifies embedded assets.";

/// Parse the smoke-test subcommand and report failures with an unsuccessful exit status.
#[tokio::main]
async fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return Ok(());
    }
    if arguments
        .first()
        .is_none_or(|argument| argument != "smoke-test")
    {
        return Err(USAGE.into());
    }
    let mut binary = None;
    let mut frontend = None;
    let mut remaining = arguments[1..].iter();
    while let Some(argument) = remaining.next() {
        match argument.to_str() {
            Some("--help" | "-h") => {
                println!("{USAGE}");
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
                return Err(
                    format!("unknown argument: {}\n{USAGE}", argument.to_string_lossy()).into(),
                );
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
