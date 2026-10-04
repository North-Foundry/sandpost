//! Versioned HTTP transport for the local development catcher.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use sandpost_core::{Attachment, MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_storage::{MessageSummary, Storage};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, time::Duration};
use tokio::sync::broadcast;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

#[derive(Debug, Clone, Serialize)]
pub struct MessageEvent {
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    #[serde(rename = "seq")]
    pub sequence: MessageSequence,
}

#[derive(Clone)]
pub struct ApplicationState {
    pub storage: Storage,
    pub events: broadcast::Sender<MessageEvent>,
}

/// Build the HTTP router for the catcher API.
pub fn router(state: ApplicationState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/messages", get(list_messages))
        .route("/api/v1/messages/{identifier}", get(message))
        .route("/api/v1/messages/{identifier}/raw", get(raw_message))
        .route("/api/v1/events", get(events))
        .with_state(state)
}

#[derive(Debug)]
struct ApplicationError(StatusCode, &'static str);
impl IntoResponse for ApplicationError {
    /// Convert the application error into its status and JSON error response.
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}
#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

/// Log an internal failure and return a generic server error to the client.
fn internal(error: impl std::fmt::Display) -> ApplicationError {
    tracing::error!(%error, "API storage operation failed");
    ApplicationError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage operation failed",
    )
}

/// Report whether the backing storage can answer a health query.
async fn health(State(state): State<ApplicationState>) -> Result<Json<Health>, ApplicationError> {
    tokio::task::spawn_blocking(move || state.storage.health())
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(Json(Health {
        status: "ok",
        mode: "local-development",
    }))
}
#[derive(Serialize)]
struct Health {
    status: &'static str,
    mode: &'static str,
}

#[derive(Default, Deserialize)]
struct Page {
    before: Option<u64>,
    limit: Option<usize>,
}
/// Return a page of stored messages with the requested cursor and limit.
async fn list_messages(
    State(state): State<ApplicationState>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<MessageSummary>>, ApplicationError> {
    let limit = page.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(ApplicationError(
            StatusCode::BAD_REQUEST,
            "limit must be between 1 and 100",
        ));
    }
    let rows = tokio::task::spawn_blocking(move || {
        state
            .storage
            .list_messages(page.before.map(MessageSequence), limit)
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Serialize)]
struct MessageDetail {
    #[serde(rename = "id")]
    identifier: MessageIdentifier,
    facts: MessageFacts,
    attachments: Vec<Attachment>,
}
/// Return parsed facts and attachment metadata for the requested message.
async fn message(
    State(state): State<ApplicationState>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<Json<MessageDetail>, ApplicationError> {
    let (facts, attachments) =
        tokio::task::spawn_blocking(move || state.storage.get_message_metadata(identifier))
            .await
            .map_err(internal)?
            .map_err(internal)?
            .ok_or(ApplicationError(StatusCode::NOT_FOUND, "message not found"))?;
    Ok(Json(MessageDetail {
        identifier,
        facts,
        attachments,
    }))
}
/// Return the original RFC 822 bytes for the requested message.
async fn raw_message(
    State(state): State<ApplicationState>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<Response, ApplicationError> {
    let message = tokio::task::spawn_blocking(move || state.storage.get_message(identifier))
        .await
        .map_err(internal)?
        .map_err(internal)?
        .ok_or(ApplicationError(StatusCode::NOT_FOUND, "message not found"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "message/rfc822"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=message.eml",
            ),
        ],
        message.raw_message,
    )
        .into_response())
}

/// Stream message notifications and resynchronization events to subscribers.
async fn events(
    State(state): State<ApplicationState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let updates = BroadcastStream::new(state.events.subscribe()).map(|event| {
        let event = match event {
            Ok(message) => match Event::default()
                .event("message")
                .id(message.sequence.0.to_string())
                .json_data(message)
            {
                Ok(event) => event,
                Err(error) => {
                    tracing::error!(%error, "event serialization failed");
                    Event::default().event("resync").data("{}")
                }
            },
            Err(_) => Event::default().event("resync").data("{}"),
        };
        Ok(event)
    });
    let ready = tokio_stream::once(Ok(Event::default().event("ready").data("{}")));
    Sse::new(ready.chain(updates)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use sandpost_core::Message;
    use tower::ServiceExt;
    /// Build a test router containing one sample message.
    fn build_test_application() -> Router {
        let storage = Storage::memory().unwrap();
        let message = Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: "hello".into(),
                ..MessageFacts::default()
            },
            raw_message: b"Subject: hello\r\n\r\nbody".to_vec(),
            attachments: vec![],
        };
        storage.insert_message(&message, &[]).unwrap();
        router(ApplicationState {
            storage,
            events: broadcast::channel(16).0,
        })
    }
    /// Verify health, listing, detail, raw-message, and pagination error responses.
    #[tokio::test]
    async fn health_list_detail_raw_and_validation() {
        let application = build_test_application();
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let rows: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rows[0]["subject"], "hello");
        let identifier = rows[0]["id"].as_str().unwrap();
        for suffix in ["", "/raw"] {
            let response = application
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/messages/{identifier}{suffix}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/messages?limit=101")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = application
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/messages/{}", MessageIdentifier::new()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    /// Verify absent frontend routes and the initial server-sent event handshake.
    #[tokio::test]
    async fn absent_frontend_routes_and_initial_server_sent_event_handshake() {
        let application = build_test_application();
        for resource_path in ["/", "/app.js", "/style.css"] {
            let response = application
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(resource_path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        let response = application
            .oneshot(
                Request::builder()
                    .uri("/api/v1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        let mut body = response.into_body();
        let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            std::str::from_utf8(frame.data_ref().unwrap())
                .unwrap()
                .contains("event: ready")
        );
    }
    /// Verify lagging event subscribers receive a resynchronization event.
    #[tokio::test]
    async fn slow_server_sent_event_subscribers_receive_resynchronization_instead_of_silent_loss() {
        let (events, _) = broadcast::channel(1);
        let application = router(ApplicationState {
            storage: Storage::memory().unwrap(),
            events: events.clone(),
        });
        let response = application
            .oneshot(
                Request::builder()
                    .uri("/api/v1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut body = response.into_body();
        body.frame().await.unwrap().unwrap();
        for sequence in 1..=3 {
            events
                .send(MessageEvent {
                    identifier: MessageIdentifier::new(),
                    sequence: MessageSequence(sequence),
                })
                .unwrap();
        }
        let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            std::str::from_utf8(frame.data_ref().unwrap())
                .unwrap()
                .contains("event: resync")
        );
    }
}
