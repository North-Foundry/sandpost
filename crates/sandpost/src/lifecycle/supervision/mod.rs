//! Supervise transport and indexing tasks and bound their coordinated shutdown.

use super::RuntimeError;
use std::time::Duration;
use tokio::{sync::watch, task::JoinHandle};

/// Shut down a sibling on failure and bound draining of long-lived HTTP event streams.
pub(super) async fn supervise(
    mut web_server_task: JoinHandle<std::io::Result<()>>,
    mut mail_server_task: JoinHandle<std::io::Result<()>>,
    mut imap_server_task: JoinHandle<std::io::Result<()>>,
    mut indexing_task: JoinHandle<Result<(), sandpost_application::ApplicationError>>,
    shutdown_sender: watch::Sender<bool>,
    shutdown_receiver: watch::Receiver<bool>,
) -> (Result<(), RuntimeError>, tokio::time::Instant) {
    let (
        mut result,
        mut web_completed,
        mut mail_completed,
        mut imap_completed,
        mut indexing_completed,
    ): (Result<(), RuntimeError>, bool, bool, bool, bool) = tokio::select! {
        _ = wait_for_shutdown(shutdown_receiver) => (Ok(()), false, false, false, false),
        result = &mut web_server_task => (completed_task_result(result), true, false, false, false),
        result = &mut mail_server_task => (completed_task_result(result), false, true, false, false),
        result = &mut imap_server_task => (completed_task_result(result), false, false, true, false),
        result = &mut indexing_task => (completed_task_result(result), false, false, false, true),
    };
    let _ = shutdown_sender.send(true);
    tracing::info!("shutting down listeners");
    let shutdown_deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let deadline = tokio::time::sleep_until(shutdown_deadline);
    tokio::pin!(deadline);
    while !(web_completed && mail_completed && imap_completed && indexing_completed) {
        let completion = tokio::select! {
            completion = &mut web_server_task, if !web_completed => {
                web_completed = true;
                completed_task_result(completion)
            }
            completion = &mut mail_server_task, if !mail_completed => {
                mail_completed = true;
                completed_task_result(completion)
            }
            completion = &mut imap_server_task, if !imap_completed => {
                imap_completed = true;
                completed_task_result(completion)
            }
            completion = &mut indexing_task, if !indexing_completed => {
                indexing_completed = true;
                completed_task_result(completion)
            }
            _ = &mut deadline => {
                web_server_task.abort();
                mail_server_task.abort();
                imap_server_task.abort();
                indexing_task.abort();
                return (result, shutdown_deadline);
            }
        };
        result = result.and(completion);
    }
    (result, shutdown_deadline)
}

/// Preserve either a task's domain failure or its join failure at the runtime boundary.
fn completed_task_result<Failure>(
    completion: Result<Result<(), Failure>, tokio::task::JoinError>,
) -> Result<(), RuntimeError>
where
    Failure: Into<RuntimeError>,
{
    match completion {
        Ok(result) => result.map_err(Into::into),
        Err(error) => Err(error.into()),
    }
}

/// Resolve when shutdown is requested or the shutdown sender is dropped.
pub(super) async fn wait_for_shutdown(mut shutdown_receiver: watch::Receiver<bool>) {
    if *shutdown_receiver.borrow() {
        return;
    }
    while shutdown_receiver.changed().await.is_ok() {
        if *shutdown_receiver.borrow() {
            return;
        }
    }
}

/// Bound the host's wait for cleanup without blocking its async executor or cancelling cleanup.
/// A timed-out operation continues on its own thread so writer ownership remains protected.
pub(super) async fn run_shutdown_cleanup(
    cleanup: impl FnOnce() -> Result<(), RuntimeError> + Send + 'static,
    deadline: tokio::time::Instant,
) -> Result<(), RuntimeError> {
    let (completed, completion) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("sandpost-shutdown-cleanup".into())
        .spawn(move || {
            let _ = completed.send(cleanup());
        })?;
    match tokio::time::timeout_at(deadline, completion).await {
        Ok(completion) => completion?,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "search writer cleanup exceeded the shutdown deadline; cleanup continues in background",
        )
        .into()),
    }
}

#[cfg(test)]
mod tests;
