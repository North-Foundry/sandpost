//! Stable HTTP status codes and JSON responses for application failures.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sandpost_application::{ApplicationError, InstanceError};
use serde::Serialize;

/// Return a stable JSON authentication or setup error.
pub(super) fn auth_error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(ErrorBody { error: code.into() })).into_response()
}

/// Translate lifecycle errors into stable HTTP status codes and error names.
pub(super) fn map_instance_error(error: InstanceError) -> HttpError {
    match error {
        InstanceError::Authentication => {
            HttpError(StatusCode::UNAUTHORIZED, "authentication_failed".into())
        }
        InstanceError::SetupRequired => {
            HttpError(StatusCode::SERVICE_UNAVAILABLE, "setup_required".into())
        }
        InstanceError::AlreadyInitialized => {
            HttpError(StatusCode::CONFLICT, "already_initialized".into())
        }
        InstanceError::InvalidInput(message) => HttpError(StatusCode::BAD_REQUEST, message),
        InstanceError::RecoveryRequired => {
            HttpError(StatusCode::SERVICE_UNAVAILABLE, "recovery_required".into())
        }
        InstanceError::ReadMetadata(error) | InstanceError::WriteMetadata(error) => {
            tracing::error!(%error, "instance metadata operation failed");
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "instance_unavailable".into(),
            )
        }
        InstanceError::CreateDirectory(error) => {
            tracing::error!(%error, "instance directory operation failed");
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "instance_unavailable".into(),
            )
        }
        InstanceError::Storage(error) => {
            tracing::error!(%error, "instance storage operation failed");
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "instance_unavailable".into(),
            )
        }
        InstanceError::Worker(error) => {
            tracing::error!(%error, "instance worker failed");
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "instance_unavailable".into(),
            )
        }
        InstanceError::Cryptography(error) => {
            tracing::error!(%error, "instance cryptography failed");
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "instance_unavailable".into(),
            )
        }
    }
}

#[derive(Debug)]
pub(super) struct HttpError(pub(super) StatusCode, pub(super) String);
impl IntoResponse for HttpError {
    /// Convert the application error into its status and JSON error response.
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

/// Convert an application failure into the established HTTP error response.
pub(super) fn map_application_error(error: ApplicationError) -> HttpError {
    match error {
        ApplicationError::Forbidden => HttpError(StatusCode::FORBIDDEN, "forbidden".into()),
        ApplicationError::Unavailable(_) => HttpError(
            StatusCode::SERVICE_UNAVAILABLE,
            "service unavailable".into(),
        ),
        ApplicationError::Search(error) => {
            tracing::error!(%error, "search operation failed");
            HttpError(StatusCode::SERVICE_UNAVAILABLE, "search unavailable".into())
        }
        ApplicationError::Io(error) => {
            tracing::error!(%error, "application I/O operation failed");
            HttpError(StatusCode::INTERNAL_SERVER_ERROR, "operation failed".into())
        }
        ApplicationError::Instance(error) => map_instance_error(error),
        ApplicationError::InvalidPage => HttpError(
            StatusCode::BAD_REQUEST,
            "limit must be between 1 and 100".into(),
        ),
        ApplicationError::InvalidInput(message) => HttpError(StatusCode::BAD_REQUEST, message),
        ApplicationError::Conflict => HttpError(StatusCode::CONFLICT, "conflict".into()),
        ApplicationError::InUse(message) => HttpError(StatusCode::CONFLICT, message),
        ApplicationError::NotFound => HttpError(StatusCode::NOT_FOUND, "item not found".into()),
        ApplicationError::Storage(error) => {
            tracing::error!(%error, "API storage operation failed");
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage operation failed".into(),
            )
        }
        ApplicationError::Worker(error) => {
            tracing::error!(%error, "API storage operation failed");
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage operation failed".into(),
            )
        }
    }
}
