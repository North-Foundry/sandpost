//! Manual Web folder definitions, selected IMAP exposures, user mailbox placement, and Trash.

use crate::{
    ApplicationState,
    error::{HttpError, map_application_error},
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use sandpost_application::{MessageSummary, QueryOptions, SessionUser, WebFolderRequest};
use sandpost_core::{ImapFolderIdentifier, MessageIdentifier, MessageSequence, WebFolder};
use serde::{Deserialize, Serialize};

/// List only the authenticated user's manual folder definitions.
pub(super) async fn list_folders(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<WebFolder>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_web_folders()
            .await
            .map_err(map_application_error)?,
    ))
}

/// Create a Web folder with explicitly selected IMAP exposure.
pub(super) async fn create_folder(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<WebFolderRequest>,
) -> Result<(StatusCode, Json<WebFolder>), HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .authorized(&user)
                .create_web_folder(request)
                .await
                .map_err(map_application_error)?,
        ),
    ))
}

/// Rename a folder or replace its IMAP exposures without moving another account's overrides.
pub(super) async fn update_folder(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapFolderIdentifier>,
    Json(request): Json<WebFolderRequest>,
) -> Result<Json<WebFolder>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_web_folder(identifier, request)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Delete a folder and return its mail to baseline organization without deleting messages.
pub(super) async fn delete_folder(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapFolderIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_web_folder(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A bounded message page, optionally restricted with the canonical query language.
#[derive(Default, Deserialize)]
pub(super) struct FolderPage {
    before: Option<u64>,
    limit: Option<usize>,
    q: Option<String>,
}

/// List currently authorized, non-deleted messages explicitly placed in an owned Web folder.
pub(super) async fn folder_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<ImapFolderIdentifier>,
    Query(page): Query<FolderPage>,
) -> Result<Json<Vec<MessageSummary>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .query_web_mailbox(
                QueryOptions {
                    query: page.q,
                    before: page.before.map(MessageSequence),
                    limit: page.limit.unwrap_or(50),
                    ..QueryOptions::default()
                },
                Some(identifier),
            )
            .await
            .map_err(map_application_error)?,
    ))
}

/// A Web organization operation with a required destination; null restores baseline folders.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlacementRequest {
    identifiers: Vec<MessageIdentifier>,
    #[serde(deserialize_with = "Option::deserialize")]
    folder_identifier: Option<ImapFolderIdentifier>,
}

/// The number of authorized messages whose placement was processed.
#[derive(Serialize)]
pub(super) struct MovedMessages {
    moved: u64,
}

/// Apply Web placement independently of each IMAP account's organization.
pub(super) async fn move_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<PlacementRequest>,
) -> Result<Json<MovedMessages>, HttpError> {
    let moved = state
        .authorized(&user)
        .move_web_messages(request.identifiers, request.folder_identifier)
        .await
        .map_err(map_application_error)?;
    Ok(Json(MovedMessages { moved }))
}

/// A COPY destination, with null or omission selecting Inbox.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CopyRequest {
    identifiers: Vec<MessageIdentifier>,
    folder_identifier: Option<ImapFolderIdentifier>,
}

/// The number of new primary destination occurrences.
#[derive(Serialize)]
pub(super) struct CopiedMessages {
    copied: u64,
}

/// Copy into an owned normal folder or Inbox without changing the source occurrence.
pub(super) async fn copy_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<CopyRequest>,
) -> Result<Json<CopiedMessages>, HttpError> {
    let copied = state
        .authorized(&user)
        .copy_web_messages(request.identifiers, request.folder_identifier)
        .await
        .map_err(map_application_error)?;
    Ok(Json(CopiedMessages { copied }))
}

/// A bounded batch of messages to move to the caller's Web Trash.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TrashRequest {
    identifiers: Vec<MessageIdentifier>,
}

/// The number of authorized messages now in the caller's Web Trash.
#[derive(Serialize)]
pub(super) struct TrashedMessages {
    trashed: u64,
}

/// Move messages to the caller's Web Trash; moving them to a folder or Inbox restores them.
pub(super) async fn trash_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<TrashRequest>,
) -> Result<Json<TrashedMessages>, HttpError> {
    let trashed = state
        .authorized(&user)
        .trash_web_messages(request.identifiers)
        .await
        .map_err(map_application_error)?;
    Ok(Json(TrashedMessages { trashed }))
}

/// List currently authorized, non-deleted messages in the caller's Web Trash.
pub(super) async fn trash_messages_page(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Query(page): Query<FolderPage>,
) -> Result<Json<Vec<MessageSummary>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .query_web_trash(QueryOptions {
                query: page.q,
                before: page.before.map(MessageSequence),
                limit: page.limit.unwrap_or(50),
                ..QueryOptions::default()
            })
            .await
            .map_err(map_application_error)?,
    ))
}

/// The number of messages permanently deleted by emptying the Trash.
#[derive(Serialize)]
pub(super) struct EmptiedTrash {
    deleted: u64,
}

/// Permanently delete every message in the caller's Web Trash.
pub(super) async fn empty_trash(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<EmptiedTrash>, HttpError> {
    let deleted = state
        .authorized(&user)
        .empty_web_trash()
        .await
        .map_err(map_application_error)?;
    Ok(Json(EmptiedTrash { deleted }))
}
