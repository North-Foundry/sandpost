//! Forward frontend requests to a configured HTTP or HTTPS origin.

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, Version, header},
    response::{IntoResponse, Response},
    routing::any,
};
use hyper::{body::Incoming, client::conn::http1};
use hyper_util::rt::TokioIo;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, copy_bidirectional},
    net::{TcpStream, lookup_host},
    sync::watch,
    time::timeout,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, crypto::ring, pki_types::ServerName},
};

const UPSTREAM_WAIT: Duration = Duration::from_secs(30);
const CONNECTION_ATTEMPT_WAIT: Duration = Duration::from_secs(2);
const UPSTREAM_HANDSHAKE_WAIT: Duration = Duration::from_secs(5);

/// A validated frontend origin whose addresses are resolved for each request.
#[derive(Clone)]
pub(crate) struct FrontendProxy {
    hostname: String,
    port: u16,
    host: HeaderValue,
    tls: Option<TlsConnector>,
}

impl FrontendProxy {
    /// Accept an HTTP or HTTPS origin without credentials, path, query, or fragment.
    pub(crate) fn from_url(value: &str) -> Result<Self, crate::RuntimeError> {
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frontend URL must be an HTTP or HTTPS origin with a valid port",
            )
        };
        let uri: Uri = value.parse().map_err(|_| invalid())?;
        let secure = match uri.scheme_str() {
            Some("http") => false,
            Some("https") => true,
            _ => return Err(invalid().into()),
        };
        if value.contains('#') {
            return Err(invalid().into());
        }
        let authority = uri.authority().ok_or_else(invalid)?;
        if authority.as_str().contains('@')
            || !matches!(
                uri.path_and_query().map(|path| path.as_str()),
                None | Some("/")
            )
        {
            return Err(invalid().into());
        }
        let hostname = authority.host();
        if hostname.is_empty() {
            return Err(invalid().into());
        }
        let hostname = if hostname.starts_with('[') {
            let literal = hostname
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .ok_or_else(invalid)?;
            literal.parse::<IpAddr>().map_err(|_| invalid())?;
            literal
        } else {
            hostname
        };
        let suffix = authority
            .as_str()
            .strip_prefix(authority.host())
            .ok_or_else(invalid)?;
        let port = if suffix.is_empty() {
            if secure { 443 } else { 80 }
        } else {
            let digits = suffix.strip_prefix(':').ok_or_else(invalid)?;
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid().into());
            }
            digits.parse::<u16>().map_err(|_| invalid())?
        };
        if port == 0 {
            return Err(invalid().into());
        }
        let tls = if secure {
            let mut roots = RootCertStore::empty();
            let (accepted, _) =
                roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
            if accepted == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "no trusted certificates are available for the HTTPS frontend origin",
                )
                .into());
            }
            let configuration =
                ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
                    .with_safe_default_protocol_versions()
                    .map_err(std::io::Error::other)?
                    .with_root_certificates(roots)
                    .with_no_client_auth();
            Some(TlsConnector::from(Arc::new(configuration)))
        } else {
            None
        };
        Ok(Self {
            hostname: hostname.to_owned(),
            port,
            host: HeaderValue::from_str(authority.as_str()).map_err(|_| invalid())?,
            tls,
        })
    }

    /// Reject an origin that would forward frontend requests back into this HTTP listener.
    pub(crate) fn validate_listener(
        &self,
        listener: SocketAddr,
    ) -> Result<(), crate::RuntimeError> {
        if self.port == listener.port() {
            let literal = self.hostname.parse::<IpAddr>().ok();
            let matches_listener = literal
                .is_some_and(|address| is_listener_address(address, listener))
                || (self.hostname.eq_ignore_ascii_case("localhost")
                    && (listener.ip().to_canonical().is_loopback()
                        || listener.ip().to_canonical().is_unspecified()));
            if matches_listener {
                return Err(self_proxy_error().into());
            }
        }
        Ok(())
    }

    /// Keep every API route local and forward unmatched routes to the configured origin.
    pub(crate) fn router(
        self,
        application: sandpost_application::Application,
        shutdown: watch::Receiver<bool>,
        listener: SocketAddr,
    ) -> Router {
        sandpost_web_api::router(application).fallback(any(move |request| {
            let proxy = self.clone();
            let shutdown = shutdown.clone();
            async move { proxy.forward(request, shutdown, listener).await }
        }))
    }

    /// Send one request upstream, returning the response body as a stream.
    async fn forward(
        &self,
        mut request: Request,
        shutdown: watch::Receiver<bool>,
        listener: SocketAddr,
    ) -> Response {
        if request.uri().path() == "/api" || request.uri().path().starts_with("/api/") {
            return StatusCode::NOT_FOUND.into_response();
        }
        let websocket = request.method() == Method::GET && is_websocket(request.headers());
        let downstream_upgrade = websocket.then(|| hyper::upgrade::on(&mut request));
        let (mut parts, body) = request.into_parts();
        clean_headers(&mut parts.headers, websocket, true);
        parts.headers.insert(header::HOST, self.host.clone());
        parts.version = Version::HTTP_11;
        parts.uri = parts
            .uri
            .path_and_query()
            .map_or("/", |path| path.as_str())
            .parse()
            .expect("a valid request path and query form a valid URI");
        let upstream_request = Request::from_parts(parts, body);

        let connection_shutdown = shutdown.clone();
        let upstream = timeout(UPSTREAM_WAIT, async {
            let addresses: Vec<_> = timeout(
                UPSTREAM_HANDSHAKE_WAIT,
                lookup_host((self.hostname.as_str(), self.port)),
            )
            .await
            .map_err(std::io::Error::other)??
            .collect();
            if addresses.iter().any(|address| {
                address.port() == listener.port() && is_listener_address(address.ip(), listener)
            }) {
                return Err(self_proxy_error());
            }
            let mut last_error = std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "frontend origin has no addresses",
            );
            for address in addresses {
                match timeout(CONNECTION_ATTEMPT_WAIT, TcpStream::connect(address)).await {
                    Ok(Ok(stream)) => {
                        // For wildcard listeners, a local interface alias is visible as the
                        // outgoing connection's source address before any request is sent.
                        if listener.port() == address.port()
                            && listener.ip().to_canonical().is_unspecified()
                            && stream.local_addr()?.ip().to_canonical()
                                == address.ip().to_canonical()
                        {
                            return Err(self_proxy_error());
                        }
                        return if let Some(connector) = &self.tls {
                            let server_name = ServerName::try_from(self.hostname.clone())
                                .map_err(std::io::Error::other)?;
                            let stream = timeout(
                                UPSTREAM_HANDSHAKE_WAIT,
                                connector.connect(server_name, stream),
                            )
                            .await
                            .map_err(std::io::Error::other)??;
                            send_request(stream, upstream_request, connection_shutdown).await
                        } else {
                            send_request(stream, upstream_request, connection_shutdown).await
                        };
                    }
                    Ok(Err(error)) => last_error = error,
                    Err(error) => {
                        last_error = std::io::Error::new(std::io::ErrorKind::TimedOut, error)
                    }
                }
            }
            Err(last_error)
        })
        .await;
        let mut upstream_response = match upstream {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::warn!(%error, upstream = %self.host.to_str().unwrap_or("invalid"), "frontend origin unavailable");
                return bad_gateway();
            }
            Err(_) => return bad_gateway(),
        };

        let accepted_upgrade =
            websocket && upstream_response.status() == StatusCode::SWITCHING_PROTOCOLS;
        let upstream_upgrade = accepted_upgrade.then(|| hyper::upgrade::on(&mut upstream_response));
        let (mut parts, body) = upstream_response.into_parts();
        clean_headers(&mut parts.headers, accepted_upgrade, false);
        if let (Some(downstream), Some(upstream)) = (downstream_upgrade, upstream_upgrade) {
            tokio::spawn(async move {
                tokio::select! {
                    _ = wait_for_shutdown(shutdown) => {},
                    result = async {
                        let (downstream, upstream) = tokio::try_join!(downstream, upstream)
                            .map_err(std::io::Error::other)?;
                        copy_bidirectional(&mut TokioIo::new(downstream), &mut TokioIo::new(upstream)).await
                            .map_err(std::io::Error::other)
                    } => {
                        if let Err(error) = result {
                            tracing::debug!(%error, "frontend WebSocket tunnel closed");
                        }
                    }
                }
            });
            Response::from_parts(parts, Body::empty())
        } else {
            Response::from_parts(parts, Body::new(body))
        }
    }
}

