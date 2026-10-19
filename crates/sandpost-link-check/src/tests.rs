//! Extraction, destination policy, classification and live HTTP verification tests.
use crate::client::{AddressFilter, Resolve};
use crate::{
    LinkCheckSettings, LinkChecker, MAXIMUM_LINKS, classify_status, extract_links,
    is_public_address, normalize_link, status_reason,
};
use async_trait::async_trait;
use sandpost_core::{LinkCheckErrorKind, LinkOutcome};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{
    ServerConfig,
    crypto::ring,
    pki_types::{PrivateKeyDer, pem::PemObject},
};

/// How a test server answers one request.
#[derive(Clone)]
enum Reply {
    /// Answer with a status code and extra headers.
    Status(u16, Vec<(&'static str, String)>),
    /// Answer after a delay, to observe concurrency.
    Delayed(Duration, u16),
    /// Accept the request and never answer.
    Hang,
}

/// A loopback HTTP/1.1 server recording its connections and requests.
struct TestServer {
    address: SocketAddr,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    maximum_active: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl TestServer {
    /**
     * Start a server whose replies are chosen from each request's method and path.
     */
    async fn start(handler: impl Fn(&str, &str) -> Reply + Send + Sync + 'static) -> Self {
        Self::start_with_tls(handler, None).await
    }

    /**
     * Start a server, optionally speaking TLS with the supplied acceptor.
     */
    async fn start_with_tls(
        handler: impl Fn(&str, &str) -> Reply + Send + Sync + 'static,
        tls: Option<TlsAcceptor>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let handler = Arc::new(handler);
        let task = tokio::spawn({
            let connections = connections.clone();
            let requests = requests.clone();
            let maximum_active = maximum_active.clone();
            async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    connections.fetch_add(1, Ordering::SeqCst);
                    let handler = handler.clone();
                    let requests = requests.clone();
                    let active = active.clone();
                    let maximum_active = maximum_active.clone();
                    let tls = tls.clone();
                    tokio::spawn(async move {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum_active.fetch_max(current, Ordering::SeqCst);
                        match tls {
                            Some(acceptor) => {
                                if let Ok(stream) = acceptor.accept(stream).await {
                                    answer(stream, &*handler, &requests).await;
                                }
                            }
                            None => answer(stream, &*handler, &requests).await,
                        }
                        active.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            }
        });
        Self {
            address,
            connections,
            requests,
            maximum_active,
            task,
        }
    }

    /**
     * Return an absolute URL on this server.
     */
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /**
     * Return the methods and paths received so far.
     */
    fn requests(&self) -> Vec<(String, String)> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    /**
     * Stop accepting connections when the test ends.
     */
    fn drop(&mut self) {
        self.task.abort();
    }
}

/**
 * Read one request head and write the handler's reply.
 */
async fn answer<Stream>(
    mut stream: Stream,
    handler: &(dyn Fn(&str, &str) -> Reply + Send + Sync),
    requests: &Mutex<Vec<(String, String)>>,
) where
    Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut head = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&buffer[..read]),
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut request_line = head.lines().next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    requests
        .lock()
        .unwrap()
        .push((method.clone(), path.clone()));
    let (status, headers) = match handler(&method, &path) {
        Reply::Status(status, headers) => (status, headers),
        Reply::Delayed(delay, status) => {
            tokio::time::sleep(delay).await;
            (status, Vec::new())
        }
        Reply::Hang => {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            return;
        }
    };
    let mut response =
        format!("HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n");
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Resolves configured test host names to fixed addresses; others fail.
struct TestResolver(HashMap<&'static str, Vec<IpAddr>>);

#[async_trait]
impl Resolve for TestResolver {
    /**
     * Answer a configured name on the requested port, or fail like an unknown domain.
     */
    async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        self.0
            .get(host)
            .map(|addresses| {
                addresses
                    .iter()
                    .map(|address| SocketAddr::new(*address, port))
                    .collect()
            })
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "unknown host"))
    }
}

/**
 * Return short limits suitable for loopback tests.
 */
fn test_settings() -> LinkCheckSettings {
    LinkCheckSettings {
        allow_internal_addresses: true,
        connect_timeout: Duration::from_secs(2),
        response_timeout: Duration::from_secs(2),
        link_timeout: Duration::from_secs(5),
        maximum_redirects: 5,
        concurrency_per_check: 3,
        concurrency: 8,
    }
}

/**
 * Build a checker with test settings, test DNS answers and a destination filter.
 */
fn checker(
    settings: LinkCheckSettings,
    names: &[(&'static str, Vec<IpAddr>)],
    filter: AddressFilter,
) -> LinkChecker {
    LinkChecker::with_parts(
        settings,
        Arc::new(TestResolver(names.iter().cloned().collect())),
        filter,
        None,
    )
}

/**
 * Build a checker that may contact any destination.
 */
fn permissive_checker() -> LinkChecker {
    checker(test_settings(), &[], Arc::new(|_| true))
}

/**
 * Return the loopback address used by every test server.
 */
fn loopback() -> IpAddr {
    IpAddr::from([127, 0, 0, 1])
}

/**
 * Status codes map to the verdicts the interface explains.
 */
#[test]
fn status_codes_classify_into_consistent_outcomes() {
    for (code, outcome) in [
        (200, LinkOutcome::Healthy),
        (204, LinkOutcome::Healthy),
        (400, LinkOutcome::Broken),
        (404, LinkOutcome::Broken),
        (410, LinkOutcome::Broken),
        (401, LinkOutcome::Inconclusive),
        (403, LinkOutcome::Inconclusive),
        (407, LinkOutcome::Inconclusive),
        (429, LinkOutcome::Inconclusive),
        (500, LinkOutcome::ServerError),
        (503, LinkOutcome::ServerError),
        (304, LinkOutcome::Inconclusive),
        (999, LinkOutcome::Inconclusive),
    ] {
        assert_eq!(classify_status(code), outcome, "status {code}");
    }
    assert_eq!(status_reason(404), Some("Not Found"));
    assert_eq!(status_reason(503), Some("Service Unavailable"));
    assert_eq!(status_reason(999), None);
}

/**
 * Only absolute HTTP links survive normalization, without their fragments.
 */
#[test]
fn normalization_matches_browser_url_parsing() {
    assert_eq!(
        normalize_link("  HTTPS://Example.TEST  ").as_deref(),
        Some("https://example.test/")
    );
    assert_eq!(
        normalize_link("https://example.test/a b?x=1#section").as_deref(),
        Some("https://example.test/a%20b?x=1")
    );
    assert_eq!(
        normalize_link("http://example.test:80/path#").as_deref(),
        Some("http://example.test/path")
    );
    for rejected in [
        "/relative",
        "mailto:someone@example.test",
        "tel:+15551234",
        "javascript:alert(1)",
        "ftp://example.test/file",
        "https://",
        "",
    ] {
        assert_eq!(normalize_link(rejected), None, "{rejected}");
    }
    assert_eq!(
        normalize_link(&format!("https://example.test/{}", "a".repeat(3000))),
        None
    );
}

/**
 * HTML anchors come first in document order, then text-only links, each URL once.
 */
#[test]
fn extraction_deduplicates_in_first_appearance_order() {
    let html = r#"<p><a href="https://example.test/b#top">B</a>
        <a href="https://example.test/a?x=1&amp;y=2">A</a>
        <a href="https://example.test/b">B again</a>
        <a href="mailto:team@example.test">Mail</a>
        <a href="/relative">Relative</a>
        <a name="anchor">No link</a>
        <map><area href="http://example.test/area"></map>
        <img src="https://example.test/image.png"></p>"#;
    let text = "Visit https://example.test/a?x=1&y=2, or (see https://example.test/text).\n\
        Wrapped: <https://example.test/angle> and https://example.test/wiki/Foo_(bar)!\n\
        Not a link: xhttps://example.test/glued";
    let links = extract_links(html, text);
    assert_eq!(
        links.urls,
        [
            "https://example.test/b",
            "https://example.test/a?x=1&y=2",
            "http://example.test/area",
            "https://example.test/text",
            "https://example.test/angle",
            "https://example.test/wiki/Foo_(bar)",
        ]
    );
    assert_eq!(links.omitted, 0);
}

/**
 * Links beyond the per-message limit are counted rather than verified.
 */
#[test]
fn extraction_counts_links_beyond_the_limit() {
    let html: String = (0..MAXIMUM_LINKS + 7)
        .map(|index| format!(r#"<a href="https://example.test/{index}">x</a>"#))
        .collect();
    let links = extract_links(&html, "");
    assert_eq!(links.urls.len(), MAXIMUM_LINKS);
    assert_eq!(links.omitted, 7);
    assert_eq!(links.urls[0], "https://example.test/0");
}

/**
 * Internal ranges, including translated IPv4 inside IPv6, are never public.
 */
#[test]
fn only_globally_routable_addresses_are_public() {
    for internal in [
        "0.0.0.0",
        "10.1.2.3",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.255",
        "192.0.0.8",
        "192.0.2.10",
        "192.168.1.1",
        "198.18.0.1",
        "198.51.100.7",
        "203.0.113.9",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "64:ff9b::a00:1",
        "64:ff9b:1::1",
        "2002:a00:1::1",
        "2001::1",
        "2001:db8::1",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "ff02::1",
        "100::1",
    ] {
        let address: IpAddr = internal.parse().unwrap();
        assert!(!is_public_address(address), "{internal} must not be public");
    }
    for public in [
        "8.8.8.8",
        "1.1.1.1",
        "172.32.0.1",
        "100.128.0.1",
        "::ffff:8.8.8.8",
        "64:ff9b::808:808",
        "2002:808:808::1",
        "2606:4700:4700::1111",
        "2a00:1450:4001:80b::200e",
    ] {
        let address: IpAddr = public.parse().unwrap();
        assert!(is_public_address(address), "{public} must be public");
    }
}

/**
 * A successful HEAD answer is healthy without downloading anything else.
 */
#[tokio::test]
async fn successful_responses_are_healthy() {
    let server = TestServer::start(|_, _| Reply::Status(200, Vec::new())).await;
    let result = permissive_checker().check_link(&server.url("/ok")).await;
    assert_eq!(result.outcome, LinkOutcome::Healthy);
    assert_eq!(result.status_code, Some(200));
    assert_eq!(result.redirect_count, 0);
    assert_eq!(result.final_url, None);
    assert_eq!(result.error_kind, None);
    assert!(result.checked_at > 0);
    assert_eq!(server.requests(), [("HEAD".into(), "/ok".into())]);
}

/**
 * A server that refuses HEAD is verified again with GET.
 */
#[tokio::test]
async fn refused_head_requests_fall_back_to_get() {
    let server = TestServer::start(|method, _| {
        Reply::Status(if method == "HEAD" { 405 } else { 200 }, Vec::new())
    })
    .await;
    let result = permissive_checker()
        .check_link(&server.url("/page?q=1"))
        .await;
    assert_eq!(result.outcome, LinkOutcome::Healthy);
    assert_eq!(result.status_code, Some(200));
    assert_eq!(
        server.requests(),
        [
            ("HEAD".into(), "/page?q=1".into()),
            ("GET".into(), "/page?q=1".into())
        ]
    );
}

/**
 * Client errors, access refusals and server errors keep their status codes.
 */
#[tokio::test]
async fn error_statuses_are_broken_inconclusive_or_server_errors() {
    let server = TestServer::start(|_, path| match path {
        "/missing" => Reply::Status(404, Vec::new()),
        "/forbidden" => Reply::Status(403, Vec::new()),
        _ => Reply::Status(503, Vec::new()),
    })
    .await;
    let checker = permissive_checker();
    let missing = checker.check_link(&server.url("/missing")).await;
    assert_eq!(
        (missing.outcome, missing.status_code),
        (LinkOutcome::Broken, Some(404))
    );
    let forbidden = checker.check_link(&server.url("/forbidden")).await;
    assert_eq!(
        (forbidden.outcome, forbidden.status_code),
        (LinkOutcome::Inconclusive, Some(403))
    );
    let unavailable = checker.check_link(&server.url("/unavailable")).await;
    assert_eq!(
        (unavailable.outcome, unavailable.status_code),
        (LinkOutcome::ServerError, Some(503))
    );
    assert_eq!(missing.error_kind, None);
}

/**
 * Relative and absolute redirects are followed to the final destination.
 */
#[tokio::test]
async fn redirects_record_the_final_destination() {
    let server = TestServer::start(|_, path| match path {
        "/start" => Reply::Status(301, vec![("Location", "/middle#ignored".into())]),
        "/middle" => Reply::Status(302, vec![("Location", "final?done=1".into())]),
        _ => Reply::Status(200, Vec::new()),
    })
    .await;
    let result = permissive_checker().check_link(&server.url("/start")).await;
    assert_eq!(result.outcome, LinkOutcome::Healthy);
    assert_eq!(result.status_code, Some(200));
    assert_eq!(result.redirect_count, 2);
    assert_eq!(result.final_url, Some(server.url("/final?done=1")));
}

/**
 * Loops, endless chains and unsupported locations fail with distinct reasons.
 */
#[tokio::test]
async fn faulty_redirects_fail() {
    let server = TestServer::start(|_, path| match path {
        "/loop-a" => Reply::Status(302, vec![("Location", "/loop-b".into())]),
        "/loop-b" => Reply::Status(302, vec![("Location", "/loop-a".into())]),
        "/mail" => Reply::Status(302, vec![("Location", "mailto:a@example.test".into())]),
        path => {
            let step: u32 = path.trim_start_matches("/step-").parse().unwrap_or(0);
            Reply::Status(302, vec![("Location", format!("/step-{}", step + 1))])
        }
    })
    .await;
    let checker = permissive_checker();
    let looped = checker.check_link(&server.url("/loop-a")).await;
    assert_eq!(looped.outcome, LinkOutcome::Failed);
    assert_eq!(looped.error_kind, Some(LinkCheckErrorKind::RedirectLoop));
    assert_eq!(looped.status_code, Some(302));
    let endless = checker.check_link(&server.url("/step-0")).await;
    assert_eq!(
        endless.error_kind,
        Some(LinkCheckErrorKind::TooManyRedirects)
    );
    assert_eq!(endless.redirect_count, 5);
    let unsupported = checker.check_link(&server.url("/mail")).await;
    assert_eq!(
        unsupported.error_kind,
        Some(LinkCheckErrorKind::InvalidRedirect)
    );
}

/**
 * A server that never answers fails promptly with a timeout.
 */
#[tokio::test]
async fn unresponsive_servers_time_out() {
    let server = TestServer::start(|_, _| Reply::Hang).await;
    let settings = LinkCheckSettings {
        response_timeout: Duration::from_millis(200),
        ..test_settings()
    };
    let started = std::time::Instant::now();
    let result = checker(settings, &[], Arc::new(|_| true))
        .check_link(&server.url("/slow"))
        .await;
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(result.outcome, LinkOutcome::Failed);
    assert_eq!(result.error_kind, Some(LinkCheckErrorKind::Timeout));
    assert_eq!(result.status_code, None);
}

/**
 * The whole-link limit bounds slow redirect chains too.
 */
#[tokio::test]
async fn the_link_time_limit_bounds_all_requests() {
    let server = TestServer::start(|_, _| Reply::Hang).await;
    let settings = LinkCheckSettings {
        link_timeout: Duration::from_millis(150),
        ..test_settings()
    };
    let result = checker(settings, &[], Arc::new(|_| true))
        .check_link(&server.url("/"))
        .await;
    assert_eq!(result.error_kind, Some(LinkCheckErrorKind::Timeout));
}

/**
 * Unknown names and closed ports fail without an HTTP status.
 */
#[tokio::test]
async fn unreachable_destinations_fail() {
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_address = closed.local_addr().unwrap();
    drop(closed);
    let checker = permissive_checker();
    let unknown = checker.check_link("https://missing.test/").await;
    assert_eq!(unknown.outcome, LinkOutcome::Failed);
    assert_eq!(unknown.error_kind, Some(LinkCheckErrorKind::NameResolution));
    let refused = checker
        .check_link(&format!("http://{closed_address}/"))
        .await;
    assert_eq!(refused.outcome, LinkOutcome::Failed);
    assert_eq!(refused.error_kind, Some(LinkCheckErrorKind::Connection));
    assert!(refused.error_message.is_some());
}

/**
 * The default policy never connects to loopback, by literal or by name.
 */
#[tokio::test]
async fn internal_destinations_are_refused_without_connecting() {
    let server = TestServer::start(|_, _| Reply::Status(200, Vec::new())).await;
    let public_only = checker(
        LinkCheckSettings {
            allow_internal_addresses: false,
            ..test_settings()
        },
        &[("internal.test", vec![loopback()])],
        Arc::new(|address: SocketAddr| is_public_address(address.ip())),
    );
    let literal = public_only.check_link(&server.url("/")).await;
    assert_eq!(literal.outcome, LinkOutcome::Inconclusive);
    assert_eq!(literal.error_kind, Some(LinkCheckErrorKind::BlockedAddress));
    let named = public_only
        .check_link(&format!("http://internal.test:{}/", server.address.port()))
        .await;
    assert_eq!(named.error_kind, Some(LinkCheckErrorKind::BlockedAddress));
    let unspecified = public_only
        .check_link(&format!(
            "http://[::ffff:127.0.0.1]:{}/",
            server.address.port()
        ))
        .await;
    assert_eq!(
        unspecified.error_kind,
        Some(LinkCheckErrorKind::BlockedAddress)
    );
    assert_eq!(server.connections.load(Ordering::SeqCst), 0);
}

/**
 * The production constructor applies the same refusal, unless internal addresses are allowed.
 */
#[tokio::test]
async fn configured_policy_controls_internal_destinations() {
    let server = TestServer::start(|_, _| Reply::Status(204, Vec::new())).await;
    let refused = LinkChecker::new(LinkCheckSettings {
        allow_internal_addresses: false,
        ..test_settings()
    })
    .check_link(&server.url("/"))
    .await;
    assert_eq!(refused.error_kind, Some(LinkCheckErrorKind::BlockedAddress));
    assert_eq!(server.connections.load(Ordering::SeqCst), 0);
    let allowed = LinkChecker::new(test_settings())
        .check_link(&server.url("/"))
        .await;
    assert_eq!(allowed.outcome, LinkOutcome::Healthy);
}

/**
 * A permitted destination cannot redirect the check to a refused one.
 */
#[tokio::test]
async fn redirects_to_internal_destinations_are_refused() {
    let internal = TestServer::start(|_, _| Reply::Status(200, Vec::new())).await;
    let internal_url = internal.url("/secret");
    let public =
        TestServer::start(move |_, _| Reply::Status(302, vec![("Location", internal_url.clone())]))
            .await;
    let permitted = public.address;
    let result = checker(
        test_settings(),
        &[],
        Arc::new(move |address: SocketAddr| address == permitted),
    )
    .check_link(&public.url("/start"))
    .await;
    assert_eq!(result.outcome, LinkOutcome::Inconclusive);
    assert_eq!(result.error_kind, Some(LinkCheckErrorKind::BlockedAddress));
    assert_eq!(result.status_code, Some(302));
    assert_eq!(result.redirect_count, 1);
    assert_eq!(result.final_url, Some(internal.url("/secret")));
    assert_eq!(internal.connections.load(Ordering::SeqCst), 0);
}

/**
 * A name answering with any refused address is refused as a whole.
 */
#[tokio::test]
async fn mixed_dns_answers_are_refused() {
    let server = TestServer::start(|_, _| Reply::Status(200, Vec::new())).await;
    let permitted = server.address.ip();
    let result = checker(
        test_settings(),
        &[("mixed.test", vec![permitted, IpAddr::from([10, 0, 0, 1])])],
        Arc::new(move |address: SocketAddr| address.ip() == permitted),
    )
    .check_link(&format!("http://mixed.test:{}/", server.address.port()))
    .await;
    assert_eq!(result.error_kind, Some(LinkCheckErrorKind::BlockedAddress));
    assert_eq!(server.connections.load(Ordering::SeqCst), 0);
}

/**
 * An untrusted certificate fails the TLS handshake instead of being accepted.
 */
#[tokio::test]
async fn untrusted_certificates_fail() {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let private_key =
        PrivateKeyDer::from_pem_slice(certified.signing_key.serialize_pem().as_bytes()).unwrap();
    let configuration = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certified.cert.der().clone()], private_key)
        .unwrap();
    let server = TestServer::start_with_tls(
        |_, _| Reply::Status(200, Vec::new()),
        Some(TlsAcceptor::from(Arc::new(configuration))),
    )
    .await;
    let result = LinkChecker::with_parts(
        test_settings(),
        Arc::new(TestResolver(HashMap::from([(
            "localhost",
            vec![loopback()],
        )]))),
        Arc::new(|_| true),
        Some(tokio_rustls::rustls::RootCertStore::empty()),
    )
    .check_link(&format!("https://localhost:{}/", server.address.port()))
    .await;
    assert_eq!(result.outcome, LinkOutcome::Failed);
    assert_eq!(result.error_kind, Some(LinkCheckErrorKind::Tls));
    assert!(server.requests().is_empty());
}

/**
 * Concurrent checks keep their input order and respect the per-check bound.
 */
#[tokio::test]
async fn batches_preserve_order_within_the_concurrency_bound() {
    let server = TestServer::start(|_, path| {
        let code = if path.ends_with("broken") { 404 } else { 200 };
        Reply::Delayed(Duration::from_millis(60), code)
    })
    .await;
    let urls: Vec<String> = (0..9)
        .map(|index| {
            server.url(&format!(
                "/{index}-{}",
                if index % 3 == 0 { "broken" } else { "ok" }
            ))
        })
        .collect();
    let results = permissive_checker().check_links(&urls).await;
    assert_eq!(
        results
            .iter()
            .map(|result| result.url.clone())
            .collect::<Vec<_>>(),
        urls
    );
    for (index, result) in results.iter().enumerate() {
        let expected = if index % 3 == 0 {
            LinkOutcome::Broken
        } else {
            LinkOutcome::Healthy
        };
        assert_eq!(result.outcome, expected, "{}", result.url);
    }
    assert!(server.maximum_active.load(Ordering::SeqCst) <= 3);
}
