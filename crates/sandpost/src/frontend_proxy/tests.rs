//! Network-level checks for the frontend proxy.

use super::*;
use axum::http::Request as HttpRequest;
use http_body_util::BodyExt;
use sandpost_storage_sqlite::SqliteStorage;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{PrivateKeyDer, pem::PemObject},
    },
};
use tower::ServiceExt;

/// Construct an API application without persistent test data.
async fn application() -> sandpost_application::Application {
    let storage = Arc::new(SqliteStorage::memory().await.unwrap());
    let (events, _) = tokio::sync::broadcast::channel(8);
    sandpost_application::Application::new(storage, events)
        .await
        .unwrap()
}

/// Read HTTP headers without consuming bytes that belong to a subsequent tunnel.
async fn read_headers<Stream: AsyncRead + Unpin>(stream: &mut Stream) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() < 16_384, "HTTP headers exceeded test limit");
    }
    bytes
}

/// Reject credentialed, malformed, or non-origin frontend URLs.
#[test]
fn validates_frontend_origins() {
    for value in [
        "http://user:pass@127.0.0.1:5173",
        "http://127.0.0.1:5173/path",
        "http://127.0.0.1:5173/?query=1",
        "http://127.0.0.1:5173/#fragment",
        "http://127.0.0.1:0",
        "http://127.0.0.1:65536",
        "http://127.0.0.1:abc",
        "ftp://frontend:8080",
        "http://[not-ipv6]:8080",
    ] {
        assert!(FrontendProxy::from_url(value).is_err(), "{value}");
    }
    for value in [
        "http://localhost:5173",
        "http://127.0.0.2:5173",
        "http://[::1]:5173/",
        "http://frontend:8080",
        "http://192.168.1.1:8080",
        "http://[2001:db8::1]:8080",
        "https://frontend:8443",
    ] {
        assert!(FrontendProxy::from_url(value).is_ok(), "{value}");
    }
}

/// Refuse self-proxying through literal, localhost, and wildcard listener addresses.
#[tokio::test]
async fn rejects_the_backend_listener_as_upstream() {
    let proxy = FrontendProxy::from_url("http://127.0.0.1:8025").unwrap();
    for address in [
        "127.0.0.1:8025",
        "0.0.0.0:8025",
        "[::]:8025",
        "[::ffff:127.0.0.1]:8025",
    ] {
        assert!(
            proxy.validate_listener(address.parse().unwrap()).is_err(),
            "{address}"
        );
    }
    assert!(
        proxy
            .validate_listener("127.0.0.1:5173".parse().unwrap())
            .is_ok()
    );
    assert!(
        proxy
            .validate_listener("127.0.0.1:0".parse().unwrap())
            .is_ok()
    );
    let proxy = FrontendProxy::from_url("http://[::1]:8025").unwrap();
    assert!(
        proxy
            .validate_listener("[::]:8025".parse().unwrap())
            .is_err()
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = FrontendProxy::from_url(&format!("http://{address}")).unwrap();
    assert!(proxy.validate_listener(address).is_err());
    let proxy = FrontendProxy::from_url("http://localhost:8025").unwrap();
    assert!(
        proxy
            .validate_listener("127.0.0.1:8025".parse().unwrap())
            .is_err()
    );
    assert!(
        proxy
            .validate_listener("[::]:8025".parse().unwrap())
            .is_err()
    );
    let proxy = FrontendProxy::from_url("http://192.0.2.5:8025").unwrap();
    assert!(
        proxy
            .validate_listener("192.0.2.5:8025".parse().unwrap())
            .is_err()
    );
    assert!(
        proxy
            .validate_listener("192.0.2.6:8025".parse().unwrap())
            .is_ok()
    );
}

/// Allow an upstream to prepare response headers beyond the connection timeout budget.
#[tokio::test]
async fn slow_response_headers_do_not_use_the_connection_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = FrontendProxy::from_url(&format!("http://{address}")).unwrap();
    let (_, shutdown) = watch::channel(false);
    let upstream = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_headers(&mut stream).await;
        tokio::time::sleep(Duration::from_secs(6)).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready")
            .await
            .unwrap();
    });
    let response = timeout(
        Duration::from_secs(10),
        proxy
            .router(
                application().await,
                shutdown,
                "127.0.0.1:8025".parse().unwrap(),
            )
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "ready"
    );
    upstream.await.unwrap();
}

