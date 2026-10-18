//! Exercise frontend proxy configuration through the real Sandpost executable.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const PROCESS_WAIT: Duration = Duration::from_secs(15);
const NETWORK_WAIT: Duration = Duration::from_secs(5);

/// Own isolated configuration and storage paths for one real-process scenario.
struct TestInstallation {
    directory: PathBuf,
}

impl TestInstallation {
    /// Create a fresh configuration, optionally setting a TOML frontend origin.
    fn new(frontend_setting: Option<&str>) -> Self {
        let directory =
            std::env::temp_dir().join(format!("sandpost-frontend-proxy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let frontend = frontend_setting
            .map(|url| format!("[frontend]\nproxy_url = {url:?}\n"))
            .unwrap_or_default();
        std::fs::write(directory.join("config.toml"), frontend).unwrap();
        Self { directory }
    }

    /// Start the isolated runtime with only the requested command-line arguments.
    fn start_server(&self, arguments: &[&str], expected_origin: &str) -> RunningServer {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sandpost"))
            .env_clear()
            .env("SANDPOST_CONFIG", self.directory.join("config.toml"))
            .env("SANDPOST_DATA_DIRECTORY", &self.directory)
            .env("SANDPOST_HTTP_BIND_ADDRESS", "127.0.0.1")
            .env("SANDPOST_HTTP_PORT", "0")
            .env("SANDPOST_SMTP_BIND_ADDRESS", "127.0.0.1")
            .env("SANDPOST_SMTP_PORT", "0")
            .env("SANDPOST_IMAP_ENABLED", "false")
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if sender.send(line.unwrap_or_default()).is_err() {
                    break;
                }
            }
        });
        let mut server = RunningServer {
            child,
            lines,
            address: None,
        };
        server.address = Some(server.wait_until_ready(expected_origin));
        server
    }
}

impl Drop for TestInstallation {
    /// Remove all data created by this isolated process scenario.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Kill and reap a real CLI process even when an assertion fails.
struct RunningServer {
    child: Child,
    lines: mpsc::Receiver<String>,
    address: Option<SocketAddr>,
}

impl RunningServer {
    /// Read startup output with a deadline and return the bound local HTTP address.
    fn wait_until_ready(&mut self, expected_origin: &str) -> SocketAddr {
        let deadline = Instant::now() + PROCESS_WAIT;
        let mut startup = String::new();
        let mut web_address = None;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(remaining)
                .unwrap_or_else(|error| panic!("Sandpost did not start: {error}; {startup}"));
            if line.contains("Web UI") {
                web_address = line
                    .split("http://")
                    .nth(1)
                    .and_then(|address| address.parse().ok());
            }
            startup.push_str(&line);
            startup.push('\n');
            if line.contains("Frontend proxy") {
                assert!(
                    line.contains(expected_origin),
                    "unexpected proxy target: {line}"
                );
                return web_address.unwrap_or_else(|| panic!("missing Web UI address: {startup}"));
            }
        }
    }

    /// Return the HTTP listener address reported during successful startup.
    fn address(&self) -> SocketAddr {
        self.address
            .expect("server startup address was not recorded")
    }
}

impl Drop for RunningServer {
    /// Stop the child and wait for it to release its listeners and storage handles.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Serve one deterministic frontend response through a localhost TCP listener.
fn serve_frontend(listener: TcpListener, body: &'static str) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + NETWORK_WAIT;
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "proxy did not contact frontend");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("frontend listener failed: {error}"),
            }
        };
        stream.set_read_timeout(Some(NETWORK_WAIT)).unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    })
}

/// Send a bounded HTTP/1.1 request and return its status and body.
fn request(address: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect_timeout(&address, NETWORK_WAIT).unwrap();
    stream.set_read_timeout(Some(NETWORK_WAIT)).unwrap();
    stream.set_write_timeout(Some(NETWORK_WAIT)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let status = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, body.to_owned())
}

/// Exercise a TOML-only origin while keeping API and unknown API routes on Rust.
#[test]
fn toml_frontend_proxy_serves_ui_and_keeps_api_local() {
    let installation = TestInstallation::new(None);
    let frontend = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://localhost:{}", frontend.local_addr().unwrap().port());
    std::fs::write(
        installation.directory.join("config.toml"),
        format!("[frontend]\nproxy_url = {origin:?}\n"),
    )
    .unwrap();
    let frontend_response = serve_frontend(frontend, "toml frontend body");
    let server = installation.start_server(&[], &origin);
    let address = server.address();

    let (status, body) = request(address, "/");
    assert_eq!(status, 200);
    assert_eq!(body, "toml frontend body");
    frontend_response.join().unwrap();
    let (status, body) = request(address, "/api/v1/instance");
    assert_eq!(status, 200);
    assert!(body.contains("setupKeyRequired"), "{body}");
    assert_eq!(request(address, "/api/not-a-route").0, 404);
}

/// Let a valid CLI origin replace an invalid URL in the otherwise valid TOML file.
#[test]
fn command_line_frontend_proxy_overrides_invalid_toml_url() {
    let installation = TestInstallation::new(Some("ftp://invalid.example"));
    let frontend = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://localhost:{}", frontend.local_addr().unwrap().port());
    let frontend_response = serve_frontend(frontend, "command line frontend body");
    let server = installation.start_server(&["--frontend-proxy-url", &origin], &origin);
    let address = server.address();

    let (status, body) = request(address, "/");
    assert_eq!(status, 200);
    assert_eq!(body, "command line frontend body");
    frontend_response.join().unwrap();
}

