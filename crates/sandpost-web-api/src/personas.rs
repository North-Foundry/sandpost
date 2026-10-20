//! User-owned Personas, PersonaScript validation, the Persona mail view, and execution history.

use crate::{
    ApplicationState,
    error::{HttpError, map_application_error},
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use sandpost_application::{
    MessageSummary, PersonaDetails, PersonaEnabledRequest, PersonaExecutionDetails,
    PersonaExecutionSummary, PersonaRequest, PersonaScriptRequest, PersonaScriptValidation,
    QueryOptions, SessionUser,
};
use sandpost_core::{MessageSequence, PersonaExecutionIdentifier, PersonaIdentifier};
use serde::Deserialize;

/// Which Personas a listing returns.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PersonaListing {
    /// `all` lists every user's Personas (Owners only); omitted lists the caller's own.
    owner: Option<String>,
}

/**
 * List the caller's Personas, or every Persona for an Owner with `?owner=all`.
 */
pub(super) async fn list_personas(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Query(listing): Query<PersonaListing>,
) -> Result<Json<Vec<PersonaDetails>>, HttpError> {
    let caller = state.authorized(&user);
    let personas = match listing.owner.as_deref() {
        None => caller.list_personas().await,
        Some("all") => caller.list_all_personas().await,
        Some(_) => {
            return Err(HttpError(
                StatusCode::BAD_REQUEST,
                "owner must be omitted or \"all\"".into(),
            ));
        }
    };
    Ok(Json(personas.map_err(map_application_error)?))
}

/**
 * Create a Persona owned by the caller.
 */
pub(super) async fn create_persona(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<PersonaRequest>,
) -> Result<(StatusCode, Json<PersonaDetails>), HttpError> {
    let persona = state
        .authorized(&user)
        .create_persona(request)
        .await
        .map_err(map_application_error)?;
    Ok((StatusCode::CREATED, Json(persona)))
}

/**
 * Read one visible Persona.
 */
pub(super) async fn persona(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<PersonaIdentifier>,
) -> Result<Json<PersonaDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .get_persona(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Replace the caller's own Persona definition.
 */
pub(super) async fn update_persona(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<PersonaIdentifier>,
    Json(request): Json<PersonaRequest>,
) -> Result<Json<PersonaDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_persona(identifier, request)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Enable or disable a visible Persona.
 */
pub(super) async fn set_persona_enabled(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<PersonaIdentifier>,
    Json(request): Json<PersonaEnabledRequest>,
) -> Result<Json<PersonaDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .set_persona_enabled(identifier, request.enabled)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Delete a visible Persona and its history; its mail is kept.
 */
pub(super) async fn delete_persona(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<PersonaIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_persona(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/**
 * Validate PersonaScript source without saving or running it.
 */
pub(super) async fn validate_persona_script(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<PersonaScriptRequest>,
) -> Result<Json<PersonaScriptValidation>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .validate_persona_script(&request.script)
            .await
            .map_err(map_application_error)?,
    ))
}

/// A bounded newest-first page.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PersonaPage {
    before: Option<u64>,
    limit: Option<usize>,
    q: Option<String>,
}

/**
 * List readable mail addressed to a visible Persona, optionally restricted by a query.
 */
pub(super) async fn persona_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<PersonaIdentifier>,
    Query(page): Query<PersonaPage>,
) -> Result<Json<Vec<MessageSummary>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .query_persona_messages(
                identifier,
                QueryOptions {
                    query: page.q,
                    before: page.before.map(MessageSequence),
                    limit: page.limit.unwrap_or(50),
                    ..QueryOptions::default()
                },
            )
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * List a visible Persona's executions whose messages the caller can read.
 */
pub(super) async fn persona_executions(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<PersonaIdentifier>,
    Query(page): Query<PersonaPage>,
) -> Result<Json<Vec<PersonaExecutionSummary>>, HttpError> {
    if page.q.is_some() {
        return Err(HttpError(
            StatusCode::BAD_REQUEST,
            "execution history does not accept a query".into(),
        ));
    }
    Ok(Json(
        state
            .authorized(&user)
            .list_persona_executions(identifier, page.before, page.limit.unwrap_or(50))
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Read one execution with its recorded steps.
 */
pub(super) async fn persona_execution(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path((identifier, execution)): Path<(PersonaIdentifier, PersonaExecutionIdentifier)>,
) -> Result<Json<PersonaExecutionDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .get_persona_execution(identifier, execution)
            .await
            .map_err(map_application_error)?,
    ))
}
