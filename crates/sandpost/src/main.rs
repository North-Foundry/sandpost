mod config;

use config::Config;
use sandpost_api::{ApiState, MessageEvent};
use sandpost_core::{Message, Scope, ScopeId, ScopeTree};
use sandpost_match::Matcher;
use sandpost_storage::Storage;
use std::{error::Error, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{broadcast, watch},
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::from_env()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&config.log_level)?)
        .try_init()?;
    std::fs::create_dir_all(&config.data_dir)?;
    if let Some(parent) = config
        .database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let storage = Storage::open(&config.database_path)?;
    let mut scopes = storage.load_scopes()?;
    if scopes.is_empty() {
        let root = Scope {
            // Stable bootstrap identifier across restarts and fresh installations.
            id: ScopeId(uuid::Uuid::from_u128(1)),
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
    let tree = ScopeTree::new(scopes, config.max_scope_depth)?;
    let matcher = Arc::new(Matcher::new(&tree)?);
    let (events, _) = broadcast::channel(256);
    let app = sandpost_api::router(ApiState {
        storage: storage.clone(),
        events: events.clone(),
    });
    let http_listener = TcpListener::bind(config.http_listen).await?;
    let smtp_listener = TcpListener::bind(config.smtp_listen).await?;
    tracing::info!(http = %http_listener.local_addr()?, smtp = %smtp_listener.local_addr()?, database = %config.database_path.display(), "Sand Post ready (local development, no authentication)");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let http_shutdown = shutdown_rx.clone();
    let mut http = tokio::spawn(async move {
        axum::serve(http_listener, app)
            .with_graceful_shutdown(wait_shutdown(http_shutdown))
            .await
    });
    let mut smtp = tokio::spawn(sandpost_mail::serve(
        smtp_listener,
        move |message: Message| {
            let matcher = Arc::clone(&matcher);
            let storage = storage.clone();
            let events = events.clone();
            async move {
                let id = message.id;
                // Matching and all SQLite operations stay off the async runtime workers.
                let seq = tokio::task::spawn_blocking(move || {
                    let result = matcher.match_message(&message.facts);
                    tracing::debug!(
                        candidates = result.stats.candidates,
                        evaluations = result.stats.predicate_evaluations,
                        matched = result.scopes.len(),
                        "message matched"
                    );
                    storage.insert_message(&message, &result.scopes)
                })
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
                let _ = events.send(MessageEvent { id, seq });
                Ok::<(), String>(())
            }
        },
        wait_shutdown(shutdown_rx),
    ));

    // A service failure shuts down its sibling. SIGINT and SIGTERM both drain.
    let result: Result<(), Box<dyn Error + Send + Sync>> = tokio::select! {
        signal = termination_signal() => signal.map_err(Into::into),
        result = &mut http => match result { Ok(Ok(())) => Ok(()), Ok(Err(error)) => Err(error.into()), Err(error) => Err(error.into()) },
        result = &mut smtp => match result { Ok(Ok(())) => Ok(()), Ok(Err(error)) => Err(error.into()), Err(error) => Err(error.into()) },
    };
    let _ = shutdown_tx.send(true);
    tracing::info!("shutting down listeners");
    // SSE streams are long lived. Bound draining, then abort remaining transport tasks.
    let drain = async {
        if !http.is_finished() {
            let _ = (&mut http).await;
        }
        if !smtp.is_finished() {
            let _ = (&mut smtp).await;
        }
    };
    if tokio::time::timeout(std::time::Duration::from_secs(6), drain)
        .await
        .is_err()
    {
        http.abort();
        smtp.abort();
    }
    result
}

async fn wait_shutdown(mut receiver: watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
}

async fn termination_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result, _ = terminate.recv() => Ok(()) }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
