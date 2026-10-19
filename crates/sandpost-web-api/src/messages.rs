//! Message listing, canonical search, detail, downloads, and deletion.

use crate::{
    ApplicationState,
    error::{HttpError, map_application_error},
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use sandpost_application::{
    CompatibilityReport, MessageDetail, MessageSummary, QueryOptions, SessionUser,
};
use sandpost_core::{InboxIdentifier, MessageIdentifier, MessageSequence, ViewIdentifier};
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
pub(super) struct Page {
    before: Option<u64>,
    limit: Option<usize>,
    inbox_id: Option<String>,
    view_id: Option<String>,
    q: Option<String>,
}

/// Return a page of stored messages with the requested cursor and limit.
pub(super) async fn list_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<MessageSummary>>, HttpError> {
    let limit = page.limit.unwrap_or(50);
    let inbox_identifier = page
        .inbox_id
        .as_deref()
        .map(parse_inbox_identifier)
        .transpose()?;
    let view_identifier = page
        .view_id
        .as_deref()
        .map(parse_view_identifier)
        .transpose()?;
    let inbox_view = inbox_identifier
        .filter(|identifier| !identifier.0.is_nil())
        .map(|identifier| ViewIdentifier(identifier.0));
    if view_identifier.is_some() && inbox_view.is_some() && view_identifier != inbox_view {
        return Err(HttpError(
            StatusCode::BAD_REQUEST,
            "view_id and inbox_id identify different views".into(),
        ));
    }
    let options = QueryOptions {
        view_identifier: view_identifier.or(inbox_view),
        query: page.q,
        before: page.before.map(MessageSequence),
        limit,
    };
    let authorized = state.authorized(&user);
    let rows = if options.view_identifier.is_none() && options.query.is_some() {
        authorized
            .query_messages(options)
            .await
            .map_err(map_application_error)?
    } else if options.view_identifier.is_some() {
        authorized
            .query_web_mailbox(options, None)
            .await
            .map_err(map_application_error)?
    } else {
        match inbox_identifier {
            Some(identifier) => authorized
                .list_messages_in_inbox(Some(identifier), options.before, options.limit)
                .await
                .map_err(map_application_error)?,
            None => authorized
                .list_messages(options.before, options.limit)
                .await
                .map_err(map_application_error)?,
        }
    };
    Ok(Json(rows))
}

#[derive(Default, Deserialize)]
pub(super) struct Search {
    q: String,
    before: Option<u64>,
    limit: Option<usize>,
    view_id: Option<String>,
}

/// Search with the canonical DSL, optionally restricted to a saved view.
pub(super) async fn search_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Query(search): Query<Search>,
) -> Result<Json<Vec<MessageSummary>>, HttpError> {
    let before = search.before.map(MessageSequence);
    let limit = search.limit.unwrap_or(50);
    let view_identifier = search
        .view_id
        .as_deref()
        .map(parse_view_identifier)
        .transpose()?;
    let rows = state
        .authorized(&user)
        .query_messages(QueryOptions {
            view_identifier,
            query: Some(search.q),
            before,
            limit,
        })
        .await
        .map_err(map_application_error)?;
    Ok(Json(rows))
}
/// Return parsed facts and attachment metadata for the requested message.
pub(super) async fn message(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<Json<MessageDetail>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .get_message(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Delete a stored message and return the required empty success response.
pub(super) async fn delete_message(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_message(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The number of messages removed by deleting all readable mail.
#[derive(Serialize)]
pub(super) struct DeletedMessages {
    deleted: u64,
}

/// Delete every message the caller can read and report how many were removed.
pub(super) async fn delete_all_messages(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<DeletedMessages>, HttpError> {
    let deleted = state
        .authorized(&user)
        .delete_all_messages()
        .await
        .map_err(map_application_error)?;
    Ok(Json(DeletedMessages { deleted }))
}

/// Return one attachment's bytes with a generic download disposition.
pub(super) async fn attachment(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path((identifier, index)): Path<(MessageIdentifier, usize)>,
) -> Result<Response, HttpError> {
    let bytes = state
        .authorized(&user)
        .get_attachment(identifier, index)
        .await
        .map_err(map_application_error)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        bytes,
    )
        .into_response())
}

/// Parse an inbox UUID at the HTTP trust boundary.
fn parse_inbox_identifier(value: &str) -> Result<InboxIdentifier, HttpError> {
    value
        .parse::<InboxIdentifier>()
        .map_err(|_| HttpError(StatusCode::BAD_REQUEST, "invalid inbox identifier".into()))
}

/// Parse a saved-view UUID at the HTTP trust boundary.
fn parse_view_identifier(value: &str) -> Result<ViewIdentifier, HttpError> {
    value
        .parse()
        .map_err(|_| HttpError(StatusCode::BAD_REQUEST, "invalid view identifier".into()))
}

/**
 * Return the email client compatibility report for the requested message's original HTML.
 */
pub(super) async fn html_check(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<Json<CompatibilityReport>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .check_message_html(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/// Return the original RFC 822 bytes for the requested message.
pub(super) async fn raw_message(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<Response, HttpError> {
    let raw_message = state
        .authorized(&user)
        .get_raw_message(identifier)
        .await
        .map_err(map_application_error)?;
    Ok((
        [
            (header::CONTENT_TYPE, "message/rfc822"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=message.eml",
            ),
        ],
        raw_message,
    )
        .into_response())
}
