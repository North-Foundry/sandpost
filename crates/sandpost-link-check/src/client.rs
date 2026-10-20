//! Outbound HTTP verification with redirect, timeout, concurrency and destination controls.
use crate::address::is_public_address;
use crate::classification::classify_status;
use async_trait::async_trait;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::{Method, Request, header};
use hyper_util::rt::TokioIo;
use sandpost_core::{LinkCheckErrorKind, LinkCheckResult, LinkOutcome};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{OnceCell, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, crypto::ring, pki_types::ServerName};
use url::{Host, Position, Url};

/// Identifies Sandpost's checks to destination servers and their logs.
const USER_AGENT: &str = concat!("Sandpost-LinkCheck/", env!("CARGO_PKG_VERSION"));

/// Identifies simulated mail-client visits made on behalf of a Persona.
const VISIT_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; Sandpost-Persona/",
    env!("CARGO_PKG_VERSION"),
    "; +https://github.com/North-Foundry/sandpost)"
);

/// Longest diagnostic stored for a failed check, in bytes.
const MAXIMUM_ERROR_MESSAGE_LENGTH: usize = 240;

/// Limits and destination policy shared by every check of one checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCheckSettings {
    /// Permit loopback, private, link-local and other non-public destinations.
    pub allow_internal_addresses: bool,
    /// Bound on name resolution, each TCP connection attempt and the TLS handshake.
    pub connect_timeout: Duration,
    /// Bound on receiving response headers after the connection is ready.
    pub response_timeout: Duration,
    /// Bound on one link's complete verification, including fallbacks and redirects.
    pub link_timeout: Duration,
    /// Redirects followed before a link fails with too many redirects.
    pub maximum_redirects: u32,
    /// Links of one check verified at the same time.
    pub concurrency_per_check: usize,
    /// Links verified at the same time across all checks of this checker.
    pub concurrency: usize,
}

impl Default for LinkCheckSettings {
    /**
     * Refuse internal destinations and bound each link to about half a minute.
     */
    fn default() -> Self {
        Self {
            allow_internal_addresses: false,
            connect_timeout: Duration::from_secs(5),
            response_timeout: Duration::from_secs(10),
            link_timeout: Duration::from_secs(25),
            maximum_redirects: 10,
            concurrency_per_check: 6,
            concurrency: 16,
        }
    }
}

/// Host name resolution, replaceable so tests can model arbitrary DNS answers.
#[async_trait]
pub(crate) trait Resolve: Send + Sync {
    /**
     * Resolve a host name to every socket address it currently designates.
     */
    async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>>;
}

/// The operating system resolver used outside tests.
struct SystemResolver;

#[async_trait]
impl Resolve for SystemResolver {
    /**
     * Resolve through the operating system, as browsers on the server would.
     */
    async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host((host, port)).await?.collect())
    }
}

/// Decides whether a resolved destination may be contacted.
pub(crate) type AddressFilter = Arc<dyn Fn(SocketAddr) -> bool + Send + Sync>;

/// Verifies links from the server; cheap to clone and safe to share between tasks.
#[derive(Clone)]
pub struct LinkChecker {
    inner: Arc<CheckerState>,
}

/// Shared configuration, resolver, lazily loaded TLS trust and global concurrency.
struct CheckerState {
    settings: LinkCheckSettings,
    resolver: Arc<dyn Resolve>,
    address_allowed: AddressFilter,
    /// Trust anchors supplied by tests; the system trust store is loaded otherwise.
    trust_anchors: Option<RootCertStore>,
    tls: OnceCell<TlsConnector>,
    links: Semaphore,
}

/// A verification step that produced no usable response.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    kind: LinkCheckErrorKind,
    message: String,
}

impl Failure {
    /**
     * Build a failure with a bounded diagnostic message.
     */
    fn new(kind: LinkCheckErrorKind, message: impl std::fmt::Display) -> Self {
        let mut message = message.to_string();
        if message.len() > MAXIMUM_ERROR_MESSAGE_LENGTH {
            let mut end = MAXIMUM_ERROR_MESSAGE_LENGTH;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        Self { kind, message }
    }
}

/// The parts of a response that verification and visits need.
struct ResponseHead {
    status: u16,
    location: Option<String>,
    content_type: Option<String>,
    /// Body bytes read; zero when the body was not requested.
    received_bytes: u64,
    /// Reading stopped at the body limit before the response ended.
    truncated: bool,
}

/// What a visit requests, which decides the `Accept` header a mail client would send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitPurpose {
    /// A remote image displayed while a message is open, such as a tracking pixel.
    Image,
    /// A page opened by following a link.
    Document,
}