/// Resolve a DNS frontend name and forward to its available address family.
#[tokio::test]
async fn forwards_to_localhost_dns() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = FrontendProxy::from_url(&format!("http://localhost:{}", address.port())).unwrap();
    let (_, shutdown) = watch::channel(false);
    let upstream = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let headers = String::from_utf8(read_headers(&mut stream).await).unwrap();
        assert!(headers.starts_with("GET /dns HTTP/1.1\r\n"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndns")
            .await
            .unwrap();
    });
    let response = proxy
        .router(
            application().await,
            shutdown,
            "127.0.0.1:8025".parse().unwrap(),
        )
        .oneshot(
            HttpRequest::builder()
                .uri("/dns")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "dns"
    );
    upstream.await.unwrap();
}

/// Check DNS answers against the actual listener before sending any request.
#[tokio::test]
async fn rejects_dns_self_proxy_at_request_time() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = FrontendProxy::from_url(&format!("http://localhost:{}", address.port())).unwrap();
    let (_, shutdown) = watch::channel(false);
    let response = proxy
        .router(application().await, shutdown, address)
        .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

/// Forward verified HTTPS and refuse a wrong hostname or untrusted certificate.
#[tokio::test]
async fn verifies_https_before_forwarding() {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let private_key =
        PrivateKeyDer::from_pem_slice(certified.signing_key.serialize_pem().as_bytes()).unwrap();
    let server_configuration =
        ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], private_key)
            .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_configuration));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let forwarded = Arc::new(AtomicUsize::new(0));
    let server_forwarded = Arc::clone(&forwarded);
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (stream, _) = listener.accept().await.unwrap();
            if let Ok(mut stream) = acceptor.accept(stream).await {
                let headers = String::from_utf8(read_headers(&mut stream).await).unwrap();
                assert!(headers.starts_with("GET /secure HTTP/1.1\r\n"));
                server_forwarded.fetch_add(1, Ordering::SeqCst);
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecure",
                    )
                    .await
                    .unwrap();
            }
        }
    });

    let mut roots = RootCertStore::empty();
    roots.add(certified.cert.der().clone()).unwrap();
    let connector = TlsConnector::from(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ));
    let mut trusted =
        FrontendProxy::from_url(&format!("https://localhost:{}", address.port())).unwrap();
    trusted.tls = Some(connector.clone());
    let (_, shutdown) = watch::channel(false);
    let request = || {
        HttpRequest::builder()
            .uri("/secure")
            .body(Body::empty())
            .unwrap()
    };
    let backend: SocketAddr = "127.0.0.1:8025".parse().unwrap();
    let response = trusted
        .router(application().await, shutdown.clone(), backend)
        .oneshot(request())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "secure"
    );

    let mut wrong_name =
        FrontendProxy::from_url(&format!("https://127.0.0.1:{}", address.port())).unwrap();
    wrong_name.tls = Some(connector);
    let response = wrong_name
        .router(application().await, shutdown.clone(), backend)
        .oneshot(request())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let mut untrusted =
        FrontendProxy::from_url(&format!("https://localhost:{}", address.port())).unwrap();
    untrusted.tls = Some(TlsConnector::from(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(RootCertStore::empty())
            .with_no_client_auth(),
    )));
    let response = untrusted
        .router(application().await, shutdown, backend)
        .oneshot(request())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(forwarded.load(Ordering::SeqCst), 1);
}

