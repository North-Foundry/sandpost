//! Bind, supervise, and drain the HTTP and SMTP transport tasks.

use crate::{configuration::Configuration, ingestion::capture_message, startup::StartupSummary};
use sandpost_match::Matcher;
use sandpost_storage::Storage;
use sandpost_web::ApplicationState;
use std::{error::Error, io::Write, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{broadcast, watch},
};

/// Start both listeners and stop their tasks on a termination signal or service failure.
pub(crate) async fn serve(
    application_configuration: &Configuration,
    storage: Storage,
    matcher: Arc<Matcher>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (event_broadcast, _) = broadcast::channel(256);
    let application_router = sandpost_web::router(ApplicationState {
        storage: storage.clone(),
        events: event_broadcast.clone(),
    });
    let web_listener = TcpListener::bind(application_configuration.web_listen_address).await?;
    let mail_listener = TcpListener::bind(application_configuration.mail_listen_address).await?;
    let startup_summary = StartupSummary {
        configuration: application_configuration,
        mail_listen_address: mail_listener.local_addr()?,
        web_listen_address: web_listener.local_addr()?,
    };
    tracing::debug!(web = %startup_summary.web_listen_address, mail = %startup_summary.mail_listen_address, "listeners initialized");
    // Storage bootstrap and both binds have succeeded; output errors still fail startup.
    {
        let mut startup_output = std::io::stdout().lock();
        write!(startup_output, "{startup_summary}")?;
        startup_output.flush()?;
    }
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let web_shutdown_receiver = shutdown_receiver.clone();
    let mut web_server_task = tokio::spawn(async move {
        axum::serve(web_listener, application_router)
            .with_graceful_shutdown(wait_for_shutdown(web_shutdown_receiver))
            .await
    });
    let mut mail_server_task = tokio::spawn(sandpost_mail::serve(
        mail_listener,
        move |message| {
            capture_message(
                message,
                Arc::clone(&matcher),
                storage.clone(),
                event_broadcast.clone(),
            )
        },
        wait_for_shutdown(shutdown_receiver),
    ));

    // A service failure shuts down its sibling. SIGINT and SIGTERM both drain.
    let result: Result<(), Box<dyn Error + Send + Sync>> = tokio::select! {
        signal = wait_for_termination_signal() => signal.map_err(Into::into),
        result = &mut web_server_task => match result { Ok(Ok(())) => Ok(()), Ok(Err(error)) => Err(error.into()), Err(error) => Err(error.into()) },
        result = &mut mail_server_task => match result { Ok(Ok(())) => Ok(()), Ok(Err(error)) => Err(error.into()), Err(error) => Err(error.into()) },
    };
    let _ = shutdown_sender.send(true);
    tracing::info!("shutting down listeners");
    // Server-sent event streams are long lived; bound draining, then abort remaining transport tasks.
    let drain = async {
        if !web_server_task.is_finished() {
            let _ = (&mut web_server_task).await;
        }
        if !mail_server_task.is_finished() {
            let _ = (&mut mail_server_task).await;
        }
    };
    if tokio::time::timeout(std::time::Duration::from_secs(6), drain)
        .await
        .is_err()
    {
        web_server_task.abort();
        mail_server_task.abort();
    }
    result
}

/// Resolve when shutdown is requested or the shutdown sender is dropped.
async fn wait_for_shutdown(mut shutdown_receiver: watch::Receiver<bool>) {
    if *shutdown_receiver.borrow() {
        return;
    }
    while shutdown_receiver.changed().await.is_ok() {
        if *shutdown_receiver.borrow() {
            return;
        }
    }
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