/// The outcome of one visit: the final response after redirects, or why none was obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisitResult {
    /// The URL that was requested first.
    pub url: String,
    /// The last URL requested after following redirects, when any redirect was followed.
    pub final_url: Option<String>,
    /// The status code of the final response, or of the last response before a failure.
    pub status_code: Option<u16>,
    pub redirect_count: u32,
    /// The final response's `Content-Type`, when present.
    pub content_type: Option<String>,
    /// Body bytes received from the final response, at most the requested limit.
    pub received_bytes: u64,
    /// The final response's body was longer than the limit; reading stopped there.
    pub truncated: bool,
    /// Why no final response was obtained; absent on success.
    pub error_kind: Option<LinkCheckErrorKind>,
    pub error_message: Option<String>,
    /// Wall-clock time spent, including redirects and body reading.
    pub elapsed_milliseconds: u64,
}

impl VisitResult {
    /**
     * Report whether a final response was received, whatever its status code.
     */
    pub fn responded(&self) -> bool {
        self.error_kind.is_none() && self.status_code.is_some()
    }
}

/// How a request is sent and how much of its response body is read.
#[derive(Clone, Copy)]
struct RequestProfile {
    user_agent: &'static str,
    accept: &'static str,
    /// Body bytes to read after the head; `None` leaves the body unread.
    body_limit: Option<usize>,
}

/// Link checks read only response heads.
const CHECK_PROFILE: RequestProfile = RequestProfile {
    user_agent: USER_AGENT,
    accept: "*/*",
    body_limit: None,
};

/// What has been observed so far, retained when the link's time limit interrupts it.
#[derive(Default)]
struct Progress {
    status: Option<u16>,
    final_url: Option<String>,
    redirects: u32,
}

/// The final response of a visit after redirects.
struct VisitResponse {
    status: u16,
    content_type: Option<String>,
    received_bytes: u64,
    truncated: bool,
}