/// Preserve frontend method, path, query, status, type, and body while keeping API misses local.
#[tokio::test]
async fn forwards_http_and_keeps_api_local() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = FrontendProxy::from_url(&format!("http://{address}")).unwrap();
    let (_, shutdown) = watch::channel(false);
    let router = proxy.router(
        application().await,
        shutdown,
        "127.0.0.1:8025".parse().unwrap(),
    );

    let api_response = router
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(api_response.status(), StatusCode::NOT_FOUND);
    let health_response = router
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health_response.status(), StatusCode::OK);

    let upstream = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let headers = read_headers(&mut stream).await;
        let mut body = [0; 7];
        stream.read_exact(&mut body).await.unwrap();
        stream.write_all(b"HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nSet-Cookie: sandpost_session=unsafe\r\nConnection: close\r\n\r\nfrom-vite").await.unwrap();
        (String::from_utf8(headers).unwrap(), body)
    });
    let response = router
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("http://untrusted.invalid/nested/page?name=one%20two")
                .header(header::CONTENT_TYPE, "text/plain")
                .header(header::CONTENT_LENGTH, "7")
                .header(header::COOKIE, "sandpost_session=secret")
                .header(header::AUTHORIZATION, "Bearer secret")
                .body(Body::from("payload"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "text/plain");
    assert!(!response.headers().contains_key(header::SET_COOKIE));
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "from-vite"
    );
    let (headers, body) = upstream.await.unwrap();
    assert!(headers.starts_with("POST /nested/page?name=one%20two HTTP/1.1\r\n"));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains(&format!("host: {address}"))
    );
    assert!(!headers.to_ascii_lowercase().contains("cookie:"));
    assert!(!headers.to_ascii_lowercase().contains("authorization:"));
    assert_eq!(&body, b"payload");
}

/// Report a clear 502 when the configured frontend origin is absent.
#[tokio::test]
async fn unavailable_upstream_returns_bad_gateway() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let proxy = FrontendProxy::from_url(&format!("http://{address}")).unwrap();
    let (_, shutdown) = watch::channel(false);
    let response = proxy
        .router(
            application().await,
            shutdown,
            "127.0.0.1:8025".parse().unwrap(),
        )
        .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "Frontend origin unavailable"
    );
}

/// Pass a real 101 handshake and bytes in both directions, then close on shutdown.
#[tokio::test]
async fn tunnels_websocket_and_stops_on_shutdown() {
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let (mut stream, _) = upstream_listener.accept().await.unwrap();
        let headers = String::from_utf8(read_headers(&mut stream).await).unwrap();
        assert!(headers.starts_with("GET /hmr?token=abc HTTP/1.1\r\n"));
        assert!(headers.to_ascii_lowercase().contains("upgrade: websocket"));
        stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n").await.unwrap();
        let mut ping = [0; 4];
        stream.read_exact(&mut ping).await.unwrap();
        assert_eq!(&ping, b"ping");
        stream.write_all(b"pong").await.unwrap();
        let mut end = [0];
        assert_eq!(stream.read(&mut end).await.unwrap(), 0);
    });
    let proxy = FrontendProxy::from_url(&format!("http://{upstream_address}")).unwrap();
    let (shutdown_sender, shutdown) = watch::channel(false);
    let router = proxy.router(
        application().await,
        shutdown,
        "127.0.0.1:8025".parse().unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut client = TcpStream::connect(address).await.unwrap();
    client.write_all(b"GET /hmr?token=abc HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").await.unwrap();
    let headers = String::from_utf8(read_headers(&mut client).await).unwrap();
    assert!(
        headers.starts_with("HTTP/1.1 101 Switching Protocols\r\n"),
        "{headers}"
    );
    client.write_all(b"ping").await.unwrap();
    let mut pong = [0; 4];
    client.read_exact(&mut pong).await.unwrap();
    assert_eq!(&pong, b"pong");
    shutdown_sender.send(true).unwrap();
    let mut end = [0];
    assert_eq!(
        timeout(Duration::from_secs(2), client.read(&mut end))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    upstream.await.unwrap();
    server.abort();
}