/// Match literal and resolved targets against the bound HTTP address.
fn is_listener_address(target: IpAddr, listener: SocketAddr) -> bool {
    let target = target.to_canonical();
    let bound = listener.ip().to_canonical();
    if target.is_unspecified() || target == bound {
        return true;
    }
    bound.is_unspecified() && target.is_loopback()
}

/// Describe a frontend origin that would enter the HTTP listener again.
fn self_proxy_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "frontend URL must not target the Sandpost HTTP listener",
    )
}

/// Send an HTTP request over a connected plain or verified TLS stream.
async fn send_request<Stream>(
    stream: Stream,
    request: Request,
    shutdown: watch::Receiver<bool>,
) -> std::io::Result<hyper::Response<Incoming>>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream))
        .await
        .map_err(std::io::Error::other)?;
    tokio::spawn(async move {
        tokio::select! {
            result = connection.with_upgrades() => {
                if let Err(error) = result {
                    tracing::debug!(%error, "frontend connection closed");
                }
            }
            _ = wait_for_shutdown(shutdown) => {}
        }
    });
    sender
        .send_request(request)
        .await
        .map_err(std::io::Error::other)
}

/// Identify an ordinary HTTP/1 WebSocket upgrade request.
fn is_websocket(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
        && headers.get_all(header::CONNECTION).iter().any(|value| {
            value
                .as_bytes()
                .split(|byte| *byte == b',')
                .any(|token| token.trim_ascii().eq_ignore_ascii_case(b"upgrade"))
        })
}

