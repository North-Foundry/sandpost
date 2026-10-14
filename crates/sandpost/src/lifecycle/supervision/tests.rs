//! Failure propagation while supervised tasks drain.

use super::*;

/// Report listener failures that happen after the host has already requested shutdown.
#[tokio::test]
async fn supervisor_preserves_listener_failures_during_shutdown() {
    let (shutdown_sender, shutdown_receiver) = watch::channel(true);
    let web_server_task = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Err(std::io::Error::other("web drain failed"))
    });
    let mail_server_task = tokio::spawn(async { Ok(()) });
    let imap_server_task = tokio::spawn(async { Ok(()) });
    let indexing_task = tokio::spawn(async { Ok(()) });
    let error = supervise(
        web_server_task,
        mail_server_task,
        imap_server_task,
        indexing_task,
        shutdown_sender,
        shutdown_receiver,
    )
    .await
    .0
    .unwrap_err();
    assert!(error.to_string().contains("web drain failed"));
}

/// Supervision retains indexing failure even when shutdown races worker completion.
#[tokio::test]
async fn supervision_reports_indexing_failure_during_shutdown() {
    let (shutdown_sender, shutdown_receiver) = watch::channel(true);
    let web = tokio::spawn(async { Ok(()) });
    let mail = tokio::spawn(async { Ok(()) });
    let imap = tokio::spawn(async { Ok(()) });
    let indexing = tokio::spawn(async {
        Err(sandpost_application::ApplicationError::Unavailable(
            "indexing failed".into(),
        ))
    });
    let error = supervise(
        web,
        mail,
        imap,
        indexing,
        shutdown_sender,
        shutdown_receiver,
    )
    .await
    .0
    .unwrap_err();
    assert!(error.to_string().contains("indexing failed"));
}

/// A stuck HTTP drain cannot hide a worker failure that completes before the shared deadline.
#[tokio::test]
async fn supervision_preserves_worker_failure_when_listener_drain_times_out() {
    let (shutdown_sender, shutdown_receiver) = watch::channel(true);
    let web_server_task = tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(())
    });
    let mail_server_task = tokio::spawn(async { Ok(()) });
    let imap_server_task = tokio::spawn(async { Ok(()) });
    let indexing_task = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        Err(sandpost_application::ApplicationError::Unavailable(
            "indexing failed during stalled drain".into(),
        ))
    });
    let error = supervise(
        web_server_task,
        mail_server_task,
        imap_server_task,
        indexing_task,
        shutdown_sender,
        shutdown_receiver,
    )
    .await
    .0
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("indexing failed during stalled drain")
    );
}

/// Blocking cleanup keeps running after the host's deadline without extending its wait.
#[tokio::test]
async fn shutdown_cleanup_timeout_leaves_cleanup_running() {
    let (started, start) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (finished, finish) = tokio::sync::oneshot::channel();
    let cleanup = tokio::spawn(run_shutdown_cleanup(
        move || {
            let _ = started.send(());
            released.recv()?;
            let _ = finished.send(());
            Ok(())
        },
        tokio::time::Instant::now() + Duration::from_millis(100),
    ));
    start.await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), cleanup)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cleanup continues in background")
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), finish)
        .await
        .unwrap()
        .unwrap();
}

/// A contended async writer gate cannot hold the host beyond the shutdown deadline.
#[tokio::test]
async fn shutdown_cleanup_bounds_a_contended_writer_gate() {
    let gate = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let held_gate = gate.clone().lock_owned().await;
    let runtime = tokio::runtime::Handle::current();
    let (finished, finish) = tokio::sync::oneshot::channel();
    let error = run_shutdown_cleanup(
        move || {
            runtime.block_on(async move {
                let _gate = gate.lock().await;
                let _ = finished.send(());
            });
            Ok(())
        },
        tokio::time::Instant::now() + Duration::from_millis(100),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cleanup continues in background")
    );
    drop(held_gate);
    tokio::time::timeout(Duration::from_secs(1), finish)
        .await
        .unwrap()
        .unwrap();
}
