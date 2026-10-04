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
use sandpost_core::{Attachment, MessageFacts, MessageId, MessageSeq};
use sandpost_storage::{MessageSummary, Storage};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, time::Duration};
use tokio::sync::broadcast;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

#[derive(Debug, Clone, Serialize)]
pub struct MessageEvent {
    pub id: MessageId,
    pub seq: MessageSeq,
}

#[derive(Clone)]
pub struct ApiState {
    pub storage: Storage,
    pub events: broadcast::Sender<MessageEvent>,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/messages", get(list_messages))
        .route("/api/v1/messages/{id}", get(message))
        .route("/api/v1/messages/{id}/raw", get(raw_message))
        .route("/api/v1/events", get(events))
        .with_state(state)
}

#[derive(Debug)]
struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}
#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "API storage operation failed");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage operation failed",
    )
}

async fn health(State(state): State<ApiState>) -> Result<Json<Health>, ApiError> {
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
async fn list_messages(
    State(state): State<ApiState>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<MessageSummary>>, ApiError> {
    let limit = page.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "limit must be between 1 and 100",
        ));
    }
    let rows = tokio::task::spawn_blocking(move || {
        state
            .storage
            .list_messages(page.before.map(MessageSeq), limit)
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Serialize)]
struct MessageDetail {
    id: MessageId,
    facts: MessageFacts,
    attachments: Vec<Attachment>,
}
async fn message(
    State(state): State<ApiState>,
    Path(id): Path<MessageId>,
) -> Result<Json<MessageDetail>, ApiError> {
    let (facts, attachments) =
        tokio::task::spawn_blocking(move || state.storage.get_message_metadata(id))
            .await
            .map_err(internal)?
            .map_err(internal)?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "message not found"))?;
    Ok(Json(MessageDetail {
        id,
        facts,
        attachments,
    }))
}
async fn raw_message(
    State(state): State<ApiState>,
    Path(id): Path<MessageId>,
) -> Result<Response, ApiError> {
    let message = tokio::task::spawn_blocking(move || state.storage.get_message(id))
        .await
        .map_err(internal)?
        .map_err(internal)?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "message not found"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "message/rfc822"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=message.eml",
            ),
        ],
        message.raw_mime,
    )
        .into_response())
}

async fn events(
    State(state): State<ApiState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let updates = BroadcastStream::new(state.events.subscribe()).map(|event| {
        let event = match event {
            Ok(message) => match Event::default()
                .event("message")
                .id(message.seq.0.to_string())
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
    fn app() -> Router {
        let storage = Storage::memory().unwrap();
        let message = Message {
            id: MessageId::new(),
            facts: MessageFacts {
                subject: "hello".into(),
                ..MessageFacts::default()
            },
            raw_mime: b"Subject: hello\r\n\r\nbody".to_vec(),
            attachments: vec![],
        };
        storage.insert_message(&message, &[]).unwrap();
        router(ApiState {
            storage,
            events: broadcast::channel(16).0,
        })
    }
    #[tokio::test]
    async fn health_list_detail_raw_and_validation() {
        let app = app();
        let response = app
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
        let response = app
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
        let id = rows[0]["id"].as_str().unwrap();
        for suffix in ["", "/raw"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/messages/{id}{suffix}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = app
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
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/messages/{}", MessageId::new()))
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
        let app = app();
        for resource_path in ["/", "/app.js", "/style.css"] {
            let response = app
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
        let response = app
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
    #[tokio::test]
    async fn slow_sse_subscribers_receive_resync_instead_of_silent_loss() {
        let (events, _) = broadcast::channel(1);
        let app = router(ApiState {
            storage: Storage::memory().unwrap(),
            events: events.clone(),
        });
        let response = app
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
        for seq in 1..=3 {
            events
                .send(MessageEvent {
                    id: MessageId::new(),
                    seq: MessageSeq(seq),
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
