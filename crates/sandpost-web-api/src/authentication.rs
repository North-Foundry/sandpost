//! Session authentication, origin checks, and instance setup endpoints.

use crate::{
    ApplicationState,
    error::{HttpError, auth_error, map_instance_error},
};
use axum::{
    Json,
    extract::{Extension, State},
    http::{HeaderMap, Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sandpost_application::{LoginRequest, SessionUser, SetupRequest};
use serde::{Deserialize, Serialize};

/// Require a valid application session for every route except bootstrap and login.
pub(super) async fn authenticate(
    State(state): State<ApplicationState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let secure = request.uri().scheme_str() == Some("https");
    request.extensions_mut().insert(secure);
    let path = request.uri().path();
    let method = request.method();
    if (path == "/api/v1/instance" && *method == Method::GET)
        || (path == "/api/v1/setup" && *method == Method::POST)
        || (path == "/api/v1/setup/authorize" && *method == Method::POST)
        || (path == "/api/v1/login" && *method == Method::POST)
        || path == "/api/v1/health"
        || !is_protected_api_route(path)
    {
        return next.run(request).await;
    }

    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let cookie = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(session_cookie);
    let Some(token) = bearer.or(cookie).map(str::to_owned) else {
        return auth_error(StatusCode::UNAUTHORIZED, "authentication_required");
    };
    let Some(user) = state.instance().session(&token).await.ok() else {
        return auth_error(StatusCode::UNAUTHORIZED, "authentication_required");
    };
    if bearer.is_none()
        && !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        && !same_origin(request.headers(), secure)
    {
        return auth_error(StatusCode::FORBIDDEN, "same_origin_required");
    }
    request.extensions_mut().insert(user);
    request.extensions_mut().insert(SessionToken(token));
    next.run(request).await
}

#[derive(Clone)]
pub(super) struct SessionToken(pub(super) String);

/// Match only registered instance routes so static assets and API misses stay public.
fn is_protected_api_route(path: &str) -> bool {
    match path {
        "/api/v1/session"
        | "/api/v1/logout"
        | "/api/v1/inboxes"
        | "/api/v1/messages"
        | "/api/v1/messages/placement"
        | "/api/v1/folders"
        | "/api/v1/search"
        | "/api/v1/search/status"
        | "/api/v1/search/rebuild"
        | "/api/v1/scopes"
        | "/api/v1/views"
        | "/api/v1/users"
        | "/api/v1/query/validate"
        | "/api/v1/smtp/server"
        | "/api/v1/smtp/accesses"
        | "/api/v1/imap/server"
        | "/api/v1/imap/accounts"
        | "/api/v1/events" => true,
        _ => {
            if path.starts_with("/api/v1/scopes/")
                || path.starts_with("/api/v1/views/")
                || path.starts_with("/api/v1/users/")
                || path.starts_with("/api/v1/smtp/accesses/")
                || path.starts_with("/api/v1/imap/accounts/")
                || path.starts_with("/api/v1/folders/")
            {
                return true;
            }
            let Some(path) = path.strip_prefix("/api/v1/messages/") else {
                return false;
            };
            let segments: Vec<_> = path.split('/').collect();
            segments.len() == 1
                || (segments.len() == 2 && segments[1] == "raw")
                || (segments.len() == 3 && segments[1] == "attachments")
        }
    }
}

/// Extract the HTTP-only session cookie without accepting query-string credentials.
fn session_cookie(value: &str) -> Option<&str> {
    value.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "sandpost_session").then_some(value)
    })
}

/// Require cookie-authenticated mutations to match the direct request origin.
fn same_origin(headers: &HeaderMap, secure: bool) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let scheme = if secure { "https" } else { "http" };
    origin == format!("{scheme}://{host}")
}

/// Report setup completion without exposing the bootstrap secret or other metadata.
pub(super) async fn instance_status(
    State(state): State<ApplicationState>,
) -> Result<Json<Initialized>, HttpError> {
    let status = state.instance().status().map_err(map_instance_error)?;
    Ok(Json(Initialized {
        initialized: status.initialized,
        setup_key_required: !status.initialized,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Initialized {
    initialized: bool,
    setup_key_required: bool,
}

/// Key verification input, excluding owner details and Debug to avoid credential logging.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetupAuthorizationRequest {
    setup_key: String,
}

/// Temporary credential for completing setup through the legacy bootstrapSecret field.
#[derive(Serialize)]
struct SetupAuthorizationResponse {
    authorization: String,
}

/// Verify the boot key without creating an Owner and return a temporary setup authorization.
pub(super) async fn authorize_setup(
    State(state): State<ApplicationState>,
    Json(request): Json<SetupAuthorizationRequest>,
) -> Result<Response, HttpError> {
    let authorization = state
        .instance()
        .authorize_setup(&request.setup_key)
        .map_err(map_instance_error)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(SetupAuthorizationResponse { authorization }),
    )
        .into_response())
}

/// Complete first-run setup and create the initial owner account.
pub(super) async fn setup(
    State(state): State<ApplicationState>,
    Json(request): Json<SetupRequest>,
) -> Result<Response, HttpError> {
    let status = state
        .instance()
        .setup(request)
        .await
        .map_err(map_instance_error)?;
    Ok((StatusCode::CREATED, Json(status)).into_response())
}

/// Authenticate an owner and issue an HTTP-only session cookie.
pub(super) async fn login(
    State(state): State<ApplicationState>,
    Extension(secure): Extension<bool>,
    Json(credentials): Json<LoginRequest>,
) -> Result<Response, HttpError> {
    let response = state
        .instance()
        .login(credentials)
        .await
        .map_err(map_instance_error)?;
    let cookie = format!(
        "sandpost_session={}; HttpOnly; SameSite=Strict; Path=/api/v1; Max-Age=86400{}",
        response.token,
        if secure { "; Secure" } else { "" }
    );
    let mut result = Json(response).into_response();
    result.headers_mut().insert(
        header::SET_COOKIE,
        cookie.parse().map_err(|_| {
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "instance_unavailable".into(),
            )
        })?,
    );
    Ok(result)
}

/// Return the authenticated owner represented by the request session.
pub(super) async fn current_session(Extension(user): Extension<SessionUser>) -> Json<SessionUser> {
    Json(user)
}

/// Revoke the current session and clear its browser cookie.
pub(super) async fn logout(
    State(state): State<ApplicationState>,
    Extension(secure): Extension<bool>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| {
            headers
                .get(header::COOKIE)
                .and_then(|value| value.to_str().ok())
                .and_then(session_cookie)
        })
        .ok_or_else(|| HttpError(StatusCode::UNAUTHORIZED, "authentication_required".into()))?;
    state.instance().logout(token).map_err(map_instance_error)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    let cookie = format!(
        "sandpost_session=; HttpOnly; SameSite=Strict; Path=/api/v1; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie.parse().map_err(|_| {
            HttpError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "instance_unavailable".into(),
            )
        })?,
    );
    Ok(response)
}