/// Remove hop-by-hop headers and credentials while retaining WebSocket upgrade fields.
fn clean_headers(headers: &mut HeaderMap, websocket: bool, request: bool) {
    let nominated: Vec<Vec<u8>> = headers
        .get_all(header::CONNECTION)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .map(|token| token.trim_ascii().to_ascii_lowercase())
        .collect();
    let original = std::mem::take(headers);
    for (name, value) in original.iter() {
        let header_name = name.as_str();
        let upgrade_header = websocket && (header_name == "connection" || header_name == "upgrade");
        if !upgrade_header
            && nominated
                .iter()
                .any(|token| token == header_name.as_bytes())
        {
            continue;
        }
        if !upgrade_header
            && matches!(
                header_name,
                "connection"
                    | "upgrade"
                    | "keep-alive"
                    | "proxy-connection"
                    | "te"
                    | "trailer"
                    | "transfer-encoding"
                    | "proxy-authenticate"
                    | "proxy-authorization"
            )
        {
            continue;
        }
        if request && matches!(header_name, "cookie" | "authorization") {
            continue;
        }
        if !request && header_name == "set-cookie" {
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    if websocket {
        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    }
}

/// Return a clear proxy failure without disclosing upstream connection details.
fn bad_gateway() -> Response {
    (StatusCode::BAD_GATEWAY, "Frontend origin unavailable").into_response()
}

/// Wait until runtime shutdown is announced, including an already-stopped runtime.
async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests;
