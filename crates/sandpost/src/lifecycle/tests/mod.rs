//! Runtime startup, listener integration, persistence, and shutdown regressions.

use super::*;
use std::{path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Own isolated runtime data and remove it after all listener tasks have stopped.
struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    /// Remove the isolated SQLite installation when the test finishes.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build isolated configuration with system-allocated listener ports.
fn test_configuration() -> (Configuration, TestDirectory) {
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("sandpost-runtime-{}", uuid::Uuid::new_v4())),
    );
    let configuration = Configuration::from_values(|name| match name {
        "SANDPOST_DATA_DIRECTORY" => Some(directory.0.to_string_lossy().into_owned()),
        "SANDPOST_HTTP_LISTEN" | "SANDPOST_SMTP_LISTEN" | "SANDPOST_IMAP_LISTEN" => {
            Some("127.0.0.1:0".into())
        }
        _ => None,
    })
    .unwrap();
    (configuration, directory)
}

/// Embedding hosts cannot bypass network IMAP encryption checks by editing parsed settings.
#[tokio::test]
async fn modified_configuration_is_validated_before_runtime_start() {
    let (mut configuration, directory) = test_configuration();
    configuration.imap.listen_address = Some("0.0.0.0:0".parse().unwrap());
    match SandpostRuntime::start(configuration).await {
        Ok(running) => {
            running.shutdown().await.unwrap();
            panic!("network IMAP without TLS must require an explicit plaintext opt-in");
        }
        Err(error) => assert!(error.to_string().contains("SANDPOST_IMAP_LISTEN")),
    }
    assert!(
        !directory.0.exists(),
        "invalid settings must fail before opening storage"
    );
}

/// Verify native embedding, real bound sockets, and bounded draining of active SSE.
#[tokio::test]
async fn embedded_runtime_is_ready_and_bounds_shutdown_with_active_events() {
    let (configuration, _directory) = test_configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let secret = running
        .application
        .instance()
        .bootstrap_secret_for_startup()
        .unwrap()
        .unwrap();
    running
        .application
        .instance()
        .setup(sandpost_application::SetupRequest {
            bootstrap_secret: secret,
            name: "Runtime Owner".into(),
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    let login = running
        .application
        .instance()
        .login(sandpost_application::LoginRequest {
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    running.application.health().await.unwrap();
    assert!(
        running
            .application
            .authorized(&login.user)
            .list_messages(None, 50)
            .await
            .unwrap()
            .is_empty()
    );
    let web_address = running.web_listen_address;
    let mail_address = running.mail_listen_address.unwrap();
    let mut stream = tokio::net::TcpStream::connect(web_address).await.unwrap();
    stream
        .write_all(format!("GET /api/v1/events HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n", login.token).as_bytes())
        .await
        .unwrap();
    let mut response = [0; 1024];
    let length = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&response[..length]).contains("200 OK"));
    let mail_stream = tokio::net::TcpStream::connect(mail_address).await.unwrap();
    drop(mail_stream);
    tokio::time::timeout(Duration::from_secs(8), running.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(tokio::net::TcpStream::connect(web_address).await.is_err());
    assert!(tokio::net::TcpStream::connect(mail_address).await.is_err());
    let mut remaining_response = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(1),
        stream.read_to_end(&mut remaining_response),
    )
    .await
    .expect("shutdown must close the accepted SSE connection")
    .unwrap();
}

/// Shutdown releases writer ownership even when the native host retains its application handle.
#[tokio::test]
async fn shutdown_allows_restart_with_a_retained_application_handle() {
    let (configuration, _directory) = test_configuration();
    let running = SandpostRuntime::start(configuration.clone()).await.unwrap();
    let retained = running.application.clone();
    running.shutdown().await.unwrap();
    let restarted = SandpostRuntime::start(configuration).await.unwrap();
    assert!(
        retained
            .search_coordinator()
            .synchronize_batch()
            .await
            .is_err()
    );
    restarted.shutdown().await.unwrap();
}

/// Serve the compiled frontend publicly when present, while unknown API routes remain empty.
#[tokio::test]
async fn runtime_serves_the_compiled_frontend_and_preserves_api_misses() {
    let (configuration, _directory) = test_configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    for path in ["/", "/mail/deep/link", "/api/v1/not-a-route"] {
        let mut stream = tokio::net::TcpStream::connect(running.web_listen_address)
            .await
            .unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        if sandpost_web_api::EMBEDDED_FRONTEND && !path.starts_with("/api/") {
            assert!(
                response.starts_with("HTTP/1.1 200 OK\r\n"),
                "{path}: {response}"
            );
            assert!(response.contains("text/html; charset=utf-8"), "{path}");
        } else {
            assert!(
                response.starts_with("HTTP/1.1 404 Not Found\r\n"),
                "{path}: {response}"
            );
            assert!(response.ends_with("\r\n\r\n"), "{path}: {response}");
        }
    }
    running.shutdown().await.unwrap();
}

mod smtp;
