//! Authenticated Web message read status updates.

use crate::{
    ApplicationState,
    error::{HttpError, map_application_error},
};
use axum::{Extension, Json, extract::State};
use sandpost_application::SessionUser;
use sandpost_core::MessageIdentifier;
use serde::{Deserialize, Serialize};

/// A bounded update to the caller's shared read status.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetSeenRequest {
    identifiers: Vec<MessageIdentifier>,
    seen: bool,
}

/// Number of currently authorized message rows updated.
#[derive(Serialize)]
pub(super) struct SetSeenResponse {
    changed: u64,
}

/// Set or clear shared Seen on messages the caller can currently read.
pub(super) async fn set_seen(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<SetSeenRequest>,
) -> Result<Json<SetSeenResponse>, HttpError> {
    let updated = state
        .authorized(&user)
        .set_web_messages_seen(request.identifiers, request.seen)
        .await
        .map_err(map_application_error)?;
    Ok(Json(SetSeenResponse { changed: updated }))
}

/// Change supported shared primary flags with the same request semantics as the native boundary.
pub(super) async fn set_flags(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<sandpost_application::WebMessageFlagsRequest>,
) -> Result<Json<SetSeenResponse>, HttpError> {
    let (identifiers, change) = request.into_parts();
    let changed = state
        .authorized(&user)
        .set_web_messages_flags(identifiers, change)
        .await
        .map_err(map_application_error)?;
    Ok(Json(SetSeenResponse { changed }))
}
