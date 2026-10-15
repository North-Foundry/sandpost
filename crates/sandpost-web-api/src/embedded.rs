//! The frontend embedded at compile time, served when no frontend directory is configured.

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};

include!(concat!(env!("OUT_DIR"), "/frontend_assets.rs"));

/// Asset served for the application entry point and for unknown application routes.
const ENTRY_POINT: &str = "index.html";

/// Whether this build embedded the built frontend.
pub const EMBEDDED_FRONTEND: bool = !FRONTEND_ASSETS.is_empty();

/// Serve an embedded asset, falling back to the entry point for unknown application routes.
pub(crate) async fn serve_asset(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        ENTRY_POINT
    } else {
        requested
    };
    let (path, bytes) = match asset(requested) {
        Some(bytes) => (requested, bytes),
        None => match asset(ENTRY_POINT) {
            Some(bytes) => (ENTRY_POINT, bytes),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    ([(header::CONTENT_TYPE, content_type(path))], bytes).into_response()
}

/// Find an embedded asset by its frontend-relative path.
fn asset(path: &str) -> Option<&'static [u8]> {
    FRONTEND_ASSETS
        .iter()
        .find_map(|(name, bytes)| (*name == path).then_some(*bytes))
}

/// Content type for an embedded asset, covering the extensions the frontend build emits.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