/// Wait for the frontend's delayed hot-file publication and validate its trimmed origin.
#[cfg(debug_assertions)]
#[test]
fn command_line_frontend_proxy_file_waits_for_publication() {
    let installation = TestInstallation::new(Some("ftp://invalid.example"));
    let frontend = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://localhost:{}", frontend.local_addr().unwrap().port());
    let frontend_response = serve_frontend(frontend, "file frontend body");
    let path = installation.directory.join("vite-origin");
    let published_path = path.clone();
    let published_origin = origin.clone();
    let publisher = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        std::fs::write(published_path, format!("  {published_origin}\n")).unwrap();
    });
    let path = path.to_string_lossy().into_owned();
    let server = installation.start_server(&["--frontend-proxy-file", &path], &origin);
    publisher.join().unwrap();
    let (status, body) = request(server.address(), "/");
    assert_eq!(status, 200);
    assert_eq!(body, "file frontend body");
    frontend_response.join().unwrap();
}

/// The explicit URL wins without opening, waiting for, or validating the file option.
#[cfg(debug_assertions)]
#[test]
fn command_line_frontend_proxy_url_ignores_file_in_either_order() {
    let installation = TestInstallation::new(None);
    let missing_file = installation.directory.join("missing-vite-origin");
    let invalid_file = installation.directory.join("invalid-vite-origin");
    let unreadable_file = installation.directory.join("directory-is-not-a-file");
    std::fs::write(&invalid_file, "ftp://invalid.example\n").unwrap();
    std::fs::create_dir(&unreadable_file).unwrap();
    for file in [&missing_file, &invalid_file, &unreadable_file] {
        let file_path = file.to_string_lossy().into_owned();
        for file_first in [false, true] {
            let frontend = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://localhost:{}", frontend.local_addr().unwrap().port());
            let frontend_response = serve_frontend(frontend, "explicit URL body");
            let arguments = if file_first {
                vec![
                    "--frontend-proxy-file",
                    file_path.as_str(),
                    "--frontend-proxy-url",
                    origin.as_str(),
                ]
            } else {
                vec![
                    "--frontend-proxy-url",
                    origin.as_str(),
                    "--frontend-proxy-file",
                    file_path.as_str(),
                ]
            };
            let server = installation.start_server(&arguments, &origin);
            assert_eq!(
                request(server.address(), "/"),
                (200, "explicit URL body".into())
            );
            frontend_response.join().unwrap();
        }
    }
}

/// Return a failing command's output without creating runtime storage.
fn run_rejected_command(
    installation: &TestInstallation,
    arguments: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sandpost"))
        .env_clear()
        .env(
            "SANDPOST_CONFIG",
            installation.directory.join("config.toml"),
        )
        .env("SANDPOST_DATA_DIRECTORY", &installation.directory)
        .args(arguments)
        .output()
        .unwrap()
}

/// Fail with 502 for an unavailable upstream and reject invalid or removed CLI options early.
#[test]
fn frontend_proxy_failure_and_rejected_options_are_bounded() {
    let unavailable_installation = TestInstallation::new(None);
    let unavailable_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let unavailable_port = unavailable_listener.local_addr().unwrap().port();
    drop(unavailable_listener);
    let unavailable_origin = format!("http://localhost:{unavailable_port}");
    let server = unavailable_installation.start_server(
        &["--frontend-proxy-url", &unavailable_origin],
        &unavailable_origin,
    );
    let address = server.address();
    assert_eq!(request(address, "/").0, 502);
    drop(server);

    let rejected_installation = TestInstallation::new(None);
    let invalid = run_rejected_command(
        &rejected_installation,
        &["--frontend-proxy-url", "ftp://invalid.example"],
    );
    assert!(!invalid.status.success());
    assert!(
        !rejected_installation
            .directory
            .join("sandpost.sqlite3")
            .exists()
    );

    let legacy = run_rejected_command(
        &rejected_installation,
        &["--frontend-dev-url", "http://localhost:5173"],
    );
    assert!(!legacy.status.success());
    assert!(
        !rejected_installation
            .directory
            .join("sandpost.sqlite3")
            .exists()
    );
}

/// Send file contents through the existing frontend-origin configuration validator.
#[cfg(debug_assertions)]
#[test]
fn frontend_proxy_file_contents_are_validated() {
    let installation = TestInstallation::new(None);
    let invalid_file = installation.directory.join("vite-origin");
    std::fs::write(&invalid_file, "ftp://invalid.example\n").unwrap();
    let invalid_file_path = invalid_file.to_string_lossy().into_owned();
    let output = run_rejected_command(
        &installation,
        &["--frontend-proxy-file", &invalid_file_path],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("proxy"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An invalid explicit URL fails even when a valid file origin is also supplied.
#[cfg(debug_assertions)]
#[test]
fn invalid_command_line_url_does_not_fall_back_to_file() {
    let installation = TestInstallation::new(None);
    let valid_file = installation.directory.join("vite-origin");
    std::fs::write(&valid_file, "http://localhost:5173\n").unwrap();
    let valid_file_path = valid_file.to_string_lossy().into_owned();
    let output = run_rejected_command(
        &installation,
        &[
            "--frontend-proxy-file",
            &valid_file_path,
            "--frontend-proxy-url",
            "ftp://invalid.example",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("proxy"));
}

/// Release executables reject the debug-only file option before runtime startup.
#[cfg(not(debug_assertions))]
#[test]
fn release_rejects_frontend_proxy_file_option() {
    let installation = TestInstallation::new(None);
    let output = run_rejected_command(
        &installation,
        &["--frontend-proxy-file", "/tmp/unused-vite-origin"],
    );
    assert!(!output.status.success());
    assert!(
        !installation.directory.join("sandpost.sqlite3").exists(),
        "rejected CLI options must not start runtime storage"
    );
}
