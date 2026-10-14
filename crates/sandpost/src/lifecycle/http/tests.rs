//! Shutdown cancellation tests for HTTP handlers and streaming bodies.

use super::cancel_on_shutdown;
use axum::{Router, body::Body, http::StatusCode, middleware, routing::get};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceExt;

/// Finite responses finish normally while the runtime remains running.
#[tokio::test]
async fn finite_response_finishes_without_shutdown() {
    let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
    let router = Router::new()
        .route("/", get(|| async { "complete response" }))
        .layer(middleware::from_fn_with_state(
            shutdown_receiver,
            cancel_on_shutdown,
        ));
    let response = router
        .oneshot(axum::http::Request::new(Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio::time::timeout(Duration::from_secs(1), response.into_body().collect())
        .await
        .expect("normal response EOF must not wait for shutdown")
        .unwrap()
        .to_bytes();
    assert_eq!(body, "complete response");
}

/// Verify shutdown drops a pending handler future and releases its held lock.
#[tokio::test]
async fn shutdown_cancels_handler_and_releases_held_lock() {
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let lock = Arc::new(Mutex::new(()));
    let handler_lock = lock.clone();
    let started = Arc::new(Notify::new());
    let handler_started = started.clone();
    let router = Router::new()
        .route(
            "/",
            get(move || {
                let handler_lock = handler_lock.clone();
                let handler_started = handler_started.clone();
                async move {
                    let _guard = handler_lock.lock().await;
                    handler_started.notify_one();
                    std::future::pending::<()>().await;
                }
            }),
        )
        .layer(middleware::from_fn_with_state(
            shutdown_receiver,
            cancel_on_shutdown,
        ));

    let request = tokio::spawn(router.oneshot(axum::http::Request::new(Body::empty())));
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .expect("handler acquired its lock");
    shutdown_sender
        .send(true)
        .expect("shutdown receiver exists");
    let response = tokio::time::timeout(Duration::from_secs(1), request)
        .await
        .expect("shutdown cancelled the request")
        .expect("request task completed")
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(lock.try_lock().is_ok(), "handler lock was released");
}

/// Verify shutdown ends a response whose streaming body is still pending.
#[tokio::test]
async fn shutdown_ends_pending_response_stream() {
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let (body_sender, body_receiver) =
        mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(1);
    let body_receiver = Arc::new(std::sync::Mutex::new(Some(body_receiver)));
    let handler_body_receiver = body_receiver.clone();
    let router = Router::new()
        .route(
            "/",
            get(move || {
                let body_receiver = handler_body_receiver
                    .lock()
                    .expect("body receiver lock")
                    .take()
                    .expect("single request");
                async move {
                    let mut response = axum::http::Response::new(Body::from_stream(
                        ReceiverStream::new(body_receiver),
                    ));
                    response
                        .headers_mut()
                        .insert("x-preserved", "yes".parse().expect("valid header"));
                    response
                }
            }),
        )
        .layer(middleware::from_fn_with_state(
            shutdown_receiver,
            cancel_on_shutdown,
        ));
    let response = router
        .oneshot(axum::http::Request::new(Body::empty()))
        .await
        .expect("request succeeds");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-preserved"], "yes");
    let mut body = response.into_body();
    body_sender
        .send(Ok(axum::body::Bytes::from_static(b"running")))
        .await
        .expect("body receiver exists");
    assert_eq!(
        body.frame()
            .await
            .expect("body chunk exists")
            .expect("body frame succeeds")
            .into_data()
            .expect("body frame contains data"),
        "running"
    );
    shutdown_sender
        .send(true)
        .expect("shutdown receiver exists");
    assert!(
        tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .expect("shutdown ended the pending body")
            .is_none(),
        "stream ended on shutdown"
    );
}
