//! Administration of the global SMTP server and its SMTP access credentials.
//!
//! Only global administrators may use these routes; the application service enforces that. The
//! creation and regeneration responses are the only ones that carry a password, and they are
//! marked `no-store` so neither the browser nor an intermediary caches the secret.
use crate::{
    ApplicationState,
    error::{HttpError, map_application_error},
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use sandpost_application::{
    IssuedSmtpCredentials, NewSmtpAccessRequest, SessionUser, SmtpAccessUpdate, SmtpServerDetails,
    SmtpServerUpdate,
};
use sandpost_core::{SmtpAccess, SmtpAccessIdentifier};

/// Return the global SMTP server address and listener status.
pub(super) async fn smtp_server(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<SmtpServerDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .smtp_server()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Persist the global SMTP server address and rebind its listener immediately.
pub(super) async fn save_smtp_server(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(update): Json<SmtpServerUpdate>,
) -> Result<Json<SmtpServerDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .save_smtp_server(update)
            .await
            .map_err(map_application_error)?,
    ))
}

/// List SMTP accesses without passwords or hashes.
pub(super) async fn list_smtp_accesses(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<SmtpAccess>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_smtp_accesses()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Create an SMTP access and return its generated password exactly once.
pub(super) async fn create_smtp_access(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<NewSmtpAccessRequest>,
) -> Result<Response, HttpError> {
    let issued = state
        .authorized(&user)
        .create_smtp_access(request)
        .await
        .map_err(map_application_error)?;
    Ok(secret_response(StatusCode::CREATED, issued))
}

/// Replace an SMTP access's password and return the new one exactly once.
pub(super) async fn regenerate_smtp_access_password(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpAccessIdentifier>,
) -> Result<Response, HttpError> {
    let issued = state
        .authorized(&user)
        .regenerate_smtp_access_password(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(secret_response(StatusCode::OK, issued))
}

/// Change the status or authentication rules of the SMTP access named by the route.
pub(super) async fn update_smtp_access(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpAccessIdentifier>,
    Json(update): Json<SmtpAccessUpdate>,
) -> Result<Json<SmtpAccess>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_smtp_access(identifier, update)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Delete the SMTP access named by the route.
pub(super) async fn delete_smtp_access(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpAccessIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_smtp_access(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Serialize issued credentials with headers that forbid caching the password.
fn secret_response(status: StatusCode, issued: IssuedSmtpCredentials) -> Response {
    let mut response = (status, Json(issued)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}