/// Aborts a spawned connection driver when the request that owns it ends.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    /**
     * Stop driving the connection so no response body is read after the head.
     */
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl LinkChecker {
    /**
     * Create a checker that resolves names with the operating system and trusts the system's
     * certificate store, which is loaded on the first HTTPS request.
     */
    pub fn new(settings: LinkCheckSettings) -> Self {
        let allow_internal = settings.allow_internal_addresses;
        Self::with_parts(
            settings,
            Arc::new(SystemResolver),
            Arc::new(move |address: SocketAddr| allow_internal || is_public_address(address.ip())),
            None,
        )
    }

    /**
     * Create a checker from replaceable resolution, destination policy and trust anchors.
     */
    pub(crate) fn with_parts(
        settings: LinkCheckSettings,
        resolver: Arc<dyn Resolve>,
        address_allowed: AddressFilter,
        trust_anchors: Option<RootCertStore>,
    ) -> Self {
        let links = Semaphore::new(settings.concurrency.max(1));
        Self {
            inner: Arc::new(CheckerState {
                settings,
                resolver,
                address_allowed,
                trust_anchors,
                tls: OnceCell::new(),
                links,
            }),
        }
    }

    /**
     * Return the limits and destination policy of this checker.
     */
    pub fn settings(&self) -> &LinkCheckSettings {
        &self.inner.settings
    }

    /**
     * Verify several links concurrently, returning results in the order of `urls`.
     *
     * At most `concurrency_per_check` of these links run at once, and every checker-wide
     * check shares the `concurrency` bound.
     */
    pub async fn check_links(&self, urls: &[String]) -> Vec<LinkCheckResult> {
        let per_check = Arc::new(Semaphore::new(
            self.inner.settings.concurrency_per_check.max(1),
        ));
        let mut tasks = JoinSet::new();
        for (index, url) in urls.iter().cloned().enumerate() {
            let checker = self.clone();
            let per_check = per_check.clone();
            tasks.spawn(async move {
                let _permit = per_check.acquire_owned().await;
                (index, checker.check_link(&url).await)
            });
        }
        let mut results: Vec<Option<LinkCheckResult>> = vec![None; urls.len()];
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((index, result)) => results[index] = Some(result),
                Err(error) => tracing::warn!(%error, "link check task failed"),
            }
        }
        results
            .into_iter()
            .zip(urls)
            .map(|(result, url)| {
                result.unwrap_or_else(|| {
                    failed_result(
                        url,
                        Progress::default(),
                        Failure::new(LinkCheckErrorKind::Protocol, "the check was interrupted"),
                    )
                })
            })
            .collect()
    }

    /**
     * Verify one absolute HTTP or HTTPS link, following redirects within the time limit.
     *
     * The verdict comes from the final response; a refused destination is inconclusive and any
     * other failure to obtain a response is failed. The last status seen before a failure, such
     * as the redirect that pointed to a private address, is kept with the result.
     */
    pub async fn check_link(&self, url: &str) -> LinkCheckResult {
        let _permit = self.inner.links.acquire().await;
        let mut progress = Progress::default();
        let limit = self.inner.settings.link_timeout;
        let outcome = tokio::time::timeout(limit, self.follow(url, &mut progress)).await;
        let result = match outcome {
            Ok(Ok(status)) => LinkCheckResult {
                url: url.to_owned(),
                outcome: classify_status(status),
                status_code: Some(status),
                final_url: progress.final_url,
                redirect_count: progress.redirects,
                error_kind: None,
                error_message: None,
                checked_at: now(),
            },
            Ok(Err(failure)) => failed_result(url, progress, failure),
            Err(_) => failed_result(
                url,
                progress,
                Failure::new(
                    LinkCheckErrorKind::Timeout,
                    format!("no final response within {} seconds", limit.as_secs()),
                ),
            ),
        };
        if let Some(kind) = result.error_kind {
            tracing::debug!(url, kind = kind.as_str(), "link check failed");
        }
        result
    }

    /**
     * Visit a URL as a mail client would: `GET` it, follow redirects, and read the final
     * response body up to `maximum_body_bytes`.
     *
     * Every hop is resolved once and refused when any resolved address is not allowed, exactly
     * like link checks, so redirects and DNS answers cannot reach internal destinations unless
     * the checker permits them. Requests carry no cookies, credentials, or referrer. The visit
     * shares this checker's concurrency bound and per-link time limit, which also bounds body
     * reading. A visit is never retried: a request that may have reached the server is reported
     * once, because repeating it could repeat its effect.
     */
    pub async fn visit(
        &self,
        url: &str,
        purpose: VisitPurpose,
        maximum_body_bytes: usize,
    ) -> VisitResult {
        let _permit = self.inner.links.acquire().await;
        let started = Instant::now();
        let profile = RequestProfile {
            user_agent: VISIT_USER_AGENT,
            accept: match purpose {
                VisitPurpose::Image => {
                    "image/avif,image/webp,image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5"
                }
                VisitPurpose::Document => {
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                }
            },
            body_limit: Some(maximum_body_bytes),
        };
        let mut progress = Progress::default();
        let limit = self.inner.settings.link_timeout;
        let outcome =
            tokio::time::timeout(limit, self.follow_visit(url, profile, &mut progress)).await;
        let failure = match outcome {
            Ok(Ok(response)) => {
                return VisitResult {
                    url: url.to_owned(),
                    final_url: progress.final_url,
                    status_code: Some(response.status),
                    redirect_count: progress.redirects,
                    content_type: response.content_type,
                    received_bytes: response.received_bytes,
                    truncated: response.truncated,
                    error_kind: None,
                    error_message: None,
                    elapsed_milliseconds: elapsed_milliseconds(started),
                };
            }
            Ok(Err(failure)) => failure,
            Err(_) => Failure::new(
                LinkCheckErrorKind::Timeout,
                format!("no complete response within {} seconds", limit.as_secs()),
            ),
        };
        tracing::debug!(url, kind = failure.kind.as_str(), "visit failed");
        VisitResult {
            url: url.to_owned(),
            final_url: progress.final_url,
            status_code: progress.status,
            redirect_count: progress.redirects,
            content_type: None,
            received_bytes: 0,
            truncated: false,
            error_kind: Some(failure.kind),
            error_message: Some(failure.message),
            elapsed_milliseconds: elapsed_milliseconds(started),
        }
    }

    /**
     * `GET` a URL and follow its redirects, reading only the final response body.
     */
    async fn follow_visit(
        &self,
        url: &str,
        profile: RequestProfile,
        progress: &mut Progress,
    ) -> Result<VisitResponse, Failure> {
        let mut current = Url::parse(url)
            .map_err(|error| Failure::new(LinkCheckErrorKind::InvalidRedirect, error))?;
        if !matches!(current.scheme(), "http" | "https") || current.host_str().is_none() {
            return Err(Failure::new(
                LinkCheckErrorKind::InvalidRedirect,
                format!("unsupported {}: URL", current.scheme()),
            ));
        }
        let mut visited = HashSet::from([current.to_string()]);
        loop {
            // One GET per hop: the body is read only when the response is final.
            let response = self.request(Method::GET, &current, profile).await?;
            progress.status = Some(response.status);
            let Some(location) = redirect_location(&response) else {
                return Ok(VisitResponse {
                    status: response.status,
                    content_type: response.content_type,
                    received_bytes: response.received_bytes,
                    truncated: response.truncated,
                });
            };
            current = self.next_location(&current, &location, progress, &mut visited)?;
        }
    }

    /**
     * Request a link and follow its redirects, returning the final status code.
     */
    async fn follow(&self, url: &str, progress: &mut Progress) -> Result<u16, Failure> {
        let mut current = Url::parse(url)
            .map_err(|error| Failure::new(LinkCheckErrorKind::InvalidRedirect, error))?;
        let mut visited = HashSet::from([current.to_string()]);
        loop {
            let response = self.request_with_fallback(&current).await?;
            progress.status = Some(response.status);
            let Some(location) = redirect_location(&response) else {
                return Ok(response.status);
            };
            current = self.next_location(&current, &location, progress, &mut visited)?;
        }
    }

    /**
     * Resolve and validate a redirect target, recording it as the latest destination.
     *
     * Only `http` and `https` targets with a host are followed; the redirect limit and loops
     * fail the request.
     */
    fn next_location(
        &self,
        current: &Url,
        location: &str,
        progress: &mut Progress,
        visited: &mut HashSet<String>,
    ) -> Result<Url, Failure> {
        let mut next = current.join(location).map_err(|_| {
            Failure::new(
                LinkCheckErrorKind::InvalidRedirect,
                "the redirect location is not a valid URL",
            )
        })?;
        if !matches!(next.scheme(), "http" | "https") || next.host_str().is_none() {
            return Err(Failure::new(
                LinkCheckErrorKind::InvalidRedirect,
                format!("redirects to an unsupported {}: URL", next.scheme()),
            ));
        }
        next.set_fragment(None);
        if progress.redirects >= self.inner.settings.maximum_redirects {
            return Err(Failure::new(
                LinkCheckErrorKind::TooManyRedirects,
                format!(
                    "more than {} redirects",
                    self.inner.settings.maximum_redirects
                ),
            ));
        }
        progress.redirects += 1;
        progress.final_url = Some(next.to_string());
        if !visited.insert(next.to_string()) {
            return Err(Failure::new(
                LinkCheckErrorKind::RedirectLoop,
                "redirects back to a URL already visited",
            ));
        }
        Ok(next)
    }

    /**
     * Send `HEAD`, retrying with `GET` when it answers with an error status.
     *
     * Some servers refuse or mishandle `HEAD`; the `GET` response head decides instead, and its
     * body is never read. If the retry cannot obtain a response, the `HEAD` answer stands.
     */
    async fn request_with_fallback(&self, url: &Url) -> Result<ResponseHead, Failure> {
        let head = self.request(Method::HEAD, url, CHECK_PROFILE).await?;
        if head.status < 400 {
            return Ok(head);
        }
        Ok(self
            .request(Method::GET, url, CHECK_PROFILE)
            .await
            .unwrap_or(head))
    }

    /**
     * Send one request to validated addresses and read its response head, plus the body up to
     * the profile's limit when the response is not a redirect.
     */
    async fn request(
        &self,
        method: Method,
        url: &Url,
        profile: RequestProfile,
    ) -> Result<ResponseHead, Failure> {
        let settings = &self.inner.settings;
        let host = url.host().ok_or_else(|| {
            Failure::new(LinkCheckErrorKind::InvalidRedirect, "the URL has no host")
        })?;
        let port = url.port_or_known_default().ok_or_else(|| {
            Failure::new(LinkCheckErrorKind::InvalidRedirect, "the URL has no port")
        })?;
        let addresses = match &host {
            Host::Ipv4(address) => vec![SocketAddr::new(IpAddr::V4(*address), port)],
            Host::Ipv6(address) => vec![SocketAddr::new(IpAddr::V6(*address), port)],
            Host::Domain(name) => {
                match tokio::time::timeout(
                    settings.connect_timeout,
                    self.inner.resolver.resolve(name, port),
                )
                .await
                {
                    Err(_) => {
                        return Err(Failure::new(
                            LinkCheckErrorKind::Timeout,
                            format!("resolving {name} timed out"),
                        ));
                    }
                    Ok(Err(error)) => {
                        return Err(Failure::new(
                            LinkCheckErrorKind::NameResolution,
                            format!("{name} could not be resolved: {error}"),
                        ));
                    }
                    Ok(Ok(addresses)) => addresses,
                }
            }
        };
        if addresses.is_empty() {
            return Err(Failure::new(
                LinkCheckErrorKind::NameResolution,
                "the host has no addresses",
            ));
        }
        // Refuse the whole host if any answer is internal, so a mixed or rebinding DNS answer
        // cannot direct a later attempt to a private address.
        if let Some(blocked) = addresses
            .iter()
            .find(|address| !(self.inner.address_allowed)(**address))
        {
            return Err(Failure::new(
                LinkCheckErrorKind::BlockedAddress,
                format!("{} is not a public address", blocked.ip()),
            ));
        }
        let stream = self.connect(&addresses).await?;
        let request = Request::builder()
            .method(method)
            .uri(match &url[Position::BeforePath..Position::AfterQuery] {
                "" => "/",
                target => target,
            })
            .header(
                header::HOST,
                &url[Position::BeforeHost..Position::AfterPort],
            )
            .header(header::USER_AGENT, profile.user_agent)
            .header(header::ACCEPT, profile.accept)
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::CONNECTION, "close")
            .body(Empty::<Bytes>::new())
            .map_err(|error| Failure::new(LinkCheckErrorKind::Protocol, error))?;
        if url.scheme() == "https" {
            let server_name = match host {
                Host::Domain(name) => ServerName::try_from(name.to_owned())
                    .map_err(|error| Failure::new(LinkCheckErrorKind::Tls, error))?,
                Host::Ipv4(address) => ServerName::IpAddress(IpAddr::V4(address).into()),
                Host::Ipv6(address) => ServerName::IpAddress(IpAddr::V6(address).into()),
            };
            let connector = self.tls_connector().await?;
            let stream = tokio::time::timeout(
                settings.connect_timeout,
                connector.connect(server_name, stream),
            )
            .await
            .map_err(|_| Failure::new(LinkCheckErrorKind::Timeout, "the TLS handshake timed out"))?
            .map_err(|error| Failure::new(LinkCheckErrorKind::Tls, error))?;
            self.exchange(stream, request, profile.body_limit).await
        } else {
            self.exchange(stream, request, profile.body_limit).await
        }
    }

    /**
     * Connect to the first reachable validated address, in resolver order.
     */
    async fn connect(&self, addresses: &[SocketAddr]) -> Result<TcpStream, Failure> {
        let mut failure = None;
        for address in addresses {
            match tokio::time::timeout(
                self.inner.settings.connect_timeout,
                TcpStream::connect(address),
            )
            .await
            {
                Ok(Ok(stream)) => return Ok(stream),
                Ok(Err(error)) => {
                    failure = Some(Failure::new(
                        LinkCheckErrorKind::Connection,
                        format!("{address}: {error}"),
                    ));
                }
                Err(_) => {
                    failure = Some(Failure::new(
                        LinkCheckErrorKind::Timeout,
                        format!("connecting to {address} timed out"),
                    ));
                }
            }
        }
        Err(failure.unwrap_or_else(|| {
            Failure::new(
                LinkCheckErrorKind::Connection,
                "no address could be reached",
            )
        }))
    }

    /**
     * Send the request over HTTP/1.1 and return its response head within the time limit.
     *
     * With a body limit, a final (non-redirect) response's body is then read until it ends or
     * the limit is reached; the connection closes as soon as reading stops, so a larger body is
     * never transferred further. Body reading is bounded by the caller's overall time limit.
     */
    async fn exchange<Stream>(
        &self,
        stream: Stream,
        request: Request<Empty<Bytes>>,
        body_limit: Option<usize>,
    ) -> Result<ResponseHead, Failure>
    where
        Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let timeout = self.inner.settings.response_timeout;
        let exchange = async {
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
            let driver = AbortOnDrop(tokio::spawn(async move {
                let _ = connection.await;
            }));
            sender
                .send_request(request)
                .await
                .map(|response| (response, driver))
        };
        let (response, _driver) = tokio::time::timeout(timeout, exchange)
            .await
            .map_err(|_| {
                Failure::new(
                    LinkCheckErrorKind::Timeout,
                    format!("no response within {} seconds", timeout.as_secs()),
                )
            })?
            .map_err(|error| {
                let kind = if error.is_parse() || error.is_parse_status() {
                    LinkCheckErrorKind::Protocol
                } else {
                    LinkCheckErrorKind::Connection
                };
                Failure::new(kind, error)
            })?;
        let mut head = ResponseHead {
            status: response.status().as_u16(),
            location: response
                .headers()
                .get(header::LOCATION)
                .map(|location| {
                    String::from_utf8_lossy(location.as_bytes())
                        .trim()
                        .to_owned()
                })
                .filter(|location| !location.is_empty()),
            content_type: response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(|value| bounded_text(&String::from_utf8_lossy(value.as_bytes()), 120)),
            received_bytes: 0,
            truncated: false,
        };
        let Some(limit) = body_limit.filter(|_| redirect_location(&head).is_none()) else {
            return Ok(head);
        };
        let mut body = response.into_body();
        while let Some(frame) = body.frame().await {
            let frame =
                frame.map_err(|error| Failure::new(LinkCheckErrorKind::Connection, error))?;
            let Some(data) = frame.data_ref() else {
                continue;
            };
            let remaining = limit.saturating_sub(head.received_bytes as usize);
            if data.len() > remaining {
                head.received_bytes += remaining as u64;
                head.truncated = true;
                break;
            }
            head.received_bytes += data.len() as u64;
        }
        Ok(head)
    }

    /**
     * Return the shared TLS connector, loading trust anchors on first use.
     */
    async fn tls_connector(&self) -> Result<TlsConnector, Failure> {
        self.inner
            .tls
            .get_or_try_init(|| async {
                let supplied = self.inner.trust_anchors.clone();
                let roots = match supplied {
                    Some(roots) => roots,
                    None => tokio::task::spawn_blocking(|| {
                        let mut roots = RootCertStore::empty();
                        roots.add_parsable_certificates(
                            rustls_native_certs::load_native_certs().certs,
                        );
                        roots
                    })
                    .await
                    .map_err(|error| Failure::new(LinkCheckErrorKind::Tls, error))?,
                };
                let mut configuration =
                    ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
                        .with_safe_default_protocol_versions()
                        .map_err(|error| Failure::new(LinkCheckErrorKind::Tls, error))?
                        .with_root_certificates(roots)
                        .with_no_client_auth();
                configuration.alpn_protocols = vec![b"http/1.1".to_vec()];
                Ok(TlsConnector::from(Arc::new(configuration)))
            })
            .await
            .cloned()
    }
}

