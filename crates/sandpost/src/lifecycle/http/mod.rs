//! Cancel HTTP handlers and response streams when runtime shutdown begins.

use super::supervision::wait_for_shutdown;
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::sync::watch;
use tokio_stream::{StreamExt, wrappers::WatchStream};

/// Cancel active handler work and terminate response streams on runtime shutdown.
pub(super) async fn cancel_on_shutdown(
    State(shutdown): State<watch::Receiver<bool>>,
    request: Request,
    next: Next,
) -> Response {
    if *shutdown.borrow() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    let response = tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown.clone()) => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        response = next.run(request) => response,
    };
    let (parts, body) = response.into_parts();
    let shutdown_stream = WatchStream::new(shutdown)
        .filter(|stopped| *stopped)
        .map(|_| None)
        .chain(tokio_stream::iter([None]));
    let body = Body::from_stream(
        body.into_data_stream()
            .map(Some)
            .chain(tokio_stream::iter([None]))
            .merge(shutdown_stream)
            .take_while(Option::is_some)
            .filter_map(|item| item),
    );
    Response::from_parts(parts, body)
}

#[cfg(test)]
mod tests;
