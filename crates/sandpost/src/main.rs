mod configuration;

use configuration::Configuration;
use sandpost_core::{Message, Scope, ScopeIdentifier, ScopeTree};
use sandpost_match::Matcher;
use sandpost_storage::Storage;
use sandpost_web::{ApplicationState, MessageEvent};
use std::{error::Error, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{broadcast, watch},
};
use tracing_subscriber::EnvFilter;

/// Configure and run the local web API and SMTP capture services.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let application_configuration = Configuration::from_environment()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&application_configuration.log_level)?)
        .try_init()?;
    std::fs::create_dir_all(&application_configuration.data_directory)?;
    if let Some(parent) = application_configuration
        .database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let storage = Storage::open(&application_configuration.database_path)?;
    let mut scopes = storage.load_scopes()?;
    if scopes.is_empty() {
        let root = Scope {
            // Stable bootstrap identifier across restarts and fresh installations.
            identifier: ScopeIdentifier(uuid::Uuid::from_u128(1)),
            parent: None,
            name: "All Mail".into(),
            description: Some("Local development capture root".into()),
            filter: String::new(),
            position: 0,
            policy_version: 1,
        };
        storage.save_scope(&root)?;
        scopes.push(root);
    }
    let tree = ScopeTree::new(scopes, application_configuration.maximum_scope_depth)?;
    let matcher = Arc::new(Matcher::new(&tree)?);
    let (event_broadcast, _) = broadcast::channel(256);
    let application_router = sandpost_web::router(ApplicationState {
        storage: storage.clone(),
        events: event_broadcast.clone(),
    });
    let web_listener = TcpListener::bind(application_configuration.web_listen_address).await?;
    let mail_listener = TcpListener::bind(application_configuration.mail_listen_address).await?;
    tracing::info!(web = %web_listener.local_addr()?, mail = %mail_listener.local_addr()?, database = %application_configuration.database_path.display(), "Sand Post ready (local development, no authentication)");
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let web_shutdown_receiver = shutdown_receiver.clone();
    let mut web_server_task = tokio::spawn(async move {
        axum::serve(web_listener, application_router)
            .with_graceful_shutdown(wait_for_shutdown(web_shutdown_receiver))
            .await
    });
    let mut mail_server_task = tokio::spawn(sandpost_mail::serve(
        mail_listener,
        move |message: Message| {
            let matcher = Arc::clone(&matcher);
            let storage = storage.clone();
            let event_broadcast = event_broadcast.clone();
            async move {
                let identifier = message.identifier;
                // Matching and all SQLite operations stay off the async runtime workers.
                let sequence = tokio::task::spawn_blocking(move || {
                    let result = matcher.match_message(&message.facts);
                    tracing::debug!(
                        candidates = result.statistics.candidates,
                        evaluations = result.statistics.predicate_evaluations,
                        matched = result.scopes.len(),
                        "message matched"
                    );
                    storage.insert_message(&message, &result.scopes)
                })
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
                let _ = event_broadcast.send(MessageEvent {
                    identifier,
                    sequence,
                });
                Ok::<(), String>(())
            }
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
