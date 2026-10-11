//! Management of the caller's own IMAP accounts and their mailbox configuration.
//!
//! Every route acts on the authenticated caller's accounts only; the application service
//! enforces ownership and validates filters. Creation and password regeneration are the only
//! responses that carry a password, and they are marked `no-store`.
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
    ImapAccountDetails, ImapAccountUpdate, ImapFolderRequest, ImapServerDetails,
    IssuedImapCredentials, NewImapAccountRequest, SessionUser,
};
use sandpost_core::{ImapAccountIdentifier, ImapFolderIdentifier, ImapLocalFolder};
use serde::Serialize;

/// Return how clients connect to the IMAP service.
pub(super) async fn imap_server(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<ImapServerDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .imap_server()
            .await
            .map_err(map_application_error)?,
    ))
}

/// List the caller's IMAP accounts.
pub(super) async fn list_imap_accounts(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<ImapAccountDetails>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_imap_accounts()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Create an IMAP account and return its generated password exactly once.
pub(super) async fn create_imap_account(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<NewImapAccountRequest>,
) -> Result<Response, HttpError> {
    let issued = state
        .authorized(&user)
        .create_imap_account(request)
        .await
        .map_err(map_application_error)?;
    Ok(secret_response(StatusCode::CREATED, issued))
}

/// Return one of the caller's IMAP accounts.
pub(super) async fn imap_account(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapAccountIdentifier>,
) -> Result<Json<ImapAccountDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .get_imap_account(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Change an IMAP account's name, status, or mailbox configuration.
pub(super) async fn update_imap_account(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapAccountIdentifier>,
    Json(update): Json<ImapAccountUpdate>,
) -> Result<Json<ImapAccountDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_imap_account(identifier, update)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Delete an IMAP account; captured mail is untouched.
pub(super) async fn delete_imap_account(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapAccountIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_imap_account(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replace an IMAP account's password and return the new one exactly once.
pub(super) async fn regenerate_imap_account_password(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapAccountIdentifier>,
) -> Result<Response, HttpError> {
    let issued = state
        .authorized(&user)
        .regenerate_imap_account_password(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(secret_response(StatusCode::OK, issued))
}

/// The number of messages made visible again by restoring hidden messages.
#[derive(Serialize)]
pub(super) struct RestoredMessages {
    restored: u64,
}

/// Let messages expunged in IMAP clients reappear in the account's mailboxes.
pub(super) async fn restore_imap_hidden_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapAccountIdentifier>,
) -> Result<Json<RestoredMessages>, HttpError> {
    let restored = state
        .authorized(&user)
        .restore_imap_hidden_messages(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(Json(RestoredMessages { restored }))
}

/// Create an IMAP-local folder.
pub(super) async fn create_imap_folder(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapAccountIdentifier>,
    Json(request): Json<ImapFolderRequest>,
) -> Result<(StatusCode, Json<ImapLocalFolder>), HttpError> {
    let folder = state
        .authorized(&user)
        .create_imap_folder(identifier, request)
        .await
        .map_err(map_application_error)?;
    Ok((StatusCode::CREATED, Json(folder)))
}

/// Replace an IMAP-local folder's name, filter, and INBOX participation.
pub(super) async fn update_imap_folder(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path((identifier, folder)): Path<(ImapAccountIdentifier, ImapFolderIdentifier)>,
    Json(request): Json<ImapFolderRequest>,
) -> Result<Json<ImapLocalFolder>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_imap_folder(identifier, folder, request)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Delete an IMAP-local folder.
pub(super) async fn delete_imap_folder(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path((identifier, folder)): Path<(ImapAccountIdentifier, ImapFolderIdentifier)>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_imap_folder(identifier, folder)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Serialize issued credentials with headers that forbid caching the password.
fn secret_response(status: StatusCode, issued: IssuedImapCredentials) -> Response {
    let mut response = (status, Json(issued)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}
