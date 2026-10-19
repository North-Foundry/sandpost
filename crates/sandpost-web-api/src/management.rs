//! Scope, view, account, and search-index administration.

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
    AccountDetails, MessageSummary, NewAccount, QueryValidation, ScopePreview,
    SearchSynchronization, SessionUser, UpdateAccount,
};
use sandpost_core::{Scope, ScopeIdentifier, UserIdentifier, View, ViewIdentifier};
use serde::Deserialize;

/// Return configured scopes when the caller may manage scope policy.
pub(super) async fn list_scopes(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<Scope>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_scopes()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Create or replace the scope identified by the submitted record.
pub(super) async fn save_scope(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(scope): Json<Scope>,
) -> Result<Json<Scope>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .save_scope(scope)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Replace a scope only when its body and path identifiers agree.
pub(super) async fn save_scope_at(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ScopeIdentifier>,
    Json(scope): Json<Scope>,
) -> Result<Json<Scope>, HttpError> {
    if scope.identifier != identifier {
        return Err(HttpError(
            StatusCode::BAD_REQUEST,
            "scope identifier does not match path".into(),
        ));
    }
    save_scope(State(state), Extension(user), Json(scope)).await
}

/// Delete the scope named by the route.
pub(super) async fn delete_scope(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ScopeIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_scope(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sample the newest mail an unsaved scope filter would select, inheriting its parent chain.
pub(super) async fn preview_scope(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(preview): Json<ScopePreview>,
) -> Result<Json<Vec<MessageSummary>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .preview_scope(preview)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Query source submitted for validation.
#[derive(Deserialize)]
pub(super) struct QueryValidationRequest {
    query: String,
}

/// Check query syntax and types without saving or running the query.
pub(super) async fn validate_query(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<QueryValidationRequest>,
) -> Json<QueryValidation> {
    Json(state.authorized(&user).validate_query(&request.query))
}

/// List user identifiers assigned to the selected scope.
pub(super) async fn list_scope_assignments(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ScopeIdentifier>,
) -> Result<Json<Vec<UserIdentifier>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_scope_assignments(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Assign the path user to the path scope.
pub(super) async fn assign_scope(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path((scope, account)): Path<(ScopeIdentifier, UserIdentifier)>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .assign_scope(account, scope)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove the path user's assignment from the path scope.
pub(super) async fn remove_scope_assignment(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path((scope, account)): Path<(ScopeIdentifier, UserIdentifier)>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .remove_scope_assignment(account, scope)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Return saved views when the caller may inspect them.
pub(super) async fn list_views(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<View>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_views()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Create or replace the view identified by the submitted record.
pub(super) async fn save_view(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(view): Json<View>,
) -> Result<Json<View>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .save_view(view)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Replace a view only when its body and path identifiers agree.
pub(super) async fn save_view_at(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ViewIdentifier>,
    Json(view): Json<View>,
) -> Result<Json<View>, HttpError> {
    if view.identifier != identifier {
        return Err(HttpError(
            StatusCode::BAD_REQUEST,
            "view identifier does not match path".into(),
        ));
    }
    save_view(State(state), Extension(user), Json(view)).await
}

/// Delete the view named by the route.
pub(super) async fn delete_view(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ViewIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_view(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Return account details without password hashes.
pub(super) async fn list_users(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<AccountDetails>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_users()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Create an account with the submitted identity, role and mail access.
pub(super) async fn create_user(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(account): Json<NewAccount>,
) -> Result<(StatusCode, Json<AccountDetails>), HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .authorized(&user)
                .create_user(account)
                .await
                .map_err(map_application_error)?,
        ),
    ))
}

/// Update the account named by the route.
pub(super) async fn update_user(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<UserIdentifier>,
    Json(update): Json<UpdateAccount>,
) -> Result<Json<AccountDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_user(identifier, update)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Delete the account named by the route.
pub(super) async fn delete_user(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<UserIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_user(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Report search-index synchronization progress to authorized callers.
pub(super) async fn search_status(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<SearchSynchronization>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .search_status()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Rebuild the search index for authorized callers.
pub(super) async fn rebuild_search(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<SearchSynchronization>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .rebuild_search()
            .await
            .map_err(map_application_error)?,
    ))
}
