//! SMTP Outbounds, scope routing actions and outbound delivery status.
//!
//! Only Owners may use these routes; the application service enforces it. No response ever
//! carries an SMTP Outbound password.
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
    ConnectionTestResult, OutboundDeliveryDetails, OutboundDeliveryListRequest, OutboundPolicy,
    RoutingActionRequest, SessionUser, SmtpOutboundDetails, SmtpOutboundEnabledRequest,
    SmtpOutboundRequest, SmtpOutboundTestRequest,
};
use sandpost_core::{
    MessageIdentifier, OutboundDelivery, OutboundDeliveryIdentifier, RoutingAction,
    RoutingActionIdentifier, ScopeIdentifier, SmtpOutboundIdentifier,
};

/**
 * Describe the instance's outbound delivery policy.
 */
pub(super) async fn outbound_policy(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<OutboundPolicy>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .outbound_policy()
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * List SMTP Outbounds without passwords.
 */
pub(super) async fn list_smtp_outbounds(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
) -> Result<Json<Vec<SmtpOutboundDetails>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_smtp_outbounds()
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Create an SMTP Outbound.
 */
pub(super) async fn create_smtp_outbound(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<SmtpOutboundRequest>,
) -> Result<(StatusCode, Json<SmtpOutboundDetails>), HttpError> {
    let outbound = state
        .authorized(&user)
        .create_smtp_outbound(request)
        .await
        .map_err(map_application_error)?;
    Ok((StatusCode::CREATED, Json(outbound)))
}

/**
 * Read one SMTP Outbound without its password.
 */
pub(super) async fn smtp_outbound(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpOutboundIdentifier>,
) -> Result<Json<SmtpOutboundDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .get_smtp_outbound(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Replace an SMTP Outbound; an omitted password keeps the stored one.
 */
pub(super) async fn update_smtp_outbound(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpOutboundIdentifier>,
    Json(request): Json<SmtpOutboundRequest>,
) -> Result<Json<SmtpOutboundDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_smtp_outbound(identifier, request)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Enable or disable an SMTP Outbound.
 */
pub(super) async fn set_smtp_outbound_enabled(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpOutboundIdentifier>,
    Json(request): Json<SmtpOutboundEnabledRequest>,
) -> Result<Json<SmtpOutboundDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .set_smtp_outbound_enabled(identifier, request.enabled)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Delete an SMTP Outbound that no routing action references.
 */
pub(super) async fn delete_smtp_outbound(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpOutboundIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_smtp_outbound(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/**
 * Test the connection of a saved SMTP Outbound without sending mail.
 */
pub(super) async fn test_smtp_outbound(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<SmtpOutboundIdentifier>,
) -> Result<Json<ConnectionTestResult>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .test_smtp_outbound(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Test an unsaved SMTP Outbound definition without sending mail.
 */
pub(super) async fn test_smtp_outbound_draft(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Json(request): Json<SmtpOutboundTestRequest>,
) -> Result<Json<ConnectionTestResult>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .test_smtp_outbound_draft(request)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * List a scope's routing actions.
 */
pub(super) async fn list_routing_actions(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(scope): Path<ScopeIdentifier>,
) -> Result<Json<Vec<RoutingAction>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_routing_actions(scope)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Attach a routing action to a scope.
 */
pub(super) async fn create_routing_action(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(scope): Path<ScopeIdentifier>,
    Json(request): Json<RoutingActionRequest>,
) -> Result<(StatusCode, Json<RoutingAction>), HttpError> {
    let action = state
        .authorized(&user)
        .create_routing_action(scope, request)
        .await
        .map_err(map_application_error)?;
    Ok((StatusCode::CREATED, Json(action)))
}

/**
 * Replace a routing action.
 */
pub(super) async fn update_routing_action(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<RoutingActionIdentifier>,
    Json(request): Json<RoutingActionRequest>,
) -> Result<Json<RoutingAction>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .update_routing_action(identifier, request)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Delete a routing action.
 */
pub(super) async fn delete_routing_action(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<RoutingActionIdentifier>,
) -> Result<StatusCode, HttpError> {
    state
        .authorized(&user)
        .delete_routing_action(identifier)
        .await
        .map_err(map_application_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/**
 * List deliveries newest first, bounded by the caller's mail visibility.
 */
pub(super) async fn list_outbound_deliveries(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Query(request): Query<OutboundDeliveryListRequest>,
) -> Result<Json<Vec<OutboundDelivery>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_outbound_deliveries(request)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Read one delivery with its attempt history.
 */
pub(super) async fn outbound_delivery(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<OutboundDeliveryIdentifier>,
) -> Result<Json<OutboundDeliveryDetails>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .get_outbound_delivery(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * Schedule a failed delivery again.
 */
pub(super) async fn retry_outbound_delivery(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<OutboundDeliveryIdentifier>,
) -> Result<Json<OutboundDelivery>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .retry_outbound_delivery(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}

/**
 * List the deliveries of one readable message.
 */
pub(super) async fn message_deliveries(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Path(identifier): Path<MessageIdentifier>,
) -> Result<Json<Vec<OutboundDelivery>>, HttpError> {
    Ok(Json(
        state
            .authorized(&user)
            .list_message_deliveries(identifier)
            .await
            .map_err(map_application_error)?,
    ))
}
