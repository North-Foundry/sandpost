//! Configure tracing and host the shared runtime until a process termination signal.

use sandpost::{Configuration, MaintenanceCommand, RuntimeError, SandpostRuntime, StartupSummary};
use std::io::{IsTerminal, Write};
use tracing_subscriber::EnvFilter;

/// Run a maintenance command, or start the catcher, report bound listeners, and drain on
/// signals or service failure.
#[tokio::main]
async fn main() -> Result<(), RuntimeError> {
    let configuration = Configuration::from_environment()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&configuration.log_level)?)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none())
        .try_init()?;
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(command) = MaintenanceCommand::parse(&arguments)? {
        return command.run(&configuration).await;
    }
    let mut running = SandpostRuntime::start(configuration.clone()).await?;
    let bootstrap_secret = running
        .application
        .instance()
        .bootstrap_secret_for_startup()?;
    let output_result = {
        let mut output = std::io::stdout().lock();
        write!(
            output,
            "{}",
            StartupSummary {
                configuration: &configuration,
                mail_listen_address: running.mail_listen_address,
                mail_authentication_required: running.mail_authentication_required,
                mail_encryption: running.mail_encryption,
                web_listen_address: running.web_listen_address,
                imap: running.imap.as_ref(),
                bootstrap_secret: bootstrap_secret.as_deref(),
            }
        )
        .and_then(|()| output.flush())
    };
    drop(bootstrap_secret);
    if let Err(error) = output_result {
        let _ = running.shutdown().await;
        return Err(error.into());
    }
    let result = tokio::select! {
        signal = wait_for_termination_signal() => signal.map_err(Into::into),
        result = running.wait() => result,
    };
    let shutdown_result = running.shutdown().await;
    result.and(shutdown_result)
}

/// Wait for the platform's standard interrupt or termination signal.
async fn wait_for_termination_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result, _ = terminate.recv() => Ok(()) }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