/**
 * Return the location of a redirect response that should be followed.
 */
fn redirect_location(response: &ResponseHead) -> Option<String> {
    matches!(response.status, 301 | 302 | 303 | 307 | 308)
        .then(|| response.location.clone())
        .flatten()
}

/**
 * Shorten text to at most `maximum` bytes on a character boundary.
 */
fn bounded_text(value: &str, maximum: usize) -> String {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim().to_owned()
}

/**
 * Return the milliseconds elapsed since `started`.
 */
fn elapsed_milliseconds(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/**
 * Build the stored result of a link whose verification produced no final response.
 *
 * A refused destination is inconclusive because the link may work for its readers; every other
 * failure is failed.
 */
fn failed_result(url: &str, progress: Progress, failure: Failure) -> LinkCheckResult {
    LinkCheckResult {
        url: url.to_owned(),
        outcome: if failure.kind == LinkCheckErrorKind::BlockedAddress {
            LinkOutcome::Inconclusive
        } else {
            LinkOutcome::Failed
        },
        status_code: progress.status,
        final_url: progress.final_url,
        redirect_count: progress.redirects,
        error_kind: Some(failure.kind),
        error_message: Some(failure.message),
        checked_at: now(),
    }
}

/**
 * Return the current Unix time in whole seconds.
 */
pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
