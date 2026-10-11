//! Compatibility inbox endpoints backed by saved views.

use crate::{
    ApplicationState,
    error::{HttpError, map_application_error},
};
use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
};
use sandpost_application::{ApplicationError, InboxDetails, SessionUser};
use serde::Deserialize;

/// Return the available system and user-defined inboxes.
pub(super) async fn list_inboxes(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<InboxDetails>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_inboxes()
            .await
            .map_err(map_application_error)?,
    ))
}

#[derive(Deserialize)]
pub(super) struct NewInbox {
    name: String,
    address: String,
}

/// Create a named inbox from its display name and email address.
pub(super) async fn create_inbox(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(inbox): Json<NewInbox>,
) -> Result<Json<InboxDetails>, HttpError> {
    match state
        .authorized(&user)
        .create_inbox(inbox.name, inbox.address)
        .await
    {
        Ok(inbox) => Ok(Json(inbox)),
        Err(ApplicationError::Conflict) => Err(HttpError(
            StatusCode::CONFLICT,
            "an inbox for this address already exists".into(),
        )),
        Err(error) => Err(map_application_error(error)),
    }
}
