//! Versioned API routing, health checks, and optional static SPA hosting.

use crate::{
    ApplicationState, authentication,
    error::{HttpError, map_application_error},
    events, imap, inboxes, management, messages, smtp,
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware,
    routing::{any, get},
};
use serde::Serialize;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

/// Build the HTTP router for the catcher API.
pub fn router(state: ApplicationState) -> Router {
    api_router()
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state,
            authentication::authenticate,
        ))
}

/// Build the API router with a static frontend and single-page-app fallback.
pub fn router_with_frontend(state: ApplicationState, directory: PathBuf) -> Router {
    api_router()
        .fallback_service(
            ServeDir::new(&directory).fallback(ServeFile::new(directory.join("index.html"))),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state,
            authentication::authenticate,
        ))
}

/// Build versioned API routes, including a non-HTML fallback for unknown API paths.
fn api_router() -> Router<ApplicationState> {
    Router::new()
        .route("/api/v1/instance", get(authentication::instance_status))
        .route("/api/v1/setup", axum::routing::post(authentication::setup))
        .route(
            "/api/v1/setup/authorize",
            axum::routing::post(authentication::authorize_setup),
        )
        .route("/api/v1/login", axum::routing::post(authentication::login))
        .route("/api/v1/session", get(authentication::current_session))
        .route(
            "/api/v1/logout",
            axum::routing::post(authentication::logout),
        )
        .route("/api/v1/health", get(health))
        .route(
            "/api/v1/inboxes",
            get(inboxes::list_inboxes).post(inboxes::create_inbox),
        )
        .route(
            "/api/v1/messages",
            get(messages::list_messages).delete(messages::delete_all_messages),
        )
        .route("/api/v1/search", get(messages::search_messages))
        .route("/api/v1/search/status", get(management::search_status))
        .route(
            "/api/v1/search/rebuild",
            axum::routing::post(management::rebuild_search),
        )
        .route(
            "/api/v1/smtp/server",
            get(smtp::smtp_server).put(smtp::save_smtp_server),
        )
        .route(
            "/api/v1/smtp/accesses",
            get(smtp::list_smtp_accesses).post(smtp::create_smtp_access),
        )
        .route(
            "/api/v1/smtp/accesses/{identifier}",
            axum::routing::put(smtp::update_smtp_access).delete(smtp::delete_smtp_access),
        )
        .route(
            "/api/v1/smtp/accesses/{identifier}/password",
            axum::routing::post(smtp::regenerate_smtp_access_password),
        )
        .route("/api/v1/imap/server", get(imap::imap_server))
        .route(
            "/api/v1/imap/accounts",
            get(imap::list_imap_accounts).post(imap::create_imap_account),
        )
        .route(
            "/api/v1/imap/accounts/{identifier}",
            get(imap::imap_account)
                .put(imap::update_imap_account)
                .delete(imap::delete_imap_account),
        )
        .route(
            "/api/v1/imap/accounts/{identifier}/password",
            axum::routing::post(imap::regenerate_imap_account_password),
        )
        .route(
            "/api/v1/imap/accounts/{identifier}/restore",
            axum::routing::post(imap::restore_imap_hidden_messages),
        )
        .route(
            "/api/v1/imap/accounts/{identifier}/folders",
            axum::routing::post(imap::create_imap_folder),
        )
        .route(
            "/api/v1/imap/accounts/{identifier}/folders/{folder}",
            axum::routing::put(imap::update_imap_folder).delete(imap::delete_imap_folder),
        )
        .route(
            "/api/v1/query/validate",
            axum::routing::post(management::validate_query),
        )
        .route(
            "/api/v1/scopes",
            get(management::list_scopes).post(management::save_scope),
        )
        .route(
            "/api/v1/scopes/preview",
            axum::routing::post(management::preview_scope),
        )
        .route(
            "/api/v1/scopes/{identifier}",
            axum::routing::put(management::save_scope_at).delete(management::delete_scope),
        )
        .route(
            "/api/v1/scopes/{identifier}/assignments",
            get(management::list_scope_assignments),
        )
        .route(
            "/api/v1/scopes/{identifier}/assignments/{user}",
            axum::routing::put(management::assign_scope)
                .delete(management::remove_scope_assignment),
        )
        .route(
            "/api/v1/views",
            get(management::list_views).post(management::save_view),
        )
        .route(
            "/api/v1/views/{identifier}",
            axum::routing::put(management::save_view_at).delete(management::delete_view),
        )
        .route(
            "/api/v1/users",
            get(management::list_users).post(management::create_user),
        )
        .route(
            "/api/v1/users/{identifier}",
            axum::routing::put(management::update_user).delete(management::delete_user),
        )
        .route(
            "/api/v1/messages/{identifier}",
            get(messages::message).delete(messages::delete_message),
        )
        .route(
            "/api/v1/messages/{identifier}/attachments/{index}",
            get(messages::attachment),
        )
        .route(
            "/api/v1/messages/{identifier}/raw",
            get(messages::raw_message),
        )
        .route("/api/v1/events", get(events::events))
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found))
}

/// Return an empty not-found response for API paths without a registered route.
async fn api_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// Report whether the backing storage can answer a health query.
async fn health(State(state): State<ApplicationState>) -> Result<Json<Health>, HttpError> {
    state.health().await.map_err(map_application_error)?;
    Ok(Json(Health {
        status: "ok",
        mode: "local-development",
    }))
}
#[derive(Serialize)]
struct Health {
    status: &'static str,
    mode: &'static str,
}
