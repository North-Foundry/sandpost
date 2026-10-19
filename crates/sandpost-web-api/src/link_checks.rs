//! Reading and starting shared link checks of authorized messages.

use crate::{
    ApplicationState,
    error::{HttpError, map_application_error},
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use sandpost_application::{
    BulkLinkCheckRequest, BulkLinkCheckResult, LinkCheckReport, SessionUser,
};
use sandpost_core::MessageIdentifier;

/**
 * Return the stored and running link check state of a message without starting a check.
 */
pub(super) async fn link_check(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<Json<LinkCheckReport>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .get_link_check(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Start, or join, a background check of a message's links and return 202 with its state.
 */
pub(super) async fn start_link_check(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<(StatusCode, Json<LinkCheckReport>), HttpError> {
    let report = state
        .authorized(&user)
        .start_link_check(identifier)
        .await
        .map_err(map_application_error)?;
    Ok((StatusCode::ACCEPTED, Json(report)))
}

/**
 * Start background checks of several messages and return 202 with what each request started.
 */
pub(super) async fn start_bulk_link_checks(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<BulkLinkCheckRequest>,
) -> Result<(StatusCode, Json<BulkLinkCheckResult>), HttpError> {
    let result = state
        .authorized(&user)
        .start_bulk_link_checks(request)
        .await
        .map_err(map_application_error)?;
    Ok((StatusCode::ACCEPTED, Json(result)))
}
