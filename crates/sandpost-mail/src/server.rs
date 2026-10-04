//! Connection acceptance, concurrency bounds, and graceful service shutdown.
use crate::{
    MailError,
    limits::{MAXIMUM_CONNECTION_COUNT, SHUTDOWN_DRAIN_TIMEOUT},
    session::mail_protocol_session,
};
use sandpost_core::Message;
use std::{fmt::Display, future::Future, sync::Arc};
use tokio::{net::TcpListener, task::JoinSet, time::timeout};

/// Serve SMTP sessions. The handler runs after valid DATA and must persist the
/// message before returning `Ok(())`; only then does the server acknowledge it.
/// The handler's error is returned to the SMTP peer as a temporary failure.
/// ESMTP parameters are unsupported and receive `501`.
pub async fn serve<Handler, HandlerFuture, HandlerError>(
    listener: TcpListener,
    handler: Handler,
    shutdown: impl Future<Output = ()>,
) -> Result<(), MailError>
where
    Handler: Fn(Message) -> HandlerFuture + Send + Sync + 'static,
    HandlerFuture: Future<Output = Result<(), HandlerError>> + Send + 'static,
    HandlerError: Display,
{
    let handler = Arc::new(handler);
    let mut sessions = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                if sessions.len() >= MAXIMUM_CONNECTION_COUNT {
                    drop(stream);
                    continue;
                }
                let handler = Arc::clone(&handler);
                sessions.spawn(async move {
                    if let Err(error) = mail_protocol_session(stream, handler).await {
                        tracing::debug!(%error, "SMTP session ended");
                    }
                });
            }
        }
    }
    if timeout(SHUTDOWN_DRAIN_TIMEOUT, async {
        while sessions.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        sessions.abort_all();
        while sessions.join_next().await.is_some() {}
    }
    Ok(())
}
