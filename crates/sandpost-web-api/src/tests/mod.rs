//! Shared fixtures and external imports for HTTP unit tests.
use crate::{ApplicationState, MessageEvent, router, router_with_frontend};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    middleware::{self, Next},
};
use http_body_util::BodyExt;
use sandpost_application::{LoginRequest, NewAccount, SetupRequest};
use sandpost_core::{
    GlobalRole, Message, MessageFacts, MessageIdentifier, MessageSequence, View, ViewIdentifier,
};
use sandpost_storage::Storage;
use sandpost_storage_sqlite::SqliteStorage;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

const TEST_EMAIL: &str = "owner@example.test";
const TEST_PASSWORD: &str = "a-secure-test-password";

/// Construct one fresh in-memory backend-neutral storage handle.
async fn memory_storage() -> Arc<dyn Storage> {
    Arc::new(
        SqliteStorage::memory()
            .await
            .expect("in-memory storage should initialize"),
    )
}

/// Build initialized application state and one valid owner token for HTTP tests.
async fn initialized_state(
    storage: Arc<dyn Storage>,
    events: tokio::sync::broadcast::Sender<MessageEvent>,
) -> (ApplicationState, String) {
    let directory = std::env::temp_dir().join(format!(
        "sandpost-web-api-instance-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let instance = sandpost_application::Instance::open(storage.clone(), directory)
        .await
        .unwrap();
    let bootstrap_secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
    instance
        .setup(SetupRequest {
            bootstrap_secret,
            name: "Test Owner".into(),
            email: TEST_EMAIL.into(),
            password: TEST_PASSWORD.into(),
        })
        .await
        .unwrap();
    let token = instance
        .login(LoginRequest {
            email: TEST_EMAIL.into(),
            password: TEST_PASSWORD.into(),
        })
        .await
        .unwrap()
        .token;
    (
        ApplicationState::with_instance(storage, events, instance)
            .await
            .unwrap(),
        token,
    )
}

/// Attach one fixture token to HTTP test requests that exercise existing API routes.
fn with_test_auth(application: Router, token: String) -> Router {
    application.layer(middleware::from_fn(
        move |mut request: Request<Body>, next: Next| {
            let authorization = format!("Bearer {token}");
            async move {
                request
                    .headers_mut()
                    .insert(header::AUTHORIZATION, authorization.parse().unwrap());
                next.run(request).await
            }
        },
    ))
}

/// Build an initialized test router and authenticate its ordinary requests.
async fn initialized_router(storage: Arc<dyn Storage>) -> Router {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, token) = initialized_state(storage, events).await;
    with_test_auth(router(state), token)
}

/// Build a test router containing one sample message.
async fn build_test_application() -> Router {
    let storage = memory_storage().await;
    let message = Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            subject: "hello".into(),
            ..MessageFacts::default()
        },
        raw_message: b"Subject: hello\r\n\r\nbody".to_vec(),
        attachments: vec![],
    };
    storage.insert_message(&message).await.unwrap();
    initialized_router(storage).await
}

mod authentication;
mod events;
mod imap;
mod management;
mod messages;
mod routing;
mod smtp;
